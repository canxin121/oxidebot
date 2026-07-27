use crate::{
    budget::{QueueAcquireError, QueueLease, QueueLimiter},
    CommandError, MetricsHandle, OverloadPolicy, PlatformError, PlatformErrorKind, QueueBudget,
};
use async_trait::async_trait;
use futures_util::{future::BoxFuture, stream::FuturesUnordered, FutureExt, StreamExt};
use oxidebot_core::{
    BotId, BotSlot, ConversationKey, MessageReceipt, MessageRef, MessageTarget, NativeData,
    OutgoingMessage, PlatformId,
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fmt,
    panic::AssertUnwindSafe,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

/// Immutable identity for one adapter connection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BotDescriptor {
    /// Platform identifier.
    pub platform: PlatformId,
    /// Bot identifier inside the platform.
    pub id: BotId,
    /// Optional human-readable name.
    pub display_name: Option<Arc<str>>,
}

impl BotDescriptor {
    /// Creates a bot descriptor.
    #[must_use]
    pub fn new(platform: PlatformId, id: BotId) -> Self {
        Self {
            platform,
            id,
            display_name: None,
        }
    }

    /// Adds a display name.
    #[must_use]
    pub fn display_name(mut self, display_name: impl Into<Arc<str>>) -> Self {
        self.display_name = Some(display_name.into());
        self
    }

    pub(crate) fn identity(&self) -> String {
        format!("{}:{}", self.platform, self.id)
    }
}

/// Coarse service capabilities available on one bot connection.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BotCapabilities {
    /// Portable message operations are available.
    pub messages: bool,
    /// Interaction responses are available.
    pub interactions: bool,
    /// Lossless platform-native calls are available.
    pub native_api: bool,
}

/// Portable message service implemented by an adapter.
#[async_trait]
pub trait MessageService: Send + Sync + 'static {
    /// Sends one portable message.
    async fn send(
        &self,
        target: &MessageTarget,
        message: &OutgoingMessage,
    ) -> std::result::Result<MessageReceipt, PlatformError>;

    /// Edits an existing message.
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

    /// Deletes an existing message.
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
    /// Answers one interaction.
    async fn respond(
        &self,
        interaction_id: &str,
        response: &NativeData,
    ) -> std::result::Result<(), PlatformError>;
}

/// Lossless platform-native API service implemented by an adapter.
#[async_trait]
pub trait NativeService: Send + Sync + 'static {
    /// Calls one platform-native method.
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
    /// Creates services with portable messaging support.
    #[must_use]
    pub fn messages(messages: Arc<dyn MessageService>) -> Self {
        Self {
            messages,
            interactions: None,
            native: None,
            capabilities: BotCapabilities {
                messages: true,
                interactions: false,
                native_api: false,
            },
        }
    }

    /// Adds interaction support.
    #[must_use]
    pub fn interactions(mut self, interactions: Arc<dyn InteractionService>) -> Self {
        self.interactions = Some(interactions);
        self.capabilities.interactions = true;
        self
    }

    /// Adds lossless native API support.
    #[must_use]
    pub fn native(mut self, native: Arc<dyn NativeService>) -> Self {
        self.native = Some(native);
        self.capabilities.native_api = true;
        self
    }

    /// Returns the immutable service capabilities.
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

    /// Returns the dense runtime bot slot.
    #[must_use]
    pub const fn slot(&self) -> BotSlot {
        self.slot
    }

    /// Returns immutable bot identity.
    #[must_use]
    pub fn descriptor(&self) -> &BotDescriptor {
        self.descriptor.as_ref()
    }

    /// Returns available services.
    #[must_use]
    pub const fn capabilities(&self) -> BotCapabilities {
        self.capabilities
    }

    /// Sends a message and waits for its platform receipt.
    pub async fn send(
        &self,
        target: MessageTarget,
        message: impl Into<OutgoingMessage>,
    ) -> std::result::Result<MessageReceipt, CommandError> {
        match self
            .client
            .call(CommandOperation::Send {
                target,
                message: message.into(),
            })
            .await?
        {
            CommandResult::Message(receipt) => Ok(receipt),
            CommandResult::Unit | CommandResult::Native(_) => Err(CommandError::UnexpectedResult),
        }
    }

    /// Edits a message and waits for completion.
    pub async fn edit(
        &self,
        message: MessageRef,
        replacement: impl Into<OutgoingMessage>,
    ) -> std::result::Result<(), CommandError> {
        match self
            .client
            .call(CommandOperation::Edit {
                message,
                replacement: replacement.into(),
            })
            .await?
        {
            CommandResult::Unit => Ok(()),
            CommandResult::Message(_) | CommandResult::Native(_) => {
                Err(CommandError::UnexpectedResult)
            }
        }
    }

    /// Deletes a message and waits for completion.
    pub async fn delete(&self, message: MessageRef) -> std::result::Result<(), CommandError> {
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

    /// Answers an interaction and waits for completion.
    pub async fn respond_interaction(
        &self,
        interaction_id: impl Into<Arc<str>>,
        response: NativeData,
    ) -> std::result::Result<(), CommandError> {
        match self
            .client
            .call(CommandOperation::RespondInteraction {
                interaction_id: interaction_id.into(),
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

    /// Calls a platform-native method and waits for its response.
    pub async fn call_native(
        &self,
        method: impl Into<Arc<str>>,
        request: NativeData,
    ) -> std::result::Result<NativeData, CommandError> {
        match self
            .client
            .call(CommandOperation::NativeCall {
                method: method.into(),
                request,
            })
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
        self.client
            .enqueue(CommandOperation::Send { target, message })
            .await
    }

    #[allow(dead_code)]
    pub(crate) async fn enqueue_edit(
        &self,
        message: MessageRef,
        replacement: OutgoingMessage,
    ) -> std::result::Result<(), CommandError> {
        self.client
            .enqueue(CommandOperation::Edit {
                message,
                replacement,
            })
            .await
    }

    #[allow(dead_code)]
    pub(crate) async fn enqueue_delete(
        &self,
        message: MessageRef,
    ) -> std::result::Result<(), CommandError> {
        self.client
            .enqueue(CommandOperation::Delete { message })
            .await
    }

    #[allow(dead_code)]
    pub(crate) async fn enqueue_interaction_response(
        &self,
        interaction_id: Arc<str>,
        response: NativeData,
    ) -> std::result::Result<(), CommandError> {
        self.client
            .enqueue(CommandOperation::RespondInteraction {
                interaction_id,
                response,
            })
            .await
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

    /// Returns a bot by dense slot.
    #[must_use]
    pub fn get(&self, slot: BotSlot) -> Option<&BotHandle> {
        usize::try_from(slot.0)
            .ok()
            .and_then(|index| self.0.get(index))
    }

    /// Iterates over registered bots.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &BotHandle> {
        self.0.iter()
    }

    /// Returns the number of registered bots.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns whether no bots are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[derive(Clone)]
struct CommandClient {
    sender: mpsc::Sender<CommandEnvelope>,
    limiter: QueueLimiter,
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
        let lease = match self.overload {
            OverloadPolicy::Block => self.limiter.acquire(operation.estimated_bytes()).await,
            OverloadPolicy::DropNewest => self.limiter.try_acquire(operation.estimated_bytes()),
        }
        .map_err(map_command_admission)?;
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
        let envelope = CommandEnvelope {
            key: operation.key(sequence),
            operation,
            reply,
            _lease: lease,
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
            Self::RespondInteraction { .. } | Self::NativeCall { .. } => {
                CommandKey::Independent(sequence)
            }
        }
    }

    fn estimated_bytes(&self) -> usize {
        match self {
            Self::Send { message, .. }
            | Self::Edit {
                replacement: message,
                ..
            } => message.estimated_bytes().saturating_add(256),
            Self::Delete { .. } => 128,
            Self::RespondInteraction { response, .. }
            | Self::NativeCall {
                request: response, ..
            } => response.estimated_bytes().saturating_add(256),
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

    fn idempotent(&self) -> bool {
        match self {
            Self::Send { message, .. } => message.options.idempotency_key.is_some(),
            Self::Delete { .. } => true,
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
    operation: CommandOperation,
    reply: Option<oneshot::Sender<std::result::Result<CommandResult, CommandError>>>,
    _lease: QueueLease,
}

struct CommandCompletion {
    key: CommandKey,
}

/// Fixed per-bot worker combining bounded admission, key ordering, and limited concurrency.
pub(crate) struct CommandWorker {
    receiver: mpsc::Receiver<CommandEnvelope>,
    services: BotServices,
    max_in_flight: usize,
    command_timeout: Option<Duration>,
    max_retries: u8,
    retry_base: Duration,
    metrics: MetricsHandle,
}

impl CommandWorker {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build(
        slot: BotSlot,
        descriptor: BotDescriptor,
        services: BotServices,
        budget: QueueBudget,
        overload: OverloadPolicy,
        max_in_flight: usize,
        command_timeout: Option<Duration>,
        max_retries: u8,
        retry_base: Duration,
        metrics: MetricsHandle,
    ) -> (BotHandle, Self) {
        let (sender, receiver) = mpsc::channel(budget.max_items);
        let client = CommandClient {
            sender,
            limiter: QueueLimiter::new(budget),
            overload,
            sequence: Arc::new(AtomicU64::new(0)),
        };
        let capabilities = services.capabilities();
        let bot = BotHandle::new(slot, descriptor, capabilities, client);
        (
            bot,
            Self {
                receiver,
                services,
                max_in_flight,
                command_timeout,
                max_retries,
                retry_base,
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
        let mut running: FuturesUnordered<BoxFuture<'static, CommandCompletion>> =
            FuturesUnordered::new();
        let mut input_closed = false;

        loop {
            while running.len() < self.max_in_flight {
                let key = high_ready.pop_front().or_else(|| normal_ready.pop_front());
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
                active.insert(key.clone());
                running.push(execute_command(
                    key,
                    envelope,
                    self.services.clone(),
                    self.command_timeout,
                    self.max_retries,
                    self.retry_base,
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

fn execute_command(
    key: CommandKey,
    envelope: CommandEnvelope,
    services: BotServices,
    command_timeout: Option<Duration>,
    max_retries: u8,
    retry_base: Duration,
    metrics: MetricsHandle,
) -> BoxFuture<'static, CommandCompletion> {
    Box::pin(async move {
        metrics.command();
        let result = AssertUnwindSafe(execute_with_retry(
            envelope.operation,
            services,
            command_timeout,
            max_retries,
            retry_base,
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
        drop(envelope._lease);
        CommandCompletion { key }
    })
}

async fn execute_with_retry(
    operation: CommandOperation,
    services: BotServices,
    command_timeout: Option<Duration>,
    max_retries: u8,
    retry_base: Duration,
) -> std::result::Result<CommandResult, CommandError> {
    let idempotent = operation.idempotent();
    let mut attempt = 0_u8;
    loop {
        let result = if let Some(timeout) = command_timeout {
            match tokio::time::timeout(timeout, execute_once(&operation, &services)).await {
                Ok(result) => result,
                Err(_) => Err(CommandError::Platform(PlatformError::new(
                    PlatformErrorKind::Timeout,
                    "platform command attempt timed out",
                ))),
            }
        } else {
            execute_once(&operation, &services).await
        };
        match result {
            Ok(value) => return Ok(value),
            Err(CommandError::Platform(error))
                if idempotent && error.is_retryable() && attempt < max_retries =>
            {
                let exponential = 1_u32.checked_shl(attempt.into()).unwrap_or(u32::MAX);
                let delay = error
                    .retry_after
                    .unwrap_or_else(|| retry_base.saturating_mul(exponential));
                tokio::time::sleep(delay).await;
                attempt = attempt.saturating_add(1);
            }
            Err(error) => return Err(error),
        }
    }
}

async fn execute_once(
    operation: &CommandOperation,
    services: &BotServices,
) -> std::result::Result<CommandResult, CommandError> {
    match operation {
        CommandOperation::Send { target, message } => services
            .messages
            .send(target, message)
            .await
            .map(CommandResult::Message)
            .map_err(CommandError::from),
        CommandOperation::Edit {
            message,
            replacement,
        } => services
            .messages
            .edit(message, replacement)
            .await
            .map(|()| CommandResult::Unit)
            .map_err(CommandError::from),
        CommandOperation::Delete { message } => services
            .messages
            .delete(message)
            .await
            .map(|()| CommandResult::Unit)
            .map_err(CommandError::from),
        CommandOperation::RespondInteraction {
            interaction_id,
            response,
        } => services
            .interactions
            .as_ref()
            .ok_or(CommandError::InteractionsUnsupported)?
            .respond(interaction_id, response)
            .await
            .map(|()| CommandResult::Unit)
            .map_err(CommandError::from),
        CommandOperation::NativeCall { method, request } => services
            .native
            .as_ref()
            .ok_or(CommandError::NativeUnsupported)?
            .call(method, request)
            .await
            .map(CommandResult::Native)
            .map_err(CommandError::from),
    }
}
