use crate::{
    budget::{HierarchicalLease, PriorityQueueLimiter, QueueAcquireError},
    CommandError, MetricsHandle, OverloadPolicy, PlatformError, PlatformErrorKind, QueueBudget,
    RuntimeMetrics,
};
use futures_util::{stream::FuturesUnordered, FutureExt, StreamExt};
use oxidebot_core::{
    application::CommandDefinition,
    capability::BotCapabilities,
    collaboration::{Reaction, ReactionOptions},
    conversation::{
        ConversationKind, ConversationRef as PublicConversationRef, MessageRef as PublicMessageRef,
        MessageTarget as PublicMessageTarget,
    },
    interaction::{InteractionResponse, InteractionResponseHandle, InteractionVisibility},
    source::message::{
        DeliveryPlan as PublicDeliveryPlan, DeliveryReport as PublicDeliveryReport,
        Message as PublicMessage,
    },
    BotId, BotIdentity, BotObject, BotSlot, CallApiTrait, FallbackPolicy, InvalidId, PlatformId,
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fmt,
    hash::{Hash, Hasher},
    panic::{catch_unwind, AssertUnwindSafe},
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
pub struct RuntimeCapabilities {
    pub messages: bool,
    pub interactions: bool,
    pub native_api: bool,
    pub api: bool,
    pub send_idempotency: IdempotencyGuarantee,
    pub delete_idempotent: bool,
}

fn platform_error(error: anyhow::Error) -> PlatformError {
    match error.downcast::<PlatformError>() {
        Ok(error) => error,
        Err(error) => PlatformError::new(PlatformErrorKind::Permanent, error.to_string()),
    }
}

fn command_api_error(error: anyhow::Error) -> CommandError {
    match error.downcast::<oxidebot_core::PartialDeliveryError>() {
        Ok(error) => CommandError::PartialDelivery(error),
        Err(error) => CommandError::Platform(platform_error(error)),
    }
}

/// Runtime services derived from one OxideBot API implementation.
#[derive(Clone)]
pub struct BotServices {
    pub(crate) api: Option<Arc<dyn CallApiTrait>>,
    capabilities: RuntimeCapabilities,
}

impl BotServices {
    /// Creates adapter services from the single OxideBot API object.
    #[must_use]
    pub fn new(api: Arc<dyn CallApiTrait>) -> Self {
        Self {
            api: Some(api),
            capabilities: RuntimeCapabilities {
                messages: true,
                api: true,
                ..RuntimeCapabilities::default()
            },
        }
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
    pub fn capabilities(&self) -> RuntimeCapabilities {
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

struct BotHandleInner {
    slot: BotSlot,
    identity: BotIdentity,
    descriptor: BotDescriptor,
    capabilities: RuntimeCapabilities,
    api: Option<Arc<dyn CallApiTrait>>,
    client: CommandClient,
}

/// Clone-cheap runtime handle for one bot connection. Cloning a context or
/// dispatch job increments one `Arc` rather than cloning every scheduler handle.
#[derive(Clone)]
pub struct BotHandle(Arc<BotHandleInner>);

impl BotHandle {
    fn new(
        slot: BotSlot,
        descriptor: BotDescriptor,
        capabilities: RuntimeCapabilities,
        api: Option<Arc<dyn CallApiTrait>>,
        client: CommandClient,
    ) -> Self {
        let identity = descriptor.identity();
        Self(Arc::new(BotHandleInner {
            slot,
            identity,
            descriptor,
            capabilities,
            api,
            client,
        }))
    }

    #[must_use]
    pub fn slot(&self) -> BotSlot {
        self.0.slot
    }

    #[must_use]
    pub fn identity(&self) -> &BotIdentity {
        &self.0.identity
    }

    #[must_use]
    pub fn descriptor(&self) -> &BotDescriptor {
        &self.0.descriptor
    }

    #[must_use]
    pub fn capabilities(&self) -> RuntimeCapabilities {
        self.0.capabilities
    }

    /// Returns the adapter's complete OxideBot API surface.
    pub(crate) fn api(&self) -> std::result::Result<BotObject, CommandError> {
        self.0
            .api
            .as_ref()
            .map(Arc::clone)
            .ok_or(CommandError::ApiUnsupported)
    }

    /// Reads the adapter's public capability model behind a panic boundary.
    pub(crate) fn bot_capabilities(&self) -> std::result::Result<BotCapabilities, CommandError> {
        let api = self.api()?;
        catch_unwind(AssertUnwindSafe(|| api.bot_capabilities()))
            .map_err(|_| CommandError::ServicePanicked)
    }

    /// Plans one public message without invoking transport. Planning is kept
    /// outside the queue so delivery middleware can inspect and transform the
    /// exact physical plan before the bounded transport operation is admitted.
    pub(crate) fn plan_outgoing_message(
        &self,
        message: &PublicMessage,
        fallback: FallbackPolicy,
    ) -> std::result::Result<PublicDeliveryPlan, CommandError> {
        let api = self.api()?;
        catch_unwind(AssertUnwindSafe(|| {
            api.plan_outgoing_message(message, fallback)
        }))
        .map_err(|_| CommandError::ServicePanicked)?
        .map_err(|error| CommandError::Platform(platform_error(error)))
    }

    /// Executes an already planned public delivery through the bounded
    /// per-bot command scheduler.
    pub(crate) async fn send_delivery_plan(
        &self,
        target: PublicMessageTarget,
        plan: PublicDeliveryPlan,
    ) -> std::result::Result<PublicDeliveryReport, CommandError> {
        match self
            .0
            .client
            .call(CommandOperation::Deliver { target, plan })
            .await?
        {
            CommandResult::Delivery(report) => Ok(report),
            _ => Err(CommandError::UnexpectedResult),
        }
    }

    pub(crate) async fn send_outgoing_message_with(
        &self,
        target: PublicMessageTarget,
        message: PublicMessage,
        fallback: FallbackPolicy,
    ) -> std::result::Result<PublicDeliveryReport, CommandError> {
        let plan = self.plan_outgoing_message(&message, fallback)?;
        self.send_delivery_plan(target, plan).await
    }

    pub(crate) async fn edit_outgoing_message(
        &self,
        message: PublicMessageRef,
        replacement: PublicMessage,
    ) -> std::result::Result<(), CommandError> {
        match self
            .0
            .client
            .call(CommandOperation::EditPublic {
                message,
                replacement,
            })
            .await?
        {
            CommandResult::Unit => Ok(()),
            _ => Err(CommandError::UnexpectedResult),
        }
    }

    pub(crate) async fn delete_message_ref(
        &self,
        message: PublicMessageRef,
    ) -> std::result::Result<(), CommandError> {
        match self
            .0
            .client
            .call(CommandOperation::DeletePublic { message })
            .await?
        {
            CommandResult::Unit => Ok(()),
            _ => Err(CommandError::UnexpectedResult),
        }
    }

    pub(crate) async fn add_message_reaction(
        &self,
        message: PublicMessageRef,
        reaction: Reaction,
        options: ReactionOptions,
    ) -> std::result::Result<(), CommandError> {
        match self
            .0
            .client
            .call(CommandOperation::ReactPublic {
                message,
                reaction,
                options,
            })
            .await?
        {
            CommandResult::Unit => Ok(()),
            _ => Err(CommandError::UnexpectedResult),
        }
    }

    pub(crate) async fn answer_interaction(
        &self,
        handle: InteractionResponseHandle,
        response: InteractionResponse,
    ) -> std::result::Result<(), CommandError> {
        match self
            .0
            .client
            .call(CommandOperation::AnswerInteraction { handle, response })
            .await?
        {
            CommandResult::Unit => Ok(()),
            _ => Err(CommandError::UnexpectedResult),
        }
    }

    pub(crate) async fn defer_interaction(
        &self,
        handle: InteractionResponseHandle,
        visibility: InteractionVisibility,
    ) -> std::result::Result<(), CommandError> {
        match self
            .0
            .client
            .call(CommandOperation::DeferInteraction { handle, visibility })
            .await?
        {
            CommandResult::Unit => Ok(()),
            _ => Err(CommandError::UnexpectedResult),
        }
    }

    pub(crate) async fn send_interaction_followup(
        &self,
        handle: InteractionResponseHandle,
        message: PublicMessage,
    ) -> std::result::Result<Vec<PublicMessageRef>, CommandError> {
        match self
            .0
            .client
            .call(CommandOperation::InteractionFollowup { handle, message })
            .await?
        {
            CommandResult::Messages(messages) => Ok(messages),
            _ => Err(CommandError::UnexpectedResult),
        }
    }

    pub(crate) async fn edit_interaction_response(
        &self,
        handle: InteractionResponseHandle,
        message: PublicMessage,
    ) -> std::result::Result<(), CommandError> {
        match self
            .0
            .client
            .call(CommandOperation::EditInteraction { handle, message })
            .await?
        {
            CommandResult::Unit => Ok(()),
            _ => Err(CommandError::UnexpectedResult),
        }
    }

    pub(crate) async fn set_command_definitions(
        &self,
        definitions: Vec<CommandDefinition>,
    ) -> std::result::Result<(), CommandError> {
        match self
            .0
            .client
            .call(CommandOperation::PublishCommands { definitions })
            .await?
        {
            CommandResult::Unit => Ok(()),
            _ => Err(CommandError::UnexpectedResult),
        }
    }
}

impl fmt::Debug for BotHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BotHandle")
            .field("slot", &self.0.slot)
            .field("descriptor", &self.0.descriptor)
            .field("capabilities", &self.0.capabilities)
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
    local_limiter: PriorityQueueLimiter,
    global_limiter: PriorityQueueLimiter,
    overload: OverloadPolicy,
    max_command_bytes: usize,
    total_timeout: Option<Duration>,
    sequence: Arc<AtomicU64>,
    metrics: MetricsHandle,
}

impl CommandClient {
    async fn call(
        &self,
        operation: CommandOperation,
    ) -> std::result::Result<CommandResult, CommandError> {
        let (sender, receiver) = oneshot::channel();
        let deadline = self.submit(operation, Some(sender)).await?;
        await_before(deadline, receiver)
            .await?
            .map_err(|_| CommandError::Closed)?
    }

    async fn submit(
        &self,
        operation: CommandOperation,
        reply: Option<CommandReply>,
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
        let sequence = self
            .sequence
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .map_err(|_| CommandError::SequenceExhausted)?;
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

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum CommandKey {
    PublicConversation(Arc<str>),
    Interaction(Arc<str>),
    Independent(u64),
}

enum CommandOperation {
    Deliver {
        target: PublicMessageTarget,
        plan: PublicDeliveryPlan,
    },
    EditPublic {
        message: PublicMessageRef,
        replacement: PublicMessage,
    },
    DeletePublic {
        message: PublicMessageRef,
    },
    ReactPublic {
        message: PublicMessageRef,
        reaction: Reaction,
        options: ReactionOptions,
    },
    AnswerInteraction {
        handle: InteractionResponseHandle,
        response: InteractionResponse,
    },
    DeferInteraction {
        handle: InteractionResponseHandle,
        visibility: InteractionVisibility,
    },
    InteractionFollowup {
        handle: InteractionResponseHandle,
        message: PublicMessage,
    },
    EditInteraction {
        handle: InteractionResponseHandle,
        message: PublicMessage,
    },
    PublishCommands {
        definitions: Vec<CommandDefinition>,
    },
}

impl CommandOperation {
    fn key(&self, sequence: u64) -> CommandKey {
        match self {
            Self::Deliver { target, .. } => {
                CommandKey::PublicConversation(public_target_key(target))
            }
            Self::EditPublic { message, .. }
            | Self::DeletePublic { message }
            | Self::ReactPublic { message, .. } => message
                .conversation
                .as_ref()
                .map(public_conversation_key)
                .map(CommandKey::PublicConversation)
                .unwrap_or(CommandKey::Independent(sequence)),
            Self::AnswerInteraction { handle, .. }
            | Self::DeferInteraction { handle, .. }
            | Self::InteractionFollowup { handle, .. }
            | Self::EditInteraction { handle, .. } => {
                CommandKey::Interaction(Arc::from(handle.id.as_str()))
            }
            Self::PublishCommands { .. } => CommandKey::Independent(sequence),
        }
    }

    fn retained_bytes(&self) -> usize {
        match self {
            Self::Deliver { target, plan } => public_target_bytes(target)
                .saturating_add(
                    plan.messages
                        .iter()
                        .map(PublicMessage::estimated_bytes)
                        .sum::<usize>(),
                )
                .saturating_add(
                    plan.degradations
                        .iter()
                        .map(|item| {
                            item.path
                                .len()
                                .saturating_add(item.feature.len())
                                .saturating_add(item.detail.len())
                                .saturating_add(64)
                        })
                        .sum::<usize>(),
                )
                .saturating_add(256),
            Self::EditPublic {
                message,
                replacement,
            } => public_message_ref_bytes(message)
                .saturating_add(replacement.estimated_bytes())
                .saturating_add(256),
            Self::DeletePublic { message } => public_message_ref_bytes(message).saturating_add(128),
            Self::ReactPublic {
                message,
                reaction,
                options,
            } => public_message_ref_bytes(message)
                .saturating_add(serialized_size(reaction))
                .saturating_add(serialized_size(options))
                .saturating_add(128),
            Self::AnswerInteraction { handle, response } => interaction_handle_bytes(handle)
                .saturating_add(serialized_size(response))
                .saturating_add(256),
            Self::DeferInteraction { handle, .. } => {
                interaction_handle_bytes(handle).saturating_add(128)
            }
            Self::InteractionFollowup { handle, message }
            | Self::EditInteraction { handle, message } => interaction_handle_bytes(handle)
                .saturating_add(message.estimated_bytes())
                .saturating_add(256),
            Self::PublishCommands { definitions } => serialized_size(definitions)
                .saturating_add(definitions.capacity() * std::mem::size_of::<CommandDefinition>())
                .saturating_add(256),
        }
    }

    fn priority(&self) -> CommandPriority {
        match self {
            Self::AnswerInteraction { .. }
            | Self::DeferInteraction { .. }
            | Self::InteractionFollowup { .. }
            | Self::EditInteraction { .. } => CommandPriority::High,
            Self::Deliver { .. }
            | Self::EditPublic { .. }
            | Self::DeletePublic { .. }
            | Self::ReactPublic { .. }
            | Self::PublishCommands { .. } => CommandPriority::Normal,
        }
    }

    fn idempotent(&self, capabilities: RuntimeCapabilities) -> bool {
        match self {
            Self::Deliver { plan, .. } => {
                capabilities.send_idempotency != IdempotencyGuarantee::Unsupported
                    && !plan.messages.is_empty()
                    && plan
                        .messages
                        .iter()
                        .all(|message| message.options.idempotency_key.is_some())
            }
            Self::DeletePublic { .. } => capabilities.delete_idempotent,
            // Publishing replaces the complete definition set and is safe to repeat.
            Self::PublishCommands { .. } => true,
            Self::EditPublic { .. }
            | Self::ReactPublic { .. }
            | Self::AnswerInteraction { .. }
            | Self::DeferInteraction { .. }
            | Self::InteractionFollowup { .. }
            | Self::EditInteraction { .. } => false,
        }
    }
}

fn public_conversation_key(conversation: &PublicConversationRef) -> Arc<str> {
    let mut key = String::new();
    append_public_conversation_key(conversation, &mut key);
    Arc::from(key)
}

fn public_target_key(target: &PublicMessageTarget) -> Arc<str> {
    public_conversation_key(&target.conversation)
}

fn append_public_conversation_key(conversation: &PublicConversationRef, output: &mut String) {
    if let Some(parent) = &conversation.parent {
        append_public_conversation_key(parent, output);
        output.push('/');
    }
    output.push_str(match conversation.kind {
        ConversationKind::Direct => "direct:",
        ConversationKind::Group => "group:",
        ConversationKind::Channel => "channel:",
        ConversationKind::Thread => "thread:",
        ConversationKind::Topic => "topic:",
        ConversationKind::Forum => "forum:",
        ConversationKind::Unknown => "unknown:",
        ConversationKind::PlatformNative(_) => "native:",
    });
    output.push_str(&conversation.id);
}

fn public_conversation_bytes(conversation: &PublicConversationRef) -> usize {
    conversation
        .id
        .len()
        .saturating_add(
            conversation
                .parent
                .as_deref()
                .map_or(0, public_conversation_bytes),
        )
        .saturating_add(serialized_size(&conversation.platform_data))
        .saturating_add(64)
}

fn public_target_bytes(target: &PublicMessageTarget) -> usize {
    public_conversation_bytes(&target.conversation)
        .saturating_add(target.recipients.iter().map(String::len).sum::<usize>())
        .saturating_add(target.recipients.capacity() * std::mem::size_of::<String>())
        .saturating_add(serialized_size(&target.platform_data))
        .saturating_add(64)
}

fn public_message_ref_bytes(message: &PublicMessageRef) -> usize {
    message
        .id
        .len()
        .saturating_add(
            message
                .conversation
                .as_ref()
                .map_or(0, public_conversation_bytes),
        )
        .saturating_add(serialized_size(&message.platform_data))
        .saturating_add(64)
}

fn interaction_handle_bytes(handle: &InteractionResponseHandle) -> usize {
    handle
        .id
        .len()
        .saturating_add(serialized_size(&handle.platform_data))
        .saturating_add(128)
}

fn serialized_size(value: &impl serde::Serialize) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
    Messages(Vec<PublicMessageRef>),
    Delivery(PublicDeliveryReport),
    Unit,
}

type CommandReply = oneshot::Sender<std::result::Result<CommandResult, CommandError>>;

struct CommandEnvelope {
    key: CommandKey,
    sequence: u64,
    deadline: Option<Instant>,
    priority: CommandPriority,
    operation: CommandOperation,
    reply: Option<CommandReply>,
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
        let api = services.api.clone();
        let bot = BotHandle::new(slot, descriptor, capabilities, api, client);
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

#[allow(clippy::too_many_arguments)]
async fn execute_command(
    key: CommandKey,
    envelope: CommandEnvelope,
    services: BotServices,
    global_capacity: GlobalCommandCapacity,
    cooldown: BotCooldown,
    attempt_timeout: Option<Duration>,
    max_retries: u8,
    retry_base: Duration,
    retry_max: Duration,
    metrics: MetricsHandle,
) -> CommandCompletion {
    let priority = envelope.priority;
    if envelope.is_abandoned() {
        metrics.cancelled_command();
        return CommandCompletion { key, priority };
    }
    metrics.command();
    metrics.command_started();
    let command_started = Instant::now();
    let result = AssertUnwindSafe(execute_with_retry(
        &envelope.operation,
        envelope.sequence,
        envelope.deadline,
        priority,
        &services,
        &global_capacity,
        &cooldown,
        attempt_timeout,
        max_retries,
        retry_base,
        retry_max,
        envelope.reply.as_ref(),
        &metrics,
    ))
    .catch_unwind()
    .await
    .unwrap_or(Err(CommandError::ServicePanicked));
    metrics.command_finished(command_started.elapsed());
    match &result {
        Err(CommandError::Cancelled) => metrics.cancelled_command(),
        Err(_) => metrics.command_error(),
        Ok(_) => {}
    }
    if let Some(reply) = envelope.reply {
        let _ = reply.send(result);
    } else if let Err(error) = result {
        tracing::warn!(%error, "deferred bot command failed");
    }
    drop(envelope._leases);
    CommandCompletion { key, priority }
}

#[allow(clippy::too_many_arguments)]
async fn execute_with_retry(
    operation: &CommandOperation,
    jitter_seed: u64,
    deadline: Option<Instant>,
    priority: CommandPriority,
    services: &BotServices,
    global_capacity: &GlobalCommandCapacity,
    cooldown: &BotCooldown,
    attempt_timeout: Option<Duration>,
    max_retries: u8,
    retry_base: Duration,
    retry_max: Duration,
    reply: Option<&CommandReply>,
    metrics: &RuntimeMetrics,
) -> std::result::Result<CommandResult, CommandError> {
    let idempotent = operation.idempotent(services.capabilities());
    let mut attempt = 0_u8;
    loop {
        if reply.is_some_and(|reply| reply.is_closed()) {
            return Err(CommandError::Cancelled);
        }
        cooldown.wait(deadline).await?;
        if reply.is_some_and(|reply| reply.is_closed()) {
            return Err(CommandError::Cancelled);
        }
        let permit = global_capacity.acquire(priority, deadline).await?;
        permit.touch();

        // Another in-flight command may have published a Retry-After while this
        // command was waiting for global capacity. Do not bypass that cooldown.
        if let Some(until) = cooldown.active_until() {
            drop(permit);
            if deadline.is_some_and(|deadline| until >= deadline) {
                return Err(CommandError::DeadlineExceeded);
            }
            await_before(deadline, tokio::time::sleep_until(until)).await?;
            continue;
        }
        if reply.is_some_and(|reply| reply.is_closed()) {
            drop(permit);
            return Err(CommandError::Cancelled);
        }

        // Calculate the attempt timeout after queue, cooldown, and global-capacity
        // waits, so a stale duration can never extend beyond the total deadline.
        let remaining = deadline.map(|deadline| deadline.saturating_duration_since(Instant::now()));
        if remaining.is_some_and(|remaining| remaining.is_zero()) {
            drop(permit);
            return Err(CommandError::DeadlineExceeded);
        }
        let timeout = match (attempt_timeout, remaining) {
            (Some(attempt_timeout), Some(remaining)) => Some(attempt_timeout.min(remaining)),
            (Some(attempt_timeout), None) => Some(attempt_timeout),
            (None, Some(remaining)) => Some(remaining),
            (None, None) => None,
        };

        let result = if let Some(timeout) = timeout {
            match tokio::time::timeout(timeout, execute_once(operation, services)).await {
                Ok(result) => result,
                Err(_) => Err(CommandError::Platform(PlatformError::new(
                    PlatformErrorKind::Timeout,
                    "platform command attempt timed out",
                ))),
            }
        } else {
            execute_once(operation, services).await
        };
        drop(permit);

        if reply.is_some_and(|reply| reply.is_closed()) {
            return Err(CommandError::Cancelled);
        }

        match result {
            Ok(value) => return Ok(value),
            Err(CommandError::Platform(error))
                if idempotent && error.is_retryable() && attempt < max_retries =>
            {
                metrics.command_retry();
                let exponential = 1_u32.checked_shl(attempt.into()).unwrap_or(u32::MAX);
                let delay = if let Some(retry_after) = error.retry_after {
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
                if error.kind == PlatformErrorKind::RateLimited {
                    metrics.rate_limit();
                    cooldown.extend(delay)?;
                } else {
                    await_before(deadline, tokio::time::sleep(delay)).await?;
                }
                attempt = attempt.saturating_add(1);
            }
            Err(CommandError::Platform(error)) => {
                if error.kind == PlatformErrorKind::RateLimited {
                    metrics.rate_limit();
                    if let Some(retry_after) = error.retry_after {
                        let _ = cooldown.extend(retry_after);
                    }
                }
                return Err(CommandError::Platform(error));
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
    services: &BotServices,
) -> std::result::Result<CommandResult, CommandError> {
    match operation {
        CommandOperation::Deliver { target, plan } => services
            .api
            .as_ref()
            .ok_or(CommandError::ApiUnsupported)?
            .send_delivery_plan(target.clone(), plan.clone())
            .await
            .map(CommandResult::Delivery)
            .map_err(command_api_error),
        CommandOperation::EditPublic {
            message,
            replacement,
        } => services
            .api
            .as_ref()
            .ok_or(CommandError::ApiUnsupported)?
            .edit_outgoing_message(message.clone(), replacement.clone())
            .await
            .map(|()| CommandResult::Unit)
            .map_err(|error| CommandError::Platform(platform_error(error))),
        CommandOperation::DeletePublic { message } => {
            let result = services
                .api
                .as_ref()
                .ok_or(CommandError::ApiUnsupported)?
                .delete_message_ref(message.clone())
                .await;
            match result {
                Ok(()) => Ok(CommandResult::Unit),
                Err(error) => {
                    let error = platform_error(error);
                    if services.capabilities().delete_idempotent
                        && error.kind == PlatformErrorKind::NotFound
                    {
                        Ok(CommandResult::Unit)
                    } else {
                        Err(CommandError::Platform(error))
                    }
                }
            }
        }
        CommandOperation::ReactPublic {
            message,
            reaction,
            options,
        } => services
            .api
            .as_ref()
            .ok_or(CommandError::ApiUnsupported)?
            .add_message_reaction(message.clone(), reaction.clone(), options.clone())
            .await
            .map(|()| CommandResult::Unit)
            .map_err(|error| CommandError::Platform(platform_error(error))),
        CommandOperation::AnswerInteraction { handle, response } => services
            .api
            .as_ref()
            .ok_or(CommandError::ApiUnsupported)?
            .answer_interaction(handle.id.clone(), response.clone())
            .await
            .map(|()| CommandResult::Unit)
            .map_err(|error| CommandError::Platform(platform_error(error))),
        CommandOperation::DeferInteraction { handle, visibility } => services
            .api
            .as_ref()
            .ok_or(CommandError::ApiUnsupported)?
            .defer_interaction(handle.clone(), *visibility)
            .await
            .map(|()| CommandResult::Unit)
            .map_err(|error| CommandError::Platform(platform_error(error))),
        CommandOperation::InteractionFollowup { handle, message } => services
            .api
            .as_ref()
            .ok_or(CommandError::ApiUnsupported)?
            .send_interaction_followup(handle.clone(), message.clone())
            .await
            .map(CommandResult::Messages)
            .map_err(|error| CommandError::Platform(platform_error(error))),
        CommandOperation::EditInteraction { handle, message } => services
            .api
            .as_ref()
            .ok_or(CommandError::ApiUnsupported)?
            .edit_interaction_response(handle.clone(), message.clone())
            .await
            .map(|()| CommandResult::Unit)
            .map_err(|error| CommandError::Platform(platform_error(error))),
        CommandOperation::PublishCommands { definitions } => services
            .api
            .as_ref()
            .ok_or(CommandError::ApiUnsupported)?
            .set_command_definitions(definitions.clone())
            .await
            .map(|()| CommandResult::Unit)
            .map_err(|error| CommandError::Platform(platform_error(error))),
    }
}
