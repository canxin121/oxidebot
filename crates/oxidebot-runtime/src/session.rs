use crate::{budget::HierarchicalLease, MetricsHandle, SessionError};
use futures_util::StreamExt;
use oxidebot_core::event::kernel::{DispatchEnvelope, DispatchIndex, DispatchKind};
use oxidebot_core::{ConversationKey, Event, SessionNamespace, UserKey};
use std::{
    collections::{hash_map::RandomState, HashMap, HashSet},
    fmt,
    hash::{BuildHasher, Hash},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc, RwLock,
    },
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot, OwnedSemaphorePermit, Semaphore, TryAcquireError};
use tokio_util::{
    sync::CancellationToken,
    time::{delay_queue::Key as DelayKey, DelayQueue},
};

/// Exact conversation and actor scope used for pre-decode session interest.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ScopeKey {
    conversation: ConversationKey,
    actor: UserKey,
}

impl ScopeKey {
    fn from_index(index: &DispatchIndex) -> Option<Self> {
        Some(Self {
            conversation: index.conversation.clone()?,
            actor: index.actor.clone()?,
        })
    }
}

/// Identity for one exclusive dialogue/wait registration. A scope may have only
/// one active exclusive waiter; namespace distinguishes ownership and safe
/// cancellation rather than multiplexing multiple next-message consumers.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SessionKey {
    pub conversation: ConversationKey,
    pub actor: UserKey,
    pub namespace: SessionNamespace,
}

impl SessionKey {
    #[must_use]
    pub fn new(conversation: ConversationKey, actor: UserKey, namespace: SessionNamespace) -> Self {
        Self {
            conversation,
            actor,
            namespace,
        }
    }

    fn scope(&self) -> ScopeKey {
        ScopeKey {
            conversation: self.conversation.clone(),
            actor: self.actor.clone(),
        }
    }
}

/// Whether a matched session also reaches normal routing.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SessionPolicy {
    #[default]
    Consume,
    Tap,
}

/// Options for one ask/wait operation.
#[derive(Clone, Debug)]
pub struct AskOptions {
    pub timeout: Duration,
    pub namespace: SessionNamespace,
    pub policy: SessionPolicy,
}

impl AskOptions {
    #[must_use]
    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            namespace: SessionNamespace::new("ask").expect("static namespace is valid"),
            policy: SessionPolicy::Consume,
        }
    }

    #[must_use]
    pub fn namespace(mut self, namespace: SessionNamespace) -> Self {
        self.namespace = namespace;
        self
    }

    #[must_use]
    pub const fn policy(mut self, policy: SessionPolicy) -> Self {
        self.policy = policy;
        self
    }
}

/// Event delivered to one exact session waiter.
#[derive(Debug)]
pub struct SessionEvent {
    event: Arc<DispatchEnvelope>,
    _ingress_retention: Arc<HierarchicalLease>,
}

impl SessionEvent {
    #[must_use]
    pub fn event(&self) -> &Event {
        self.event.event()
    }
}

/// Dynamic exact-scope interest consulted before full adapter decoding and
/// before the dispatch loop allocates a session command/oneshot pair.
#[derive(Clone)]
pub(crate) struct SessionInterest {
    shards: Arc<[RwLock<HashSet<ScopeKey>>]>,
    active: Arc<AtomicUsize>,
    hash_builder: Arc<RandomState>,
}

impl fmt::Debug for SessionInterest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionInterest")
            .field("shards", &self.shards.len())
            .finish_non_exhaustive()
    }
}

impl SessionInterest {
    fn new(shards: usize) -> Self {
        Self {
            shards: (0..shards)
                .map(|_| RwLock::new(HashSet::new()))
                .collect::<Vec<_>>()
                .into(),
            active: Arc::new(AtomicUsize::new(0)),
            hash_builder: Arc::new(RandomState::new()),
        }
    }

    fn shard_for(&self, scope: &ScopeKey) -> usize {
        shard_for(scope, self.shards.len(), &self.hash_builder)
    }

    pub(crate) fn accepts(&self, index: &DispatchIndex) -> bool {
        if index.kind != DispatchKind::Message || self.active.load(Ordering::Acquire) == 0 {
            return false;
        }
        let Some(scope) = ScopeKey::from_index(index) else {
            return false;
        };
        let shard = self.shard_for(&scope);
        self.shards[shard]
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .contains(&scope)
    }

    fn insert(&self, scope: ScopeKey) {
        let shard = self.shard_for(&scope);
        let inserted = self.shards[shard]
            .write()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert(scope);
        if inserted {
            self.active.fetch_add(1, Ordering::Release);
        }
    }

    fn remove(&self, scope: &ScopeKey) {
        let shard = self.shard_for(scope);
        let removed = self.shards[shard]
            .write()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(scope);
        if removed {
            self.active.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionDelivery {
    None,
    Consumed,
    Tap,
}

#[derive(Clone)]
#[doc(hidden)]
pub struct SessionRegistry {
    senders: Arc<[mpsc::Sender<SessionCommand>]>,
    interest: SessionInterest,
    capacity: Arc<Semaphore>,
    sequence: Arc<AtomicU64>,
    metrics: MetricsHandle,
}

impl SessionRegistry {
    pub(crate) fn new(
        shards: usize,
        commands_per_shard: usize,
        max_sessions: usize,
        metrics: MetricsHandle,
    ) -> (Self, Vec<SessionWorker>) {
        let interest = SessionInterest::new(shards);
        let mut senders = Vec::with_capacity(shards);
        let mut workers = Vec::with_capacity(shards);
        for _ in 0..shards {
            let (sender, receiver) = mpsc::channel(commands_per_shard);
            senders.push(sender);
            workers.push(SessionWorker {
                receiver,
                interest: interest.clone(),
            });
        }
        (
            Self {
                senders: senders.into(),
                interest,
                capacity: Arc::new(Semaphore::new(max_sessions)),
                sequence: Arc::new(AtomicU64::new(0)),
                metrics,
            },
            workers,
        )
    }

    pub(crate) fn interest(&self) -> SessionInterest {
        self.interest.clone()
    }

    pub(crate) async fn register(
        &self,
        key: SessionKey,
        timeout: Duration,
        policy: SessionPolicy,
    ) -> Result<SessionWaiter, SessionError> {
        if timeout.is_zero() || Instant::now().checked_add(timeout).is_none() {
            return Err(SessionError::InvalidTimeout);
        }
        let scope = key.scope();
        let capacity =
            Arc::clone(&self.capacity)
                .try_acquire_owned()
                .map_err(|error| match error {
                    TryAcquireError::NoPermits => SessionError::Full,
                    TryAcquireError::Closed => SessionError::Closed,
                })?;
        let registration_id = self
            .sequence
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .map_err(|_| SessionError::SequenceExhausted)?;
        let shard = self.interest.shard_for(&scope);
        let cancellation = CancellationToken::new();
        let (event_sender, event_receiver) = oneshot::channel();
        let (reply_sender, reply_receiver) = oneshot::channel();
        self.senders[shard]
            .send(SessionCommand::Register {
                key: key.clone(),
                registration_id,
                timeout,
                policy,
                capacity,
                cancellation: cancellation.clone(),
                event_sender,
                reply: reply_sender,
            })
            .await
            .map_err(|_| SessionError::Closed)?;
        reply_receiver.await.map_err(|_| SessionError::Closed)??;
        Ok(SessionWaiter {
            key,
            registration_id,
            sender: self.senders[shard].clone(),
            receiver: Some(event_receiver),
            cancellation,
            completed: false,
        })
    }

    pub(crate) async fn deliver(
        &self,
        event: Arc<DispatchEnvelope>,
        ingress_retention: Arc<HierarchicalLease>,
    ) -> Result<SessionDelivery, SessionError> {
        // Critical hot-path fast miss: no channel send, oneshot allocation, or
        // session-worker wakeup when no exact scope is active.
        if !self.interest.accepts(&event.index) {
            self.metrics.session_fast_miss();
            return Ok(SessionDelivery::None);
        }
        let Some(scope) = ScopeKey::from_index(&event.index) else {
            return Ok(SessionDelivery::None);
        };
        let shard = self.interest.shard_for(&scope);
        let (reply, receiver) = oneshot::channel();
        self.senders[shard]
            .send(SessionCommand::Deliver {
                scope,
                event,
                ingress_retention,
                reply,
            })
            .await
            .map_err(|_| SessionError::Closed)?;
        receiver.await.map_err(|_| SessionError::Closed)
    }
}

/// Registered one-shot waiter. Dropping it synchronously marks the entry
/// cancelled; the best-effort command only accelerates resource reclamation.
pub(crate) struct SessionWaiter {
    key: SessionKey,
    registration_id: u64,
    sender: mpsc::Sender<SessionCommand>,
    receiver: Option<oneshot::Receiver<Result<SessionEvent, SessionError>>>,
    cancellation: CancellationToken,
    completed: bool,
}

impl SessionWaiter {
    pub(crate) async fn wait(mut self) -> Result<SessionEvent, SessionError> {
        let receiver = self.receiver.take().ok_or(SessionError::Closed)?;
        let result = receiver.await.map_err(|_| SessionError::Closed)?;
        self.completed = true;
        result
    }
}

impl Drop for SessionWaiter {
    fn drop(&mut self) {
        if !self.completed {
            self.cancellation.cancel();
            let _ = self.sender.try_send(SessionCommand::Cancel {
                key: self.key.clone(),
                registration_id: self.registration_id,
            });
        }
    }
}

enum SessionCommand {
    Register {
        key: SessionKey,
        registration_id: u64,
        timeout: Duration,
        policy: SessionPolicy,
        capacity: OwnedSemaphorePermit,
        cancellation: CancellationToken,
        event_sender: oneshot::Sender<Result<SessionEvent, SessionError>>,
        reply: oneshot::Sender<Result<(), SessionError>>,
    },
    Deliver {
        scope: ScopeKey,
        event: Arc<DispatchEnvelope>,
        ingress_retention: Arc<HierarchicalLease>,
        reply: oneshot::Sender<SessionDelivery>,
    },
    Cancel {
        key: SessionKey,
        registration_id: u64,
    },
}

struct SessionEntry {
    key: SessionKey,
    registration_id: u64,
    policy: SessionPolicy,
    cancellation: CancellationToken,
    event_sender: oneshot::Sender<Result<SessionEvent, SessionError>>,
    _capacity: OwnedSemaphorePermit,
    deadline: DelayKey,
}

pub(crate) struct SessionWorker {
    receiver: mpsc::Receiver<SessionCommand>,
    interest: SessionInterest,
}

impl SessionWorker {
    pub(crate) async fn run(mut self) {
        let mut entries = HashMap::<ScopeKey, SessionEntry>::new();
        let mut deadlines = DelayQueue::<ScopeKey>::new();
        loop {
            tokio::select! {
                command = self.receiver.recv() => {
                    match command {
                        Some(command) => handle_command(
                            command,
                            &mut entries,
                            &mut deadlines,
                            &self.interest,
                        ),
                        None => {
                            close_all(&mut entries, &self.interest);
                            break;
                        }
                    }
                }
                expired = deadlines.next(), if !deadlines.is_empty() => {
                    if let Some(expired) = expired {
                        let scope = expired.into_inner();
                        if let Some(entry) = entries.remove(&scope) {
                            self.interest.remove(&scope);
                            let error = if entry.cancellation.is_cancelled() {
                                SessionError::Cancelled
                            } else {
                                SessionError::Timeout
                            };
                            let _ = entry.event_sender.send(Err(error));
                        }
                    }
                }
            }
        }
    }
}

fn handle_command(
    command: SessionCommand,
    entries: &mut HashMap<ScopeKey, SessionEntry>,
    deadlines: &mut DelayQueue<ScopeKey>,
    interest: &SessionInterest,
) {
    match command {
        SessionCommand::Register {
            key,
            registration_id,
            timeout,
            policy,
            capacity,
            cancellation,
            event_sender,
            reply,
        } => {
            let scope = key.scope();
            if entries.contains_key(&scope) {
                let _ = reply.send(Err(SessionError::Occupied));
                return;
            }
            if cancellation.is_cancelled() || event_sender.is_closed() {
                let _ = reply.send(Err(SessionError::Cancelled));
                return;
            }
            let deadline = deadlines.insert(scope.clone(), timeout);
            entries.insert(
                scope.clone(),
                SessionEntry {
                    key,
                    registration_id,
                    policy,
                    cancellation,
                    event_sender,
                    _capacity: capacity,
                    deadline,
                },
            );
            interest.insert(scope.clone());
            // If the caller was cancelled between enqueueing and registration,
            // the acknowledgement receiver is gone. Roll the registration back
            // immediately so it cannot consume an unrelated future message.
            if reply.send(Ok(())).is_err() {
                remove_entry(
                    &scope,
                    entries,
                    deadlines,
                    interest,
                    SessionError::Cancelled,
                );
            }
        }
        SessionCommand::Deliver {
            scope,
            event,
            ingress_retention,
            reply,
        } => {
            let delivery = if let Some(entry) = entries.remove(&scope) {
                let _ = deadlines.try_remove(&entry.deadline);
                interest.remove(&scope);
                if entry.cancellation.is_cancelled() || entry.event_sender.is_closed() {
                    let _ = entry.event_sender.send(Err(SessionError::Cancelled));
                    SessionDelivery::None
                } else {
                    let policy = entry.policy;
                    let sent = entry.event_sender.send(Ok(SessionEvent {
                        event,
                        _ingress_retention: ingress_retention,
                    }));
                    if sent.is_ok() {
                        match policy {
                            SessionPolicy::Consume => SessionDelivery::Consumed,
                            SessionPolicy::Tap => SessionDelivery::Tap,
                        }
                    } else {
                        SessionDelivery::None
                    }
                }
            } else {
                SessionDelivery::None
            };
            let _ = reply.send(delivery);
        }
        SessionCommand::Cancel {
            key,
            registration_id,
        } => {
            let scope = key.scope();
            if entries.get(&scope).is_some_and(|entry| {
                entry.key.namespace == key.namespace && entry.registration_id == registration_id
            }) {
                remove_entry(
                    &scope,
                    entries,
                    deadlines,
                    interest,
                    SessionError::Cancelled,
                );
            }
        }
    }
}

fn remove_entry(
    scope: &ScopeKey,
    entries: &mut HashMap<ScopeKey, SessionEntry>,
    deadlines: &mut DelayQueue<ScopeKey>,
    interest: &SessionInterest,
    error: SessionError,
) {
    if let Some(entry) = entries.remove(scope) {
        let _ = deadlines.try_remove(&entry.deadline);
        interest.remove(scope);
        let _ = entry.event_sender.send(Err(error));
    }
}

fn close_all(entries: &mut HashMap<ScopeKey, SessionEntry>, interest: &SessionInterest) {
    for (scope, entry) in entries.drain() {
        interest.remove(&scope);
        let _ = entry.event_sender.send(Err(SessionError::Closed));
    }
}

fn shard_for<T: Hash>(value: &T, shards: usize, hash_builder: &RandomState) -> usize {
    (hash_builder.hash_one(value) as usize) % shards
}
