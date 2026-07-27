use crate::{budget::QueueLease, SessionError};
use futures_util::StreamExt;
use oxidebot_core::{
    ConversationKey, EventEnvelope, EventIndex, EventKind, SessionNamespace, UserKey,
};
use std::{
    collections::{hash_map::DefaultHasher, HashMap, HashSet},
    fmt,
    hash::{Hash, Hasher},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, RwLock,
    },
    time::Duration,
};
use tokio::sync::{mpsc, oneshot, OwnedSemaphorePermit, Semaphore, TryAcquireError};
use tokio_util::time::{delay_queue::Key as DelayKey, DelayQueue};

/// Exact conversation and actor scope used for pre-decode session interest.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ScopeKey {
    conversation: ConversationKey,
    actor: UserKey,
}

impl ScopeKey {
    fn from_index(index: &EventIndex) -> Option<Self> {
        Some(Self {
            conversation: index.conversation.clone()?,
            actor: index.actor.clone()?,
        })
    }
}

/// Exact key for one dialogue or wait operation.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SessionKey {
    /// Conversation being awaited.
    pub conversation: ConversationKey,
    /// Actor whose next event is awaited.
    pub actor: UserKey,
    /// Logical dialogue namespace.
    pub namespace: SessionNamespace,
}

impl SessionKey {
    /// Creates an exact session key.
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
    /// Deliver only to the session waiter.
    #[default]
    Consume,
    /// Deliver to the waiter and continue normal routing afterward.
    Tap,
}

/// Options for one ask/wait operation.
#[derive(Clone, Debug)]
pub struct AskOptions {
    /// Maximum wait duration.
    pub timeout: Duration,
    /// Logical namespace.
    pub namespace: SessionNamespace,
    /// Delivery policy.
    pub policy: SessionPolicy,
}

impl AskOptions {
    /// Creates consume-once ask options.
    #[must_use]
    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            namespace: SessionNamespace::new("ask").expect("static session namespace is valid"),
            policy: SessionPolicy::Consume,
        }
    }

    /// Replaces the logical namespace.
    #[must_use]
    pub fn namespace(mut self, namespace: SessionNamespace) -> Self {
        self.namespace = namespace;
        self
    }

    /// Replaces the delivery policy.
    #[must_use]
    pub const fn policy(mut self, policy: SessionPolicy) -> Self {
        self.policy = policy;
        self
    }
}

/// Event delivered to one exact session waiter.
#[derive(Debug)]
pub struct SessionEvent {
    event: Arc<EventEnvelope>,
    _ingress_retention: Arc<QueueLease>,
}

impl SessionEvent {
    /// Returns the immutable canonical event.
    #[must_use]
    pub fn event(&self) -> &EventEnvelope {
        self.event.as_ref()
    }

    /// Returns a clone-cheap event handle.
    #[must_use]
    pub fn event_handle(&self) -> Arc<EventEnvelope> {
        Arc::clone(&self.event)
    }
}

/// Dynamic exact-scope interest consulted before full adapter decoding.
#[derive(Clone)]
pub(crate) struct SessionInterest {
    shards: Arc<[RwLock<HashSet<ScopeKey>>]>,
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
        let values = (0..shards)
            .map(|_| RwLock::new(HashSet::new()))
            .collect::<Vec<_>>();
        Self {
            shards: values.into(),
        }
    }

    pub(crate) fn accepts(&self, index: &EventIndex) -> bool {
        if index.kind != EventKind::MessageCreated {
            return false;
        }
        let Some(scope) = ScopeKey::from_index(index) else {
            return false;
        };
        let shard = shard_for(&scope, self.shards.len());
        self.shards[shard]
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .contains(&scope)
    }

    fn insert(&self, scope: ScopeKey) {
        let shard = shard_for(&scope, self.shards.len());
        self.shards[shard]
            .write()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert(scope);
    }

    fn remove(&self, scope: &ScopeKey) {
        let shard = shard_for(scope, self.shards.len());
        self.shards[shard]
            .write()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(scope);
    }
}

/// Result of attempting exact session delivery.
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
}

impl SessionRegistry {
    pub(crate) fn new(
        shards: usize,
        commands_per_shard: usize,
        max_sessions: usize,
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
        let scope = key.scope();
        let capacity =
            Arc::clone(&self.capacity)
                .try_acquire_owned()
                .map_err(|error| match error {
                    TryAcquireError::NoPermits => SessionError::Full,
                    TryAcquireError::Closed => SessionError::Closed,
                })?;
        let registration_id = self.sequence.fetch_add(1, Ordering::Relaxed);
        let shard = shard_for(&scope, self.senders.len());
        let (event_sender, event_receiver) = oneshot::channel();
        let (reply_sender, reply_receiver) = oneshot::channel();
        self.senders[shard]
            .send(SessionCommand::Register {
                key: key.clone(),
                registration_id,
                timeout,
                policy,
                capacity,
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
            completed: false,
        })
    }

    pub(crate) async fn deliver(
        &self,
        event: Arc<EventEnvelope>,
        ingress_retention: Arc<QueueLease>,
    ) -> Result<SessionDelivery, SessionError> {
        if event.index.kind != EventKind::MessageCreated {
            return Ok(SessionDelivery::None);
        }
        let Some(scope) = ScopeKey::from_index(&event.index) else {
            return Ok(SessionDelivery::None);
        };
        let shard = shard_for(&scope, self.senders.len());
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

/// Registered one-shot waiter. Dropping it cancels the registration best-effort.
pub(crate) struct SessionWaiter {
    key: SessionKey,
    registration_id: u64,
    sender: mpsc::Sender<SessionCommand>,
    receiver: Option<oneshot::Receiver<Result<SessionEvent, SessionError>>>,
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
        event_sender: oneshot::Sender<Result<SessionEvent, SessionError>>,
        reply: oneshot::Sender<Result<(), SessionError>>,
    },
    Deliver {
        scope: ScopeKey,
        event: Arc<EventEnvelope>,
        ingress_retention: Arc<QueueLease>,
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
        let mut input_closed = false;

        loop {
            if input_closed {
                close_all(&mut entries, &self.interest);
                break;
            }

            tokio::select! {
                command = self.receiver.recv() => {
                    match command {
                        Some(command) => handle_command(
                            command,
                            &mut entries,
                            &mut deadlines,
                            &self.interest,
                        ),
                        None => input_closed = true,
                    }
                }
                expired = deadlines.next(), if !deadlines.is_empty() => {
                    if let Some(expired) = expired {
                        let scope = expired.into_inner();
                        if let Some(entry) = entries.remove(&scope) {
                            self.interest.remove(&scope);
                            let _ = entry.event_sender.send(Err(SessionError::Timeout));
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
            event_sender,
            reply,
        } => {
            let scope = key.scope();
            let result = if entries.contains_key(&scope) {
                Err(SessionError::Occupied)
            } else {
                let deadline = deadlines.insert(scope.clone(), timeout);
                entries.insert(
                    scope.clone(),
                    SessionEntry {
                        key,
                        registration_id,
                        policy,
                        event_sender,
                        _capacity: capacity,
                        deadline,
                    },
                );
                interest.insert(scope);
                Ok(())
            };
            let _ = reply.send(result);
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
                if let Some(entry) = entries.remove(&scope) {
                    let _ = deadlines.try_remove(&entry.deadline);
                    interest.remove(&scope);
                    let _ = entry.event_sender.send(Err(SessionError::Closed));
                }
            }
        }
    }
}

fn close_all(entries: &mut HashMap<ScopeKey, SessionEntry>, interest: &SessionInterest) {
    for (scope, entry) in entries.drain() {
        interest.remove(&scope);
        let _ = entry.event_sender.send(Err(SessionError::Closed));
    }
}

fn shard_for<T: Hash>(value: &T, shards: usize) -> usize {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    (hasher.finish() as usize) % shards
}
