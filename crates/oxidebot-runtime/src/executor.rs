use crate::{
    budget::{HierarchicalLease, QueueAcquireError, QueueLimiter},
    router::CompiledRouter,
    BotHandle, MetricsHandle, OverloadPolicy, QueueBudget,
};
use futures_util::{stream::FuturesUnordered, StreamExt};
use oxidebot_core::event::kernel::{DispatchEnvelope, MessageExecutionPartition};
use oxidebot_core::{BotSlot, ExecutionKey};
use std::{
    collections::{hash_map::RandomState, HashMap, HashSet, VecDeque},
    hash::{BuildHasher, Hash},
    sync::Arc,
};
use tokio::sync::{mpsc, OwnedSemaphorePermit, Semaphore, TryAcquireError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExecutorSubmit {
    Accepted,
    /// Explicit loss requested by `DropNewest`.
    DroppedByPolicy,
    /// The event can never fit the configured executor envelope.
    RejectedTooLarge,
    Closed,
}

#[derive(Clone)]
pub(crate) struct ExecutorHandle {
    senders: Arc<[mpsc::Sender<DispatchJob>]>,
    global_limiter: QueueLimiter,
    per_bot_limiters: Arc<[QueueLimiter]>,
    overload: OverloadPolicy,
    message_partition: MessageExecutionPartition,
    hash_builder: RandomState,
    metrics: MetricsHandle,
}

impl ExecutorHandle {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new<S>(
        shards: usize,
        bot_count: usize,
        global_budget: QueueBudget,
        per_bot_budget: QueueBudget,
        max_in_flight_per_shard: usize,
        max_in_flight_per_bot: usize,
        overload: OverloadPolicy,
        message_partition: MessageExecutionPartition,
        router: Arc<CompiledRouter<S>>,
        metrics: MetricsHandle,
    ) -> (Self, Vec<ExecutorWorker<S>>)
    where
        S: Send + Sync + 'static,
    {
        let global_limiter = QueueLimiter::new(global_budget);
        let per_bot_limiters = (0..bot_count)
            .map(|_| QueueLimiter::new(per_bot_budget))
            .collect::<Vec<_>>();
        let in_flight_by_bot: Arc<[Arc<Semaphore>]> = (0..bot_count)
            .map(|_| Arc::new(Semaphore::new(max_in_flight_per_bot)))
            .collect::<Vec<_>>()
            .into();
        let mut senders = Vec::with_capacity(shards);
        let mut workers = Vec::with_capacity(shards);

        // Every shard can absorb the global item maximum. The shared limiter is
        // the actual total bound, so a hot shard never blocks on a smaller local
        // channel while holding global queue permits.
        for _ in 0..shards {
            let (sender, receiver) = mpsc::channel(global_budget.max_items);
            senders.push(sender);
            workers.push(ExecutorWorker {
                receiver,
                router: Arc::clone(&router),
                max_in_flight: max_in_flight_per_shard,
                in_flight_by_bot: Arc::clone(&in_flight_by_bot),
            });
        }
        (
            Self {
                senders: senders.into(),
                global_limiter,
                per_bot_limiters: per_bot_limiters.into(),
                overload,
                message_partition,
                hash_builder: RandomState::new(),
                metrics,
            },
            workers,
        )
    }

    pub(crate) async fn submit(
        &self,
        event: Arc<DispatchEnvelope>,
        bot: BotHandle,
        ingress_retention: Arc<HierarchicalLease>,
    ) -> ExecutorSubmit {
        let bytes = event.incremental_retained_bytes();
        let Some(local_limiter) = self.per_bot_limiters.get(bot.slot().0 as usize) else {
            return ExecutorSubmit::Closed;
        };
        let leases = match self.acquire_hierarchical(local_limiter, bytes).await {
            Ok(leases) => leases,
            Err(QueueAcquireError::Full) => {
                debug_assert_eq!(self.overload, OverloadPolicy::DropNewest);
                self.metrics.dropped_event();
                return ExecutorSubmit::DroppedByPolicy;
            }
            Err(QueueAcquireError::TooLarge) => {
                self.metrics.rejected_event();
                tracing::error!(event_id = %event.id, bytes, "event exceeds executor byte budget");
                return ExecutorSubmit::RejectedTooLarge;
            }
            Err(QueueAcquireError::Closed) => return ExecutorSubmit::Closed,
        };

        let key = event.index.execution_key(&event.id, self.message_partition);
        let shard = shard_for(&key, self.senders.len(), &self.hash_builder);
        let job = DispatchJob {
            key,
            bot_slot: bot.slot(),
            event,
            bot,
            _leases: leases,
            _ingress_retention: ingress_retention,
        };
        let sent = match self.overload {
            OverloadPolicy::Block => self.senders[shard].send(job).await.is_ok(),
            OverloadPolicy::DropNewest => match self.senders[shard].try_send(job) {
                Ok(()) => true,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    self.metrics.dropped_event();
                    return ExecutorSubmit::DroppedByPolicy;
                }
                Err(mpsc::error::TrySendError::Closed(_)) => false,
            },
        };
        if !sent {
            return ExecutorSubmit::Closed;
        }
        self.metrics.dispatched_event();
        ExecutorSubmit::Accepted
    }

    async fn acquire_hierarchical(
        &self,
        local: &QueueLimiter,
        bytes: usize,
    ) -> Result<HierarchicalLease, QueueAcquireError> {
        // Every caller acquires local then global. A noisy bot therefore cannot
        // reserve the entire global envelope while waiting for its local share.
        let local_lease = match self.overload {
            OverloadPolicy::Block => local.acquire(bytes).await?,
            OverloadPolicy::DropNewest => local.try_acquire(bytes)?,
        };
        let global_lease = match self.overload {
            OverloadPolicy::Block => self.global_limiter.acquire(bytes).await,
            OverloadPolicy::DropNewest => self.global_limiter.try_acquire(bytes),
        }?;
        Ok(HierarchicalLease::new(local_lease, global_lease))
    }
}

struct DispatchJob {
    key: ExecutionKey,
    bot_slot: BotSlot,
    event: Arc<DispatchEnvelope>,
    bot: BotHandle,
    _leases: HierarchicalLease,
    _ingress_retention: Arc<HierarchicalLease>,
}

struct JobCompletion {
    key: ExecutionKey,
    _bot_permit: OwnedSemaphorePermit,
    _leases: HierarchicalLease,
    _ingress_retention: Arc<HierarchicalLease>,
}

struct PermitGrant {
    key: ExecutionKey,
    permit: Option<OwnedSemaphorePermit>,
}

pub(crate) struct ExecutorWorker<S>
where
    S: Send + Sync + 'static,
{
    receiver: mpsc::Receiver<DispatchJob>,
    router: Arc<CompiledRouter<S>>,
    max_in_flight: usize,
    in_flight_by_bot: Arc<[Arc<Semaphore>]>,
}

impl<S> ExecutorWorker<S>
where
    S: Send + Sync + 'static,
{
    pub(crate) async fn run(mut self) {
        let mut queues = HashMap::<ExecutionKey, VecDeque<DispatchJob>>::new();
        let mut ready = VecDeque::<ExecutionKey>::new();
        let mut ready_set = HashSet::<ExecutionKey>::new();
        let mut waiting_keys = HashSet::<ExecutionKey>::new();
        let mut running_keys = HashSet::<ExecutionKey>::new();
        let mut running = FuturesUnordered::new();
        let mut permit_waiters = FuturesUnordered::new();
        let mut input_closed = false;

        loop {
            // Waiting for a shared per-bot permit must not consume a local running
            // slot: otherwise one saturated bot could occupy every slot in this
            // shard and prevent unrelated bots from running. Permit waiters have
            // their own bounded population, and are only polled into a running job
            // when this shard has execution capacity.
            let mut attempts = ready.len();
            while running.len() < self.max_in_flight && attempts > 0 {
                attempts -= 1;
                let Some(key) = ready.pop_front() else {
                    break;
                };
                ready_set.remove(&key);
                if running_keys.contains(&key) || waiting_keys.contains(&key) {
                    continue;
                }
                let Some(bot_slot) = queues
                    .get(&key)
                    .and_then(|queue| queue.front())
                    .map(|job| job.bot_slot)
                else {
                    queues.remove(&key);
                    continue;
                };
                let Some(semaphore) = self.in_flight_by_bot.get(bot_slot.0 as usize) else {
                    queues.remove(&key);
                    continue;
                };
                match Arc::clone(semaphore).try_acquire_owned() {
                    Ok(permit) => {
                        if let Some(job) = take_job(&key, &mut queues) {
                            running_keys.insert(key);
                            running.push(run_dispatch(job, permit, Arc::clone(&self.router)));
                        }
                    }
                    Err(TryAcquireError::NoPermits) => {
                        if permit_waiters.len() < self.max_in_flight {
                            waiting_keys.insert(key.clone());
                            permit_waiters.push(wait_for_bot_permit(key, Arc::clone(semaphore)));
                        } else if ready_set.insert(key.clone()) {
                            // A bounded number of semaphore waiters is enough to
                            // guarantee wake-ups. Leave excess keys ready so they
                            // can be reconsidered after any local completion or
                            // permit grant without allocating one future per key.
                            ready.push_back(key);
                        }
                    }
                    Err(TryAcquireError::Closed) => {
                        queues.remove(&key);
                    }
                }
            }

            if input_closed && queues.is_empty() && running.is_empty() && permit_waiters.is_empty()
            {
                break;
            }

            tokio::select! {
                job = self.receiver.recv(), if !input_closed => {
                    match job {
                        Some(job) => enqueue_job(
                            job,
                            &mut queues,
                            &mut ready,
                            &mut ready_set,
                            &running_keys,
                            &waiting_keys,
                        ),
                        None => input_closed = true,
                    }
                }
                completion = running.next(), if !running.is_empty() => {
                    if let Some(completion) = completion {
                        running_keys.remove(&completion.key);
                        requeue_if_pending(
                            completion.key,
                            &mut queues,
                            &mut ready,
                            &mut ready_set,
                            &waiting_keys,
                        );
                    }
                }
                grant = permit_waiters.next(), if !permit_waiters.is_empty() && running.len() < self.max_in_flight => {
                    if let Some(grant) = grant {
                        waiting_keys.remove(&grant.key);
                        if let Some(permit) = grant.permit {
                            if let Some(job) = take_job(&grant.key, &mut queues) {
                                running_keys.insert(grant.key);
                                running.push(run_dispatch(job, permit, Arc::clone(&self.router)));
                            }
                        } else {
                            queues.remove(&grant.key);
                        }
                    }
                }
            }
        }
    }
}

fn enqueue_job(
    job: DispatchJob,
    queues: &mut HashMap<ExecutionKey, VecDeque<DispatchJob>>,
    ready: &mut VecDeque<ExecutionKey>,
    ready_set: &mut HashSet<ExecutionKey>,
    running_keys: &HashSet<ExecutionKey>,
    waiting_keys: &HashSet<ExecutionKey>,
) {
    let key = job.key.clone();
    let queue = queues.entry(key.clone()).or_default();
    let was_empty = queue.is_empty();
    queue.push_back(job);
    if was_empty
        && !running_keys.contains(&key)
        && !waiting_keys.contains(&key)
        && ready_set.insert(key.clone())
    {
        ready.push_back(key);
    }
}

fn requeue_if_pending(
    key: ExecutionKey,
    queues: &mut HashMap<ExecutionKey, VecDeque<DispatchJob>>,
    ready: &mut VecDeque<ExecutionKey>,
    ready_set: &mut HashSet<ExecutionKey>,
    waiting_keys: &HashSet<ExecutionKey>,
) {
    if queues.get(&key).is_some_and(|queue| !queue.is_empty()) {
        if !waiting_keys.contains(&key) && ready_set.insert(key.clone()) {
            ready.push_back(key);
        }
    } else {
        queues.remove(&key);
    }
}

fn take_job(
    key: &ExecutionKey,
    queues: &mut HashMap<ExecutionKey, VecDeque<DispatchJob>>,
) -> Option<DispatchJob> {
    let job = queues.get_mut(key).and_then(VecDeque::pop_front);
    if job.is_none() {
        queues.remove(key);
    }
    job
}

async fn wait_for_bot_permit(key: ExecutionKey, semaphore: Arc<Semaphore>) -> PermitGrant {
    PermitGrant {
        key,
        permit: semaphore.acquire_owned().await.ok(),
    }
}

async fn run_dispatch<S>(
    job: DispatchJob,
    bot_permit: OwnedSemaphorePermit,
    router: Arc<CompiledRouter<S>>,
) -> JobCompletion
where
    S: Send + Sync + 'static,
{
    let DispatchJob {
        key,
        bot_slot: _,
        event,
        bot,
        _leases,
        _ingress_retention,
    } = job;
    router.dispatch(event, bot).await;
    JobCompletion {
        key,
        _bot_permit: bot_permit,
        _leases,
        _ingress_retention,
    }
}

fn shard_for<T: Hash>(value: &T, shards: usize, hash_builder: &RandomState) -> usize {
    (hash_builder.hash_one(value) as usize) % shards
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        bot::{BotDescriptor, BotServices, CommandWorker, GlobalCommandCapacity},
        handler::PreparedHandler,
        session::SessionRegistry,
        RuntimeMetrics, ShutdownSignal,
    };
    use async_trait::async_trait;
    use oxidebot_core::event::kernel::{DispatchDraft, DispatchIndex};
    use oxidebot_core::{
        api::{payload::SendMessageTarget, response::SendMessageResponse},
        event::{Event, EventType, MessageEvent},
        source::{
            message::{Message, MessageSegment},
            user::User,
        },
        BotId, CallApiTrait, CompactId, ConversationKey, EventId, PlatformId, UserKey,
    };
    use std::time::{Duration, Instant};
    use tokio_util::sync::CancellationToken;

    struct NoopApi;

    #[async_trait]
    impl CallApiTrait for NoopApi {
        async fn send_message(
            &self,
            _message: Vec<MessageSegment>,
            _target: SendMessageTarget,
        ) -> anyhow::Result<Vec<SendMessageResponse>> {
            Ok(vec![SendMessageResponse {
                sent_message_id: "1".to_owned(),
            }])
        }
    }

    fn event(platform: &PlatformId) -> Arc<DispatchEnvelope> {
        let bot = BotSlot(0);
        let conversation = ConversationKey::new(bot, CompactId::from("room"));
        let actor = UserKey::new(bot, CompactId::from("user"));
        let mut index = DispatchIndex::event(bot, platform.clone(), EventType::Message);
        index.conversation = Some(conversation.clone());
        index.actor = Some(actor.clone());
        Arc::new(
            DispatchDraft::new(
                EventId::new("wake-up").expect("static event ID"),
                index,
                Event::MessageEvent(MessageEvent {
                    id: "wake-up".to_owned(),
                    time: None,
                    sender: User {
                        id: "user".to_owned(),
                        ..User::default()
                    },
                    group: None,
                    message: Message {
                        id: "1".to_owned(),
                        segments: vec![MessageSegment::text("hello")],
                        options: Default::default(),
                    },
                }),
                2_048,
            )
            .finalize(0, Instant::now(), None),
        )
    }

    #[tokio::test]
    async fn externally_released_bot_permit_wakes_an_idle_shard() {
        let metrics = Arc::new(RuntimeMetrics::default());
        let (sessions, _session_workers) = SessionRegistry::new(1, 8, 8, Arc::clone(&metrics));
        let router = Arc::new(CompiledRouter::compile(
            Vec::<PreparedHandler<()>>::new(),
            Vec::new(),
            Arc::new(()),
            sessions,
            ShutdownSignal::new(CancellationToken::new()),
            crate::router::RouterLimits {
                handler_timeout: None,
                max_handler_replies: 8,
            },
            Arc::clone(&metrics),
        ));

        let platform = PlatformId::new("test").expect("static platform");
        let services = BotServices::new(Arc::new(NoopApi));
        let (bot, _command_worker) = CommandWorker::build(
            BotSlot(0),
            BotDescriptor::new(platform.clone(), BotId::new("bot").expect("static bot ID")),
            services,
            crate::budget::PriorityQueueLimiter::new(QueueBudget::new(8, 64 * 1024), 16 * 1024),
            GlobalCommandCapacity::new(2, 1),
            QueueBudget::new(8, 64 * 1024),
            16 * 1024,
            OverloadPolicy::Block,
            2,
            1,
            8,
            Some(Duration::from_secs(1)),
            Some(Duration::from_secs(2)),
            0,
            Duration::from_millis(1),
            Duration::from_millis(1),
            Arc::clone(&metrics),
        );

        let bot_semaphore = Arc::new(Semaphore::new(1));
        let externally_held = Arc::clone(&bot_semaphore)
            .acquire_owned()
            .await
            .expect("semaphore open");
        let (sender, receiver) = mpsc::channel(4);
        let worker = ExecutorWorker {
            receiver,
            router,
            max_in_flight: 1,
            in_flight_by_bot: vec![bot_semaphore].into(),
        };
        let worker_task = tokio::spawn(worker.run());

        let local = QueueLimiter::new(QueueBudget::new(4, 64 * 1024))
            .acquire(1024)
            .await
            .expect("local budget");
        let global = QueueLimiter::new(QueueBudget::new(4, 64 * 1024))
            .acquire(1024)
            .await
            .expect("global budget");
        let ingress_local = QueueLimiter::new(QueueBudget::new(4, 64 * 1024))
            .acquire(1024)
            .await
            .expect("local ingress budget");
        let ingress_global = QueueLimiter::new(QueueBudget::new(4, 64 * 1024))
            .acquire(1024)
            .await
            .expect("global ingress budget");
        let ingress = Arc::new(HierarchicalLease::new(ingress_local, ingress_global));
        let envelope = event(&platform);
        let key = envelope
            .index
            .execution_key(&envelope.id, MessageExecutionPartition::Conversation);
        sender
            .send(DispatchJob {
                key,
                bot_slot: BotSlot(0),
                event: envelope,
                bot,
                _leases: HierarchicalLease::new(local, global),
                _ingress_retention: ingress,
            })
            .await
            .expect("worker channel open");
        drop(sender);

        tokio::time::sleep(Duration::from_millis(20)).await;
        drop(externally_held);

        tokio::time::timeout(Duration::from_secs(1), worker_task)
            .await
            .expect("worker must wake when another shard releases the bot permit")
            .expect("worker task succeeds");
    }
}
