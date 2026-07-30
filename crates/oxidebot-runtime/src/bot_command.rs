use super::{BotDescriptor, BotHandle, BotServices};

use crate::{
    budget::{HierarchicalLease, PriorityQueueLimiter, QueueAcquireError},
    CommandError, MetricsHandle, OverloadPolicy, QueueBudget,
};
use futures_util::{stream::FuturesUnordered, StreamExt};
use oxidebot_core::BotSlot;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    sync::{mpsc, oneshot, OwnedSemaphorePermit, Semaphore},
    time::Instant,
};

#[path = "bot_command_execution.rs"]
mod execution;
#[path = "bot_command_operation.rs"]
mod operation;

use execution::execute_command;
pub(super) use operation::{ApiCommandResult, CommandOperation};
use operation::{CommandKey, CommandPriority};

#[derive(Clone)]
pub(super) struct CommandClient {
    sender: mpsc::Sender<CommandEnvelope>,
    local_limiter: PriorityQueueLimiter,
    global_limiter: PriorityQueueLimiter,
    overload: OverloadPolicy,
    max_command_bytes: usize,
    total_timeout: Option<Duration>,
    sequence: Arc<AtomicU64>,
    metrics: MetricsHandle,
}

impl CommandClient {
    pub(super) async fn call(
        &self,
        operation: CommandOperation,
    ) -> std::result::Result<ApiCommandResult, CommandError> {
        let (sender, receiver) = oneshot::channel();
        let deadline = self.submit(operation, Some(sender)).await?;
        await_before(deadline, receiver)
            .await?
            .map_err(|_| CommandError::Closed)?
    }

    async fn submit(
        &self,
        operation: CommandOperation,
        reply: Option<ApiCommandReply>,
    ) -> std::result::Result<Option<Instant>, CommandError> {
        let deadline = self
            .total_timeout
            .and_then(|timeout| Instant::now().checked_add(timeout));
        if self.total_timeout.is_some() && deadline.is_none() {
            return Err(CommandError::InvalidModel(
                "command total timeout exceeds the platform clock range".into(),
            ));
        }

        let bytes = operation.retained_bytes();
        if bytes > self.max_command_bytes {
            return Err(CommandError::PayloadTooLarge);
        }
        let priority = operation.priority();
        let local = acquire_queue(
            &self.local_limiter,
            priority,
            self.overload,
            bytes,
            deadline,
        )
        .await?;
        let global = acquire_queue(
            &self.global_limiter,
            priority,
            self.overload,
            bytes,
            deadline,
        )
        .await?;
        let mut current = self.sequence.load(Ordering::Relaxed);
        let sequence = loop {
            let next = current
                .checked_add(1)
                .ok_or(CommandError::SequenceExhausted)?;
            match self.sequence.compare_exchange_weak(
                current,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(previous) => break previous,
                Err(observed) => current = observed,
            }
        };
        let envelope = CommandEnvelope {
            key: operation.key(sequence),
            sequence,
            deadline,
            priority,
            operation,
            reply,
            submitted_at: Instant::now(),
            _leases: HierarchicalLease::new(local, global),
        };
        match self.overload {
            OverloadPolicy::Block => await_before(deadline, self.sender.send(envelope))
                .await?
                .map_err(|_| CommandError::Closed)?,
            OverloadPolicy::DropNewest => {
                self.sender
                    .try_send(envelope)
                    .map_err(|error| match error {
                        mpsc::error::TrySendError::Full(_) => CommandError::Full,
                        mpsc::error::TrySendError::Closed(_) => CommandError::Closed,
                    })?;
            }
        }
        self.metrics.outbound_queued();
        Ok(deadline)
    }
}

async fn acquire_queue(
    limiter: &PriorityQueueLimiter,
    priority: CommandPriority,
    overload: OverloadPolicy,
    bytes: usize,
    deadline: Option<Instant>,
) -> Result<crate::budget::QueueLease, CommandError> {
    match (priority, overload) {
        (CommandPriority::High, OverloadPolicy::Block) => {
            await_before(deadline, limiter.acquire_high(bytes))
                .await?
                .map_err(map_command_admission)
        }
        (CommandPriority::High, OverloadPolicy::DropNewest) => limiter
            .try_acquire_high(bytes)
            .map_err(map_command_admission),
        (CommandPriority::Normal, OverloadPolicy::Block) => {
            await_before(deadline, limiter.acquire_normal(bytes))
                .await?
                .map_err(map_command_admission)
        }
        (CommandPriority::Normal, OverloadPolicy::DropNewest) => limiter
            .try_acquire_normal(bytes)
            .map_err(map_command_admission),
    }
}

async fn await_before<T>(
    deadline: Option<Instant>,
    future: impl std::future::Future<Output = T>,
) -> Result<T, CommandError> {
    match deadline {
        Some(deadline) => tokio::time::timeout_at(deadline, future)
            .await
            .map_err(|_| CommandError::DeadlineExceeded),
        None => Ok(future.await),
    }
}

fn map_command_admission(error: QueueAcquireError) -> CommandError {
    match error {
        QueueAcquireError::Closed => CommandError::Closed,
        QueueAcquireError::Full => CommandError::Full,
        QueueAcquireError::TooLarge => CommandError::PayloadTooLarge,
    }
}

fn push_ready(
    priority: CommandPriority,
    key: CommandKey,
    high: &mut VecDeque<CommandKey>,
    normal: &mut VecDeque<CommandKey>,
) {
    match priority {
        CommandPriority::High => high.push_back(key),
        CommandPriority::Normal => normal.push_back(key),
    }
}

type ApiCommandReply = oneshot::Sender<std::result::Result<ApiCommandResult, CommandError>>;

struct CommandEnvelope {
    key: CommandKey,
    sequence: u64,
    deadline: Option<Instant>,
    priority: CommandPriority,
    operation: CommandOperation,
    reply: Option<ApiCommandReply>,
    submitted_at: Instant,
    _leases: HierarchicalLease,
}

impl CommandEnvelope {
    fn is_abandoned(&self) -> bool {
        self.reply.as_ref().is_some_and(|reply| reply.is_closed())
    }
}

struct CommandCompletion {
    key: CommandKey,
    priority: CommandPriority,
}

#[derive(Clone)]
pub(crate) struct GlobalCommandCapacity {
    normal: Arc<Semaphore>,
    reserved_high: Arc<Semaphore>,
}

enum GlobalCommandPermit {
    Normal(OwnedSemaphorePermit),
    ReservedHigh(OwnedSemaphorePermit),
}

impl GlobalCommandCapacity {
    pub(crate) fn new(total: usize, reserved_high: usize) -> Self {
        Self {
            normal: Arc::new(Semaphore::new(total - reserved_high)),
            reserved_high: Arc::new(Semaphore::new(reserved_high)),
        }
    }

    async fn acquire(
        &self,
        priority: CommandPriority,
        deadline: Option<Instant>,
    ) -> Result<GlobalCommandPermit, CommandError> {
        let acquire = async {
            match priority {
                CommandPriority::Normal => Arc::clone(&self.normal)
                    .acquire_owned()
                    .await
                    .map(GlobalCommandPermit::Normal)
                    .map_err(|_| CommandError::Closed),
                CommandPriority::High => {
                    // Prefer ordinary global capacity and preserve the high
                    // reserve for acknowledgements arriving under saturation.
                    match Arc::clone(&self.normal).try_acquire_owned() {
                        Ok(permit) => return Ok(GlobalCommandPermit::Normal(permit)),
                        Err(tokio::sync::TryAcquireError::Closed) => {
                            return Err(CommandError::Closed)
                        }
                        Err(tokio::sync::TryAcquireError::NoPermits) => {}
                    }
                    match Arc::clone(&self.reserved_high).try_acquire_owned() {
                        Ok(permit) => return Ok(GlobalCommandPermit::ReservedHigh(permit)),
                        Err(tokio::sync::TryAcquireError::Closed) => {
                            return Err(CommandError::Closed)
                        }
                        Err(tokio::sync::TryAcquireError::NoPermits) => {}
                    }
                    tokio::select! {
                        permit = Arc::clone(&self.normal).acquire_owned() => permit
                            .map(GlobalCommandPermit::Normal)
                            .map_err(|_| CommandError::Closed),
                        permit = Arc::clone(&self.reserved_high).acquire_owned() => permit
                            .map(GlobalCommandPermit::ReservedHigh)
                            .map_err(|_| CommandError::Closed),
                    }
                }
            }
        };
        await_before(deadline, acquire).await?
    }
}

impl GlobalCommandPermit {
    fn touch(&self) {
        match self {
            Self::Normal(permit) | Self::ReservedHigh(permit) => {
                let _ = permit;
            }
        }
    }
}

#[derive(Clone, Default)]
struct BotCooldown(Arc<std::sync::Mutex<Option<Instant>>>);

impl BotCooldown {
    fn active_until(&self) -> Option<Instant> {
        let mut state = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        match *state {
            Some(until) if until > Instant::now() => Some(until),
            Some(_) => {
                *state = None;
                None
            }
            None => None,
        }
    }

    async fn wait(&self, deadline: Option<Instant>) -> Result<(), CommandError> {
        loop {
            let until = self.active_until();
            let Some(until) = until else {
                return Ok(());
            };
            if deadline.is_some_and(|deadline| until >= deadline) {
                return Err(CommandError::DeadlineExceeded);
            }
            await_before(deadline, tokio::time::sleep_until(until)).await?;
        }
    }

    fn extend(&self, delay: Duration) -> Result<(), CommandError> {
        let until = Instant::now().checked_add(delay).ok_or_else(|| {
            CommandError::InvalidModel(
                "platform retry delay exceeds the monotonic clock range".into(),
            )
        })?;
        let mut state = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        if state.as_ref().is_none_or(|current| *current < until) {
            *state = Some(until);
        }
        Ok(())
    }
}

/// Fixed per-bot worker combining bounded admission, key ordering, weighted
/// priority, panic isolation, reserved interaction capacity, and limited concurrency.
pub(crate) struct CommandWorker {
    receiver: mpsc::Receiver<CommandEnvelope>,
    services: BotServices,
    max_in_flight: usize,
    reserved_high: usize,
    global_capacity: GlobalCommandCapacity,
    cooldown: BotCooldown,
    high_priority_burst: usize,
    attempt_timeout: Option<Duration>,
    max_retries: u8,
    retry_base: Duration,
    retry_max: Duration,
    metrics: MetricsHandle,
}

impl CommandWorker {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build(
        slot: BotSlot,
        descriptor: BotDescriptor,
        services: BotServices,
        global_limiter: PriorityQueueLimiter,
        global_capacity: GlobalCommandCapacity,
        local_budget: QueueBudget,
        max_command_bytes: usize,
        overload: OverloadPolicy,
        max_in_flight: usize,
        reserved_high: usize,
        high_priority_burst: usize,
        attempt_timeout: Option<Duration>,
        total_timeout: Option<Duration>,
        max_retries: u8,
        retry_base: Duration,
        retry_max: Duration,
        metrics: MetricsHandle,
    ) -> (BotHandle, Self) {
        let (sender, receiver) = mpsc::channel(local_budget.max_items);
        let client = CommandClient {
            sender,
            local_limiter: PriorityQueueLimiter::new(local_budget, max_command_bytes),
            global_limiter,
            overload,
            max_command_bytes,
            total_timeout,
            sequence: Arc::new(AtomicU64::new(0)),
            metrics: Arc::clone(&metrics),
        };
        let capabilities = services.capabilities();
        let bot_capabilities = Arc::clone(&services.bot_capabilities);
        let api = services.api.clone();
        let bot = BotHandle::new(
            slot,
            descriptor,
            capabilities,
            bot_capabilities,
            api,
            client,
        );
        (
            bot,
            Self {
                receiver,
                services,
                max_in_flight,
                reserved_high,
                global_capacity,
                cooldown: BotCooldown::default(),
                high_priority_burst,
                attempt_timeout,
                max_retries,
                retry_base,
                retry_max,
                metrics,
            },
        )
    }

    pub(crate) async fn run(mut self) {
        let mut queues: HashMap<CommandKey, VecDeque<CommandEnvelope>> = HashMap::new();
        let mut high_ready = VecDeque::new();
        let mut normal_ready = VecDeque::new();
        let mut ready_set = HashSet::new();
        let mut active = HashSet::new();
        let mut running = FuturesUnordered::new();
        let mut input_closed = false;
        let mut consecutive_high = 0_usize;
        let mut running_normal = 0_usize;
        let normal_limit = self.max_in_flight - self.reserved_high;

        loop {
            while running.len() < self.max_in_flight {
                let high_available = !high_ready.is_empty();
                let normal_available = running_normal < normal_limit && !normal_ready.is_empty();
                let key = if high_available
                    && (!normal_available || consecutive_high < self.high_priority_burst)
                {
                    consecutive_high = consecutive_high.saturating_add(1);
                    high_ready.pop_front()
                } else if normal_available {
                    consecutive_high = 0;
                    normal_ready.pop_front()
                } else if high_available {
                    consecutive_high = consecutive_high.saturating_add(1);
                    high_ready.pop_front()
                } else {
                    None
                };
                let Some(key) = key else {
                    break;
                };
                ready_set.remove(&key);
                if active.contains(&key) {
                    continue;
                }
                let Some(queue) = queues.get_mut(&key) else {
                    continue;
                };
                let Some(envelope) = queue.pop_front() else {
                    queues.remove(&key);
                    continue;
                };
                self.metrics
                    .outbound_dequeued(envelope.submitted_at.elapsed());
                if envelope.is_abandoned() {
                    self.metrics.cancelled_command();
                    requeue_command_key(
                        key,
                        &mut queues,
                        &active,
                        &mut ready_set,
                        &mut high_ready,
                        &mut normal_ready,
                    );
                    continue;
                }
                if envelope.priority == CommandPriority::Normal {
                    running_normal = running_normal.saturating_add(1);
                }
                active.insert(key.clone());
                running.push(execute_command(
                    key,
                    envelope,
                    self.services.clone(),
                    self.global_capacity.clone(),
                    self.cooldown.clone(),
                    self.attempt_timeout,
                    self.max_retries,
                    self.retry_base,
                    self.retry_max,
                    Arc::clone(&self.metrics),
                ));
            }

            if input_closed && running.is_empty() && queues.is_empty() {
                break;
            }

            tokio::select! {
                received = self.receiver.recv(), if !input_closed => {
                    match received {
                        Some(envelope) => {
                            let key = envelope.key.clone();
                            let priority = envelope.priority;
                            let queue = queues.entry(key.clone()).or_default();
                            let was_empty = queue.is_empty();
                            queue.push_back(envelope);
                            if was_empty && !active.contains(&key) && ready_set.insert(key.clone()) {
                                push_ready(priority, key, &mut high_ready, &mut normal_ready);
                            }
                        }
                        None => input_closed = true,
                    }
                }
                completed = running.next(), if !running.is_empty() => {
                    if let Some(completed) = completed {
                        active.remove(&completed.key);
                        if completed.priority == CommandPriority::Normal {
                            running_normal = running_normal.saturating_sub(1);
                        }
                        requeue_command_key(
                            completed.key,
                            &mut queues,
                            &active,
                            &mut ready_set,
                            &mut high_ready,
                            &mut normal_ready,
                        );
                    }
                }
            }
        }
    }
}

fn requeue_command_key(
    key: CommandKey,
    queues: &mut HashMap<CommandKey, VecDeque<CommandEnvelope>>,
    active: &HashSet<CommandKey>,
    ready_set: &mut HashSet<CommandKey>,
    high_ready: &mut VecDeque<CommandKey>,
    normal_ready: &mut VecDeque<CommandKey>,
) {
    if let Some(priority) = queues
        .get(&key)
        .and_then(|queue| queue.front())
        .map(|envelope| envelope.priority)
    {
        if !active.contains(&key) && ready_set.insert(key.clone()) {
            push_ready(priority, key, high_ready, normal_ready);
        }
    } else {
        queues.remove(&key);
    }
}
