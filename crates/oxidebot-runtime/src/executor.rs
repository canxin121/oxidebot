use crate::{
    budget::{QueueAcquireError, QueueLease, QueueLimiter},
    router::CompiledRouter,
    BotHandle, MetricsHandle, OverloadPolicy, QueueBudget,
};
use futures_util::{future::BoxFuture, stream::FuturesUnordered, FutureExt, StreamExt};
use oxidebot_core::{EventEnvelope, ExecutionKey};
use std::{
    collections::{hash_map::DefaultHasher, HashMap, HashSet, VecDeque},
    hash::{Hash, Hasher},
    sync::Arc,
};
use tokio::sync::mpsc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExecutorSubmit {
    Accepted,
    Dropped,
    Closed,
}

#[derive(Clone)]
pub(crate) struct ExecutorHandle {
    senders: Arc<[mpsc::Sender<DispatchJob>]>,
    limiter: QueueLimiter,
    overload: OverloadPolicy,
    metrics: MetricsHandle,
}

impl ExecutorHandle {
    pub(crate) fn new<S>(
        shards: usize,
        budget: QueueBudget,
        max_in_flight_per_shard: usize,
        overload: OverloadPolicy,
        router: Arc<CompiledRouter<S>>,
        metrics: MetricsHandle,
    ) -> (Self, Vec<ExecutorWorker<S>>)
    where
        S: Send + Sync + 'static,
    {
        let limiter = QueueLimiter::new(budget);
        let per_shard_capacity = budget.max_items.div_ceil(shards).max(1);
        let mut senders = Vec::with_capacity(shards);
        let mut workers = Vec::with_capacity(shards);
        for _ in 0..shards {
            let (sender, receiver) = mpsc::channel(per_shard_capacity);
            senders.push(sender);
            workers.push(ExecutorWorker {
                receiver,
                router: Arc::clone(&router),
                max_in_flight: max_in_flight_per_shard,
            });
        }
        (
            Self {
                senders: senders.into(),
                limiter,
                overload,
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
        let lease = match self.overload {
            OverloadPolicy::Block => self.limiter.acquire(event.estimated_bytes()).await,
            OverloadPolicy::DropNewest => self.limiter.try_acquire(event.estimated_bytes()),
        };
        let lease = match lease {
            Ok(lease) => lease,
            Err(QueueAcquireError::Full) => {
                self.metrics.dropped_event();
                return ExecutorSubmit::Dropped;
            }
            Err(QueueAcquireError::TooLarge) => {
                self.metrics.dropped_event();
                tracing::error!(event_id = %event.id, "event exceeds the executor byte budget");
                return ExecutorSubmit::Dropped;
            }
            Err(QueueAcquireError::Closed) => return ExecutorSubmit::Closed,
        };

        let key = event.index.execution_key(&event.id);
        let shard = shard_for(&key, self.senders.len());
        let job = DispatchJob {
            key,
            event,
            bot,
            _executor_lease: lease,
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
}

struct DispatchJob {
    key: ExecutionKey,
    event: Arc<EventEnvelope>,
    bot: BotHandle,
    _executor_lease: QueueLease,
    _ingress_retention: Arc<QueueLease>,
}

struct JobCompletion {
    key: ExecutionKey,
    _executor_lease: QueueLease,
    _ingress_retention: Arc<QueueLease>,
}

pub(crate) struct ExecutorWorker<S>
where
    S: Send + Sync + 'static,
{
    receiver: mpsc::Receiver<DispatchJob>,
    router: Arc<CompiledRouter<S>>,
    max_in_flight: usize,
}

impl<S> ExecutorWorker<S>
where
    S: Send + Sync + 'static,
{
    pub(crate) async fn run(mut self) {
        let mut queues = HashMap::<ExecutionKey, VecDeque<DispatchJob>>::new();
        let mut ready = VecDeque::<ExecutionKey>::new();
        let mut running_keys = HashSet::<ExecutionKey>::new();
        let mut running = FuturesUnordered::<BoxFuture<'static, JobCompletion>>::new();
        let mut input_closed = false;

        loop {
            schedule_jobs(
                &mut queues,
                &mut ready,
                &mut running_keys,
                &mut running,
                self.max_in_flight,
                Arc::clone(&self.router),
            );

            if input_closed && queues.is_empty() && running.is_empty() {
                break;
            }

            tokio::select! {
                job = self.receiver.recv(), if !input_closed => {
                    match job {
                        Some(job) => enqueue_job(job, &mut queues, &mut ready, &running_keys),
                        None => input_closed = true,
                    }
                }
                completion = running.next(), if !running.is_empty() => {
                    if let Some(completion) = completion {
                        running_keys.remove(&completion.key);
                        if queues.get(&completion.key).is_some_and(|queue| !queue.is_empty()) {
                            ready.push_back(completion.key);
                        } else {
                            queues.remove(&completion.key);
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
    running_keys: &HashSet<ExecutionKey>,
) {
    let key = job.key.clone();
    let queue = queues.entry(key.clone()).or_default();
    let was_empty = queue.is_empty();
    queue.push_back(job);
    if was_empty && !running_keys.contains(&key) {
        ready.push_back(key);
    }
}

fn schedule_jobs<S>(
    queues: &mut HashMap<ExecutionKey, VecDeque<DispatchJob>>,
    ready: &mut VecDeque<ExecutionKey>,
    running_keys: &mut HashSet<ExecutionKey>,
    running: &mut FuturesUnordered<BoxFuture<'static, JobCompletion>>,
    max_in_flight: usize,
    router: Arc<CompiledRouter<S>>,
) where
    S: Send + Sync + 'static,
{
    while running.len() < max_in_flight {
        let Some(key) = ready.pop_front() else {
            break;
        };
        if running_keys.contains(&key) {
            continue;
        }
        let job = queues.get_mut(&key).and_then(VecDeque::pop_front);
        let Some(job) = job else {
            queues.remove(&key);
            continue;
        };
        running_keys.insert(key.clone());
        let router = Arc::clone(&router);
        running.push(
            async move {
                let DispatchJob {
                    key,
                    event,
                    bot,
                    _executor_lease,
                    _ingress_retention,
                } = job;
                router.dispatch(event, bot).await;
                JobCompletion {
                    key,
                    _executor_lease,
                    _ingress_retention,
                }
            }
            .boxed(),
        );
    }
}

fn shard_for<T: Hash>(value: &T, shards: usize) -> usize {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    (hasher.finish() as usize) % shards
}
