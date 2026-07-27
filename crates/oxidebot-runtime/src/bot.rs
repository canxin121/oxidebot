use crate::{
    budget::{HierarchicalLease, QueueAcquireError, QueueLimiter},
    CommandError, MetricsHandle, OverloadPolicy, PlatformError, PlatformErrorKind, QueueBudget,
};
use async_trait::async_trait;
use futures_util::{stream::FuturesUnordered, FutureExt, StreamExt};
use oxidebot_core::{
    BotId, BotIdentity, BotSlot, ConversationKey, InvalidId, MessageReceipt, MessageRef,
    MessageTarget, NativeData, OutgoingMessage, PlatformId, MAX_ROUTE_KEY_BYTES,
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fmt,
    hash::{Hash, Hasher},
    panic::AssertUnwindSafe,
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

/// Immutable identity for one adapter connection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BotDescriptor {
    pub platform: PlatformId,
    pub id: BotId,
    pub display_name: Option<Arc<str>>,
}

impl BotDescriptor {
    #[must_use]
    pub fn new(platform: PlatformId, id: BotId) -> Self {
        Self {
            platform,
            id,
            display_name: None,
        }
    }

    #[must_use]
    pub fn display_name(mut self, display_name: impl Into<Arc<str>>) -> Self {
        self.display_name = Some(display_name.into());
        self
    }

    #[must_use]
    pub fn identity(&self) -> BotIdentity {
        BotIdentity::new(self.platform.clone(), self.id.clone())
    }

    /// Revalidates adapter-owned identity before any runtime resources start.
    pub fn validate(&self) -> Result<(), InvalidId> {
        self.platform.validate()?;
        self.id.validate()
    }
}

/// Strength of idempotency provided by an adapter/platform.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum IdempotencyGuarantee {
    #[default]
    Unsupported,
    AdapterEmulated,
    PlatformNative,
}

/// Coarse service capabilities available on one bot connection.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BotCapabilities {
    pub messages: bool,
    pub interactions: bool,
    pub native_api: bool,
    pub send_idempotency: IdempotencyGuarantee,
    pub delete_idempotent: bool,
}

/// Portable message service implemented by an adapter.
#[async_trait]
pub trait MessageService: Send + Sync + 'static {
    async fn send(
        &self,
        target: &MessageTarget,
        message: &OutgoingMessage,
    ) -> std::result::Result<MessageReceipt, PlatformError>;

    async fn edit(
        &self,
        _message: &MessageRef,
        _replacement: &OutgoingMessage,
    ) -> std::result::Result<(), PlatformError> {
        Err(PlatformError::new(
            PlatformErrorKind::Unsupported,
            "editing messages is not supported by this adapter",
        ))
    }

    async fn delete(&self, _message: &MessageRef) -> std::result::Result<(), PlatformError> {
        Err(PlatformError::new(
            PlatformErrorKind::Unsupported,
            "deleting messages is not supported by this adapter",
        ))
    }
}

/// Interaction response service implemented by an adapter.
#[async_trait]
pub trait InteractionService: Send + Sync + 'static {
    async fn respond(
        &self,
        interaction_id: &str,
        response: &NativeData,
    ) -> std::result::Result<(), PlatformError>;
}

/// Lossless platform-native API service implemented by an adapter.
#[async_trait]
pub trait NativeService: Send + Sync + 'static {
    async fn call(
        &self,
        method: &str,
        request: &NativeData,
    ) -> std::result::Result<NativeData, PlatformError>;
}

/// Split outbound services exposed by one adapter.
#[derive(Clone)]
pub struct BotServices {
    pub(crate) messages: Arc<dyn MessageService>,
    pub(crate) interactions: Option<Arc<dyn InteractionService>>,
    pub(crate) native: Option<Arc<dyn NativeService>>,
    capabilities: BotCapabilities,
}

impl BotServices {
    #[must_use]
    pub fn messages(messages: Arc<dyn MessageService>) -> Self {
        Self {
            messages,
            interactions: None,
            native: None,
            capabilities: BotCapabilities {
                messages: true,
                ..BotCapabilities::default()
            },
        }
    }

    #[must_use]
    pub fn interactions(mut self, interactions: Arc<dyn InteractionService>) -> Self {
        self.interactions = Some(interactions);
        self.capabilities.interactions = true;
        self
    }

    #[must_use]
    pub fn native(mut self, native: Arc<dyn NativeService>) -> Self {
        self.native = Some(native);
        self.capabilities.native_api = true;
        self
    }

    /// Declares that message idempotency keys are actually enforced.
    #[must_use]
    pub fn send_idempotency(mut self, guarantee: IdempotencyGuarantee) -> Self {
        self.capabilities.send_idempotency = guarantee;
        self
    }

    /// Declares delete retry semantics (`NotFound` after a prior success is OK).
    #[must_use]
    pub fn idempotent_delete(mut self, enabled: bool) -> Self {
        self.capabilities.delete_idempotent = enabled;
        self
    }

    #[must_use]
    pub const fn capabilities(&self) -> BotCapabilities {
        self.capabilities
    }
}

impl fmt::Debug for BotServices {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BotServices")
            .field("capabilities", &self.capabilities)
            .finish_non_exhaustive()
    }
}

/// Clone-cheap runtime handle for one bot connection.
#[derive(Clone)]
pub struct BotHandle {
    slot: BotSlot,
    descriptor: Arc<BotDescriptor>,
    capabilities: BotCapabilities,
    client: CommandClient,
}

impl BotHandle {
    fn new(
        slot: BotSlot,
        descriptor: BotDescriptor,
        capabilities: BotCapabilities,
        client: CommandClient,
    ) -> Self {
        Self {
            slot,
            descriptor: Arc::new(descriptor),
            capabilities,
            client,
        }
    }

    #[must_use]
    pub const fn slot(&self) -> BotSlot {
        self.slot
    }

    #[must_use]
    pub fn descriptor(&self) -> &BotDescriptor {
        self.descriptor.as_ref()
    }

    #[must_use]
    pub const fn capabilities(&self) -> BotCapabilities {
        self.capabilities
    }

    pub async fn send(
        &self,
        target: MessageTarget,
        message: impl Into<OutgoingMessage>,
    ) -> std::result::Result<MessageReceipt, CommandError> {
        let message = message.into();
        target.validate_for(self.slot)?;
        message.validate_for(self.slot, &self.descriptor.platform)?;
        match self
            .client
            .call(CommandOperation::Send { target, message })
            .await?
        {
            CommandResult::Message(receipt) => Ok(receipt),
            CommandResult::Unit | CommandResult::Native(_) => Err(CommandError::UnexpectedResult),
        }
    }

    pub async fn edit(
        &self,
        message: MessageRef,
        replacement: impl Into<OutgoingMessage>,
    ) -> std::result::Result<(), CommandError> {
        let replacement = replacement.into();
        message.validate_for(self.slot)?;
        replacement.validate_for(self.slot, &self.descriptor.platform)?;
        match self
            .client
            .call(CommandOperation::Edit {
                message,
                replacement,
            })
            .await?
        {
            CommandResult::Unit => Ok(()),
            CommandResult::Message(_) | CommandResult::Native(_) => {
                Err(CommandError::UnexpectedResult)
            }
        }
    }

    pub async fn delete(&self, message: MessageRef) -> std::result::Result<(), CommandError> {
        message.validate_for(self.slot)?;
        match self
            .client
            .call(CommandOperation::Delete { message })
            .await?
        {
            CommandResult::Unit => Ok(()),
            CommandResult::Message(_) | CommandResult::Native(_) => {
                Err(CommandError::UnexpectedResult)
            }
        }
    }

    pub async fn respond_interaction(
        &self,
        interaction_id: impl Into<Arc<str>>,
        response: NativeData,
    ) -> std::result::Result<(), CommandError> {
        if !self.capabilities.interactions {
            return Err(CommandError::InteractionsUnsupported);
        }
        let interaction_id = interaction_id.into();
        validate_command_key(&interaction_id, "interaction id")?;
        response.validate_for(&self.descriptor.platform)?;
        match self
            .client
            .call(CommandOperation::RespondInteraction {
                interaction_id,
                response,
            })
            .await?
        {
            CommandResult::Unit => Ok(()),
            CommandResult::Message(_) | CommandResult::Native(_) => {
                Err(CommandError::UnexpectedResult)
            }
        }
    }

    pub async fn call_native(
        &self,
        method: impl Into<Arc<str>>,
        request: NativeData,
    ) -> std::result::Result<NativeData, CommandError> {
        if !self.capabilities.native_api {
            return Err(CommandError::NativeUnsupported);
        }
        let method = method.into();
        validate_command_key(&method, "native method")?;
        request.validate_for(&self.descriptor.platform)?;
        match self
            .client
            .call(CommandOperation::NativeCall { method, request })
            .await?
        {
            CommandResult::Native(value) => Ok(value),
            CommandResult::Message(_) | CommandResult::Unit => Err(CommandError::UnexpectedResult),
        }
    }

    pub(crate) async fn enqueue_send(
        &self,
        target: MessageTarget,
        message: OutgoingMessage,
    ) -> std::result::Result<(), CommandError> {
        target.validate_for(self.slot)?;
        message.validate_for(self.slot, &self.descriptor.platform)?;
        self.client
            .enqueue(CommandOperation::Send { target, message })
            .await
    }
}

fn validate_command_key(value: &str, label: &str) -> Result<(), CommandError> {
    if value.is_empty() || value.len() > MAX_ROUTE_KEY_BYTES {
        Err(CommandError::InvalidModel(format!(
            "{label} must contain 1..={MAX_ROUTE_KEY_BYTES} bytes"
        )))
    } else {
        Ok(())
    }
}

impl fmt::Debug for BotHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BotHandle")
            .field("slot", &self.slot)
            .field("descriptor", &self.descriptor)
            .field("capabilities", &self.capabilities)
            .finish_non_exhaustive()
    }
}

/// Immutable bot directory indexed by dense bot slots.
#[derive(Clone, Debug)]
pub struct BotDirectory(Arc<[BotHandle]>);

impl BotDirectory {
    pub(crate) fn new(bots: Vec<BotHandle>) -> Self {
        Self(bots.into())
    }

    #[must_use]
    pub fn get(&self, slot: BotSlot) -> Option<&BotHandle> {
        self.0.get(slot.0 as usize)
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &BotHandle> {
        self.0.iter()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[derive(Clone)]
struct CommandClient {
    sender: mpsc::Sender<CommandEnvelope>,
    local_limiter: QueueLimiter,
    global_limiter: QueueLimiter,
    overload: OverloadPolicy,
    sequence: Arc<AtomicU64>,
}

impl CommandClient {
    async fn call(
        &self,
        operation: CommandOperation,
    ) -> std::result::Result<CommandResult, CommandError> {
        let (sender, receiver) = oneshot::channel();
        self.submit(operation, Some(sender)).await?;
        receiver.await.map_err(|_| CommandError::Closed)?
    }

    async fn enqueue(&self, operation: CommandOperation) -> std::result::Result<(), CommandError> {
        self.submit(operation, None).await
    }

    async fn submit(
        &self,
        operation: CommandOperation,
        reply: Option<oneshot::Sender<std::result::Result<CommandResult, CommandError>>>,
    ) -> std::result::Result<(), CommandError> {
        let bytes = operation.retained_bytes();
        // Local first: a noisy bot cannot reserve all global permits while
        // waiting for its own queue capacity.
        let local = match self.overload {
            OverloadPolicy::Block => self.local_limiter.acquire(bytes).await,
            OverloadPolicy::DropNewest => self.local_limiter.try_acquire(bytes),
        }
        .map_err(map_command_admission)?;
        let global = match self.overload {
            OverloadPolicy::Block => self.global_limiter.acquire(bytes).await,
            OverloadPolicy::DropNewest => self.global_limiter.try_acquire(bytes),
        }
        .map_err(map_command_admission)?;
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
        let envelope = CommandEnvelope {
            key: operation.key(sequence),
            sequence,
            operation,
            reply,
            _leases: HierarchicalLease::new(local, global),
        };
        match self.overload {
            OverloadPolicy::Block => self
                .sender
                .send(envelope)
                .await
                .map_err(|_| CommandError::Closed),
            OverloadPolicy::DropNewest => {
                self.sender.try_send(envelope).map_err(|error| match error {
                    mpsc::error::TrySendError::Full(_) => CommandError::Full,
                    mpsc::error::TrySendError::Closed(_) => CommandError::Closed,
                })
            }
        }
    }
}

fn map_command_admission(error: QueueAcquireError) -> CommandError {
    match error {
        QueueAcquireError::Closed => CommandError::Closed,
        QueueAcquireError::Full => CommandError::Full,
        QueueAcquireError::TooLarge => CommandError::PayloadTooLarge,
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum CommandKey {
    Conversation(ConversationKey),
    Interaction(Arc<str>),
    Independent(u64),
}

enum CommandOperation {
    Send {
        target: MessageTarget,
        message: OutgoingMessage,
    },
    Edit {
        message: MessageRef,
        replacement: OutgoingMessage,
    },
    Delete {
        message: MessageRef,
    },
    RespondInteraction {
        interaction_id: Arc<str>,
        response: NativeData,
    },
    NativeCall {
        method: Arc<str>,
        request: NativeData,
    },
}

impl CommandOperation {
    fn key(&self, sequence: u64) -> CommandKey {
        match self {
            Self::Send { target, .. } => CommandKey::Conversation(target.conversation.clone()),
            Self::Edit { message, .. } | Self::Delete { message } => {
                CommandKey::Conversation(message.conversation.clone())
            }
            Self::RespondInteraction { interaction_id, .. } => {
                CommandKey::Interaction(interaction_id.clone())
            }
            Self::NativeCall { .. } => CommandKey::Independent(sequence),
        }
    }

    fn retained_bytes(&self) -> usize {
        match self {
            Self::Send { target, message } => target
                .estimated_bytes()
                .saturating_add(message.estimated_bytes())
                .saturating_add(256),
            Self::Edit {
                message,
                replacement,
            } => message
                .estimated_bytes()
                .saturating_add(replacement.estimated_bytes())
                .saturating_add(256),
            Self::Delete { message } => message.estimated_bytes().saturating_add(128),
            Self::RespondInteraction {
                interaction_id,
                response,
            } => interaction_id
                .len()
                .saturating_add(response.estimated_bytes())
                .saturating_add(256),
            Self::NativeCall { method, request } => method
                .len()
                .saturating_add(request.estimated_bytes())
                .saturating_add(256),
        }
    }

    fn priority(&self) -> CommandPriority {
        match self {
            Self::RespondInteraction { .. } => CommandPriority::High,
            Self::Send { .. }
            | Self::Edit { .. }
            | Self::Delete { .. }
            | Self::NativeCall { .. } => CommandPriority::Normal,
        }
    }

    fn idempotent(&self, capabilities: BotCapabilities) -> bool {
        match self {
            Self::Send { message, .. } => {
                message.options.idempotency_key.is_some()
                    && capabilities.send_idempotency != IdempotencyGuarantee::Unsupported
            }
            Self::Delete { .. } => capabilities.delete_idempotent,
            Self::Edit { .. } | Self::RespondInteraction { .. } | Self::NativeCall { .. } => false,
        }
    }
}

#[derive(Clone, Copy)]
enum CommandPriority {
    High,
    Normal,
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

enum CommandResult {
    Message(MessageReceipt),
    Unit,
    Native(NativeData),
}

struct CommandEnvelope {
    key: CommandKey,
    sequence: u64,
    operation: CommandOperation,
    reply: Option<oneshot::Sender<std::result::Result<CommandResult, CommandError>>>,
    _leases: HierarchicalLease,
}

impl CommandEnvelope {
    fn is_abandoned(&self) -> bool {
        self.reply.as_ref().is_some_and(|reply| reply.is_closed())
    }
}

struct CommandCompletion {
    key: CommandKey,
}

/// Fixed per-bot worker combining bounded admission, key ordering, weighted
/// priority, panic isolation, and limited concurrency.
pub(crate) struct CommandWorker {
    receiver: mpsc::Receiver<CommandEnvelope>,
    slot: BotSlot,
    platform: PlatformId,
    services: BotServices,
    max_in_flight: usize,
    global_in_flight: Arc<Semaphore>,
    high_priority_burst: usize,
    attempt_timeout: Option<Duration>,
    total_timeout: Option<Duration>,
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
        global_limiter: QueueLimiter,
        global_in_flight: Arc<Semaphore>,
        local_budget: QueueBudget,
        overload: OverloadPolicy,
        max_in_flight: usize,
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
            local_limiter: QueueLimiter::new(local_budget),
            global_limiter,
            overload,
            sequence: Arc::new(AtomicU64::new(0)),
        };
        let capabilities = services.capabilities();
        let platform = descriptor.platform.clone();
        let bot = BotHandle::new(slot, descriptor, capabilities, client);
        (
            bot,
            Self {
                receiver,
                slot,
                platform,
                services,
                max_in_flight,
                global_in_flight,
                high_priority_burst,
                attempt_timeout,
                total_timeout,
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

        loop {
            while running.len() < self.max_in_flight {
                let key = if !high_ready.is_empty()
                    && (normal_ready.is_empty() || consecutive_high < self.high_priority_burst)
                {
                    consecutive_high = consecutive_high.saturating_add(1);
                    high_ready.pop_front()
                } else {
                    consecutive_high = 0;
                    normal_ready.pop_front().or_else(|| high_ready.pop_front())
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
                if envelope.is_abandoned() {
                    self.metrics.cancelled_command();
                    if let Some(priority) = queues
                        .get(&key)
                        .and_then(|queue| queue.front())
                        .map(|next| next.operation.priority())
                    {
                        if ready_set.insert(key.clone()) {
                            push_ready(priority, key, &mut high_ready, &mut normal_ready);
                        }
                    } else {
                        queues.remove(&key);
                    }
                    continue;
                }
                active.insert(key.clone());
                running.push(execute_command(
                    key,
                    envelope,
                    self.slot,
                    self.platform.clone(),
                    self.services.clone(),
                    Arc::clone(&self.global_in_flight),
                    self.attempt_timeout,
                    self.total_timeout,
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
                            let priority = envelope.operation.priority();
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
                        if let Some(priority) = queues
                            .get(&completed.key)
                            .and_then(|queue| queue.front())
                            .map(|envelope| envelope.operation.priority())
                        {
                            if ready_set.insert(completed.key.clone()) {
                                push_ready(
                                    priority,
                                    completed.key,
                                    &mut high_ready,
                                    &mut normal_ready,
                                );
                            }
                        } else {
                            queues.remove(&completed.key);
                        }
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn execute_command(
    key: CommandKey,
    envelope: CommandEnvelope,
    slot: BotSlot,
    platform: PlatformId,
    services: BotServices,
    global_in_flight: Arc<Semaphore>,
    attempt_timeout: Option<Duration>,
    total_timeout: Option<Duration>,
    max_retries: u8,
    retry_base: Duration,
    retry_max: Duration,
    metrics: MetricsHandle,
) -> CommandCompletion {
    if envelope.is_abandoned() {
        metrics.cancelled_command();
        return CommandCompletion { key };
    }
    let _global_permit: OwnedSemaphorePermit = global_in_flight
        .acquire_owned()
        .await
        .expect("global command semaphore is never closed");
    if envelope.is_abandoned() {
        metrics.cancelled_command();
        return CommandCompletion { key };
    }
    metrics.command();
    let result = AssertUnwindSafe(execute_with_retry(
        &envelope.operation,
        envelope.sequence,
        slot,
        &platform,
        &services,
        attempt_timeout,
        total_timeout,
        max_retries,
        retry_base,
        retry_max,
    ))
    .catch_unwind()
    .await
    .unwrap_or(Err(CommandError::ServicePanicked));
    if result.is_err() {
        metrics.command_error();
    }
    if let Some(reply) = envelope.reply {
        let _ = reply.send(result);
    } else if let Err(error) = result {
        tracing::warn!(%error, "deferred bot command failed");
    }
    drop(envelope._leases);
    CommandCompletion { key }
}

#[allow(clippy::too_many_arguments)]
async fn execute_with_retry(
    operation: &CommandOperation,
    jitter_seed: u64,
    slot: BotSlot,
    platform: &PlatformId,
    services: &BotServices,
    attempt_timeout: Option<Duration>,
    total_timeout: Option<Duration>,
    max_retries: u8,
    retry_base: Duration,
    retry_max: Duration,
) -> std::result::Result<CommandResult, CommandError> {
    let idempotent = operation.idempotent(services.capabilities());
    let deadline = match total_timeout {
        Some(timeout) => Some(Instant::now().checked_add(timeout).ok_or_else(|| {
            CommandError::InvalidModel(
                "command total timeout exceeds the platform clock range".into(),
            )
        })?),
        None => None,
    };
    let mut attempt = 0_u8;
    loop {
        let remaining = deadline.map(|deadline| deadline.saturating_duration_since(Instant::now()));
        if remaining.is_some_and(|remaining| remaining.is_zero()) {
            return Err(CommandError::Platform(PlatformError::new(
                PlatformErrorKind::Timeout,
                "platform command exceeded its total deadline",
            )));
        }
        let timeout = match (attempt_timeout, remaining) {
            (Some(attempt), Some(remaining)) => Some(attempt.min(remaining)),
            (Some(attempt), None) => Some(attempt),
            (None, Some(remaining)) => Some(remaining),
            (None, None) => None,
        };
        let result = if let Some(timeout) = timeout {
            match tokio::time::timeout(timeout, execute_once(operation, slot, platform, services))
                .await
            {
                Ok(result) => result,
                Err(_) => Err(CommandError::Platform(PlatformError::new(
                    PlatformErrorKind::Timeout,
                    "platform command attempt timed out",
                ))),
            }
        } else {
            execute_once(operation, slot, platform, services).await
        };
        match result {
            Ok(value) => return Ok(value),
            Err(CommandError::Platform(error))
                if idempotent && error.is_retryable() && attempt < max_retries =>
            {
                let exponential = 1_u32.checked_shl(attempt.into()).unwrap_or(u32::MAX);
                let delay = if let Some(retry_after) = error.retry_after {
                    // A server-provided Retry-After is a lower bound. Never cap it
                    // to the locally generated backoff ceiling and retry too early.
                    retry_after
                } else {
                    let base = retry_base.saturating_mul(exponential).min(retry_max);
                    let jitter = deterministic_jitter(base, jitter_seed, attempt);
                    base.saturating_add(jitter).min(retry_max)
                };
                if deadline.is_some_and(|deadline| {
                    Instant::now()
                        .checked_add(delay)
                        .is_none_or(|next_attempt| next_attempt >= deadline)
                }) {
                    return Err(CommandError::Platform(error));
                }
                tokio::time::sleep(delay).await;
                attempt = attempt.saturating_add(1);
            }
            Err(error) => return Err(error),
        }
    }
}

fn deterministic_jitter(base: Duration, seed: u64, attempt: u8) -> Duration {
    if base.is_zero() {
        return Duration::ZERO;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    seed.hash(&mut hasher);
    attempt.hash(&mut hasher);
    let fraction = hasher.finish() % 1_000;
    let max_nanos = base.as_nanos() / 4;
    let nanos = max_nanos.saturating_mul(u128::from(fraction)) / 1_000;
    Duration::from_nanos(u64::try_from(nanos.min(u128::from(u64::MAX))).unwrap_or(u64::MAX))
}

async fn execute_once(
    operation: &CommandOperation,
    slot: BotSlot,
    platform: &PlatformId,
    services: &BotServices,
) -> std::result::Result<CommandResult, CommandError> {
    match operation {
        CommandOperation::Send { target, message } => {
            let receipt = services.messages.send(target, message).await?;
            receipt.reference.validate_for(slot)?;
            if receipt.reference.conversation != target.conversation {
                return Err(CommandError::InvalidModel(
                    "adapter returned a receipt for a different conversation".into(),
                ));
            }
            Ok(CommandResult::Message(receipt))
        }
        CommandOperation::Edit {
            message,
            replacement,
        } => services
            .messages
            .edit(message, replacement)
            .await
            .map(|()| CommandResult::Unit)
            .map_err(CommandError::from),
        CommandOperation::Delete { message } => match services.messages.delete(message).await {
            Ok(()) => Ok(CommandResult::Unit),
            Err(error)
                if services.capabilities().delete_idempotent
                    && error.kind == PlatformErrorKind::NotFound =>
            {
                Ok(CommandResult::Unit)
            }
            Err(error) => Err(CommandError::Platform(error)),
        },
        CommandOperation::RespondInteraction {
            interaction_id,
            response,
        } => {
            response.validate_for(platform)?;
            services
                .interactions
                .as_ref()
                .ok_or(CommandError::InteractionsUnsupported)?
                .respond(interaction_id, response)
                .await
                .map(|()| CommandResult::Unit)
                .map_err(CommandError::from)
        }
        CommandOperation::NativeCall { method, request } => {
            request.validate_for(platform)?;
            let response = services
                .native
                .as_ref()
                .ok_or(CommandError::NativeUnsupported)?
                .call(method, request)
                .await?;
            response.validate_for(platform)?;
            Ok(CommandResult::Native(response))
        }
    }
}
