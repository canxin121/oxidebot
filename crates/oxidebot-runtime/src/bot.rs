use crate::{CommandError, PlatformError, PlatformErrorKind};
use oxidebot_core::{
    application::CommandDefinition,
    capability::BotCapabilities,
    collaboration::{Reaction, ReactionOptions},
    conversation::{MessageRef as PublicMessageRef, MessageTarget as PublicMessageTarget},
    interaction::{InteractionResponse, InteractionResponseHandle, InteractionVisibility},
    source::message::{
        DeliveryPlan as PublicDeliveryPlan, DeliveryReport as PublicDeliveryReport,
        Message as PublicMessage,
    },
    BotId, BotIdentity, BotObject, BotSlot, CallApiTrait, CallError, FallbackPolicy, InvalidId,
    PlatformId,
};
use std::{
    fmt,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::Arc,
};

#[path = "bot_command.rs"]
mod bot_command;

use bot_command::{ApiCommandResult, CommandClient, CommandOperation};
pub(crate) use bot_command::{CommandWorker, GlobalCommandCapacity};

/// Immutable identity for one adapter connection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BotDescriptor {
    /// Platform identifier supplied by the adapter.
    pub platform: PlatformId,
    /// Platform-specific bot identifier.
    pub id: BotId,
    /// Optional human-readable name used in diagnostics.
    pub display_name: Option<Arc<str>>,
}

impl BotDescriptor {
    /// Creates a descriptor from its platform and platform-specific bot ID.
    #[must_use]
    pub fn new(platform: PlatformId, id: BotId) -> Self {
        Self {
            platform,
            id,
            display_name: None,
        }
    }

    /// Sets the human-readable display name used in diagnostics.
    #[must_use]
    pub fn display_name(mut self, display_name: impl Into<Arc<str>>) -> Self {
        self.display_name = Some(display_name.into());
        self
    }

    /// Returns the combined stable bot identity.
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
    /// Neither the adapter nor platform can ensure idempotent sends.
    #[default]
    Unsupported,
    /// The adapter enforces idempotency above a platform without native support.
    AdapterEmulated,
    /// The underlying platform enforces idempotency keys natively.
    PlatformNative,
}

/// Runtime retry guarantees supplied by one bot connection.
///
/// Portable feature support belongs exclusively to
/// [`oxidebot_core::BotCapabilities`]. This type intentionally contains only
/// scheduler semantics that cannot be inferred from a feature's availability.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RuntimeCapabilities {
    /// Idempotency guarantee applied to message sends.
    pub send_idempotency: IdempotencyGuarantee,
    /// Whether retrying a completed delete treats not-found as success.
    pub delete_idempotent: bool,
}

fn platform_error(error: CallError) -> PlatformError {
    match error {
        CallError::Temporary { message } => {
            PlatformError::new(PlatformErrorKind::Temporary, message)
        }
        CallError::RateLimited {
            message,
            retry_after,
        } => {
            let mut error = PlatformError::new(PlatformErrorKind::RateLimited, message);
            if let Some(retry_after) = retry_after {
                error = error.retry_after(retry_after);
            }
            error
        }
        CallError::Timeout { message } => PlatformError::new(PlatformErrorKind::Timeout, message),
        CallError::NotFound { message } => PlatformError::new(PlatformErrorKind::NotFound, message),
        CallError::Unsupported { feature } => {
            PlatformError::new(PlatformErrorKind::Unsupported, feature)
        }
        CallError::InvalidRequest { message } => {
            PlatformError::new(PlatformErrorKind::InvalidRequest, message)
        }
        CallError::Permanent { message } => {
            PlatformError::new(PlatformErrorKind::Permanent, message)
        }
        CallError::Planning(error) => {
            PlatformError::new(PlatformErrorKind::InvalidRequest, error.to_string())
        }
        CallError::PartialDelivery(error) => {
            PlatformError::new(PlatformErrorKind::Permanent, error.to_string())
        }
    }
}

fn command_api_error(error: CallError) -> CommandError {
    match error {
        CallError::PartialDelivery(error) => CommandError::PartialDelivery(error),
        error => CommandError::Platform(platform_error(error)),
    }
}

/// Runtime services derived from one OxideBot API implementation.
#[derive(Clone)]
pub struct BotServices {
    pub(crate) api: Option<Arc<dyn CallApiTrait>>,
    capabilities: RuntimeCapabilities,
    bot_capabilities: Arc<BotCapabilities>,
}

impl BotServices {
    /// Creates adapter services from the single OxideBot API object.
    /// Returns scheduling and retry semantics supplied by this adapter.
    #[must_use]
    pub fn new(api: Arc<dyn CallApiTrait>) -> Self {
        let bot_capabilities = Arc::new(api.bot_capabilities());
        Self {
            api: Some(api),
            capabilities: RuntimeCapabilities::default(),
            bot_capabilities,
        }
    }

    /// Declares that message idempotency keys are actually enforced.
    /// Returns the runtime slot assigned to this bot connection.
    #[must_use]
    pub fn send_idempotency(mut self, guarantee: IdempotencyGuarantee) -> Self {
        self.capabilities.send_idempotency = guarantee;
        self
    }

    /// Declares delete retry semantics (`NotFound` after a prior success is OK).
    /// Returns this connection's stable bot identity.
    #[must_use]
    pub fn idempotent_delete(mut self, enabled: bool) -> Self {
        self.capabilities.delete_idempotent = enabled;
        self
    }

    /// Returns immutable adapter descriptor metadata.
    #[must_use]
    pub fn capabilities(&self) -> RuntimeCapabilities {
        self.capabilities
    }

    /// Returns the immutable portable capability model captured when the
    /// adapter registered its services. Runtime hot paths never re-query an
    /// adapter for this bot-wide metadata.
    /// Returns scheduling and retry semantics for this bot connection.
    #[must_use]
    pub fn bot_capabilities(&self) -> &BotCapabilities {
        &self.bot_capabilities
    }
}

impl fmt::Debug for BotServices {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BotServices")
            .field("capabilities", &self.capabilities)
            .field("bot_capabilities", &self.bot_capabilities)
            .finish_non_exhaustive()
    }
}

struct BotHandleInner {
    slot: BotSlot,
    identity: BotIdentity,
    descriptor: BotDescriptor,
    capabilities: RuntimeCapabilities,
    bot_capabilities: Arc<BotCapabilities>,
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
        bot_capabilities: Arc<BotCapabilities>,
        api: Option<Arc<dyn CallApiTrait>>,
        client: CommandClient,
    ) -> Self {
        let identity = descriptor.identity();
        Self(Arc::new(BotHandleInner {
            slot,
            identity,
            descriptor,
            capabilities,
            bot_capabilities,
            api,
            client,
        }))
    }

    /// Returns the runtime slot assigned to this bot connection.
    #[must_use]
    pub fn slot(&self) -> BotSlot {
        self.0.slot
    }

    /// Returns this connection's stable bot identity.
    #[must_use]
    pub fn identity(&self) -> &BotIdentity {
        &self.0.identity
    }

    /// Returns immutable adapter descriptor metadata.
    #[must_use]
    pub fn descriptor(&self) -> &BotDescriptor {
        &self.0.descriptor
    }

    /// Returns scheduling and retry semantics for this bot connection.
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

    /// Returns the immutable public capability model captured at registration.
    pub(crate) fn bot_capabilities(&self) -> std::result::Result<BotCapabilities, CommandError> {
        Ok((*self.0.bot_capabilities).clone())
    }

    /// Plans one public message without invoking transport. Planning is kept
    /// outside the queue so delivery middleware can inspect and transform the
    /// exact physical plan before the bounded transport operation is admitted.
    pub(crate) fn plan_outgoing_message(
        &self,
        target: &PublicMessageTarget,
        message: &PublicMessage,
        fallback: FallbackPolicy,
    ) -> std::result::Result<PublicDeliveryPlan, CommandError> {
        let api = self.api()?;
        catch_unwind(AssertUnwindSafe(|| {
            api.plan_outgoing_message(target, message, fallback)
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
            ApiCommandResult::Delivery(report) => Ok(report),
            _ => Err(CommandError::UnexpectedResult),
        }
    }

    pub(crate) async fn send_outgoing_message_with(
        &self,
        target: PublicMessageTarget,
        message: PublicMessage,
        fallback: FallbackPolicy,
    ) -> std::result::Result<PublicDeliveryReport, CommandError> {
        let plan = self.plan_outgoing_message(&target, &message, fallback)?;
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
            ApiCommandResult::Unit => Ok(()),
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
            ApiCommandResult::Unit => Ok(()),
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
            ApiCommandResult::Unit => Ok(()),
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
            ApiCommandResult::Unit => Ok(()),
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
            ApiCommandResult::Unit => Ok(()),
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
            ApiCommandResult::Messages(messages) => Ok(messages),
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
            ApiCommandResult::Unit => Ok(()),
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
            ApiCommandResult::Unit => Ok(()),
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

    /// Returns the bot handle assigned to `slot`, if present.
    #[must_use]
    pub fn get(&self, slot: BotSlot) -> Option<&BotHandle> {
        self.0.get(slot.0 as usize)
    }

    /// Iterates over connected bots in dense slot order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &BotHandle> {
        self.0.iter()
    }

    /// Returns the number of connected bots.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns whether no bots are connected.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
