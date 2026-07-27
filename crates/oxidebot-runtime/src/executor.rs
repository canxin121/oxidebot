use crate::{
    budget::{HierarchicalLease, QueueAcquireError, QueueLease, QueueLimiter},
    router::CompiledRouter,
    BotHandle, MetricsHandle, OverloadPolicy, QueueBudget,
};
use futures_util::{stream::FuturesUnordered, StreamExt};
use oxidebot_core::{BotSlot, EventEnvelope, ExecutionKey, MessageExecutionPartition};
use std::{
    collections::{hash_map::DefaultHasher, HashMap, HashSet, VecDeque},
    hash::{Hash, Hasher},
    sync::Arc,
};
use tokio::sync::{mpsc, OwnedSemaphorePermit, Semaphore, TryAcquireError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExecutorSubmit {
    Accepted,
    Dropped,
    Closed,
}

#[derive(Clone)]
pub(crate) struct ExecutorHandle {
    senders: Arc<[mpsc::Sender<DispatchJob>]>,
    global_limiter: QueueLimiter,
    per_bot_limiters: Arc<[QueueLimiter]>,
    overload: OverloadPolicy,
    message_partition: MessageExecutionPartition,
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

        // Every shard can absorb the global item maximum. The shared limiter
        // remains the actual total bound, so an admitted hot-shard event never
        // waits on a smaller local channel while holding global permits.
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
                metrics,
            },
            workers,
        )
    }

    pub(crate) async fn submit(
        &self,
        event: Arc<EventEnvelope>,
        bot: BotHandle,
        ingress_retention: Arc<QueueLease>,
    ) -> ExecutorSubmit {
        let bytes = event.incremental_retained_bytes();
        let Some(local_limiter) = self.per_bot_limiters.get(bot.slot().0 as usize) else {
            return ExecutorSubmit::Closed;
        };
        let leases = match self.acquire_hierarchical(local_limiter, bytes).await {
            Ok(leases) => leases,
            Err(QueueAcquireError::Full | QueueAcquireError::TooLarge) => {
                self.metrics.dropped_event();
                if matches!(self.overload, OverloadPolicy::Block) {
                    tracing::error!(event_id = %event.id, "event exceeds executor budget");
                }
                return ExecutorSubmit::Dropped;
            }
            Err(QueueAcquireError::Closed) => return ExecutorSubmit::Closed,
        };

        let key = event.index.execution_key(&event.id, self.message_partition);
        let shard = shard_for(&key, self.senders.len());
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
                    return ExecutorSubmit::Dropped;
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
        // Every caller acquires local then global, providing one consistent
        // ordering across all bots and resources.
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
    event: Arc<EventEnvelope>,
    bot: BotHandle,
    _leases: HierarchicalLease,
    _ingress_retention: Arc<QueueLease>,
}

struct JobCompletion {
    key: ExecutionKey,
    _bot_permit: OwnedSemaphorePermit,
    _leases: HierarchicalLease,
    _ingress_retention: Arc<QueueLease>,
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
        let mut running_keys = HashSet::<ExecutionKey>::new();
        let mut running = FuturesUnordered::new();
        let mut input_closed = false;

        loop {
            let mut attempts = ready.len();
            while running.len() < self.max_in_flight && attempts > 0 {
                attempts -= 1;
                let Some(key) = ready.pop_front() else {
                    break;
                };
                ready_set.remove(&key);
                if running_keys.contains(&key) {
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
                let bot_permit = match Arc::clone(semaphore).try_acquire_owned() {
                    Ok(permit) => permit,
                    Err(TryAcquireError::NoPermits) => {
                        if ready_set.insert(key.clone()) {
                            ready.push_back(key);
                        }
                        continue;
                    }
                    Err(TryAcquireError::Closed) => {
                        queues.remove(&key);
                        continue;
                    }
                };
                let Some(job) = queues.get_mut(&key).and_then(VecDeque::pop_front) else {
                    queues.remove(&key);
                    continue;
                };
                running_keys.insert(key);
                running.push(run_dispatch(job, bot_permit, Arc::clone(&self.router)));
                attempts = ready.len();
            }

            if input_closed && queues.is_empty() && running.is_empty() {
                break;
            }

            tokio::select! {
                job = self.receiver.recv(), if !input_closed => {
                    match job {
                        Some(job) => {
                            let key = job.key.clone();
                            let queue = queues.entry(key.clone()).or_default();
                            let was_empty = queue.is_empty();
                            queue.push_back(job);
                            if was_empty
                                && !running_keys.contains(&key)
                                && ready_set.insert(key.clone())
                            {
                                ready.push_back(key);
                            }
                        }
                        None => input_closed = true,
                    }
                }
                completion = running.next(), if !running.is_empty() => {
                    if let Some(completion) = completion {
                        running_keys.remove(&completion.key);
                        if queues
                            .get(&completion.key)
                            .is_some_and(|queue| !queue.is_empty())
                        {
                            if ready_set.insert(completion.key.clone()) {
                                ready.push_back(completion.key);
                            }
                        } else {
                            queues.remove(&completion.key);
                        }
                    }
                }
            }
        }
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

fn shard_for<T: Hash>(value: &T, shards: usize) -> usize {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    (hasher.finish() as usize) % shards
}
