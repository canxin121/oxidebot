use crate::{
    authoring::AuthoringRuntime, Address, BotHandle, BotSelection, CommandResult, HandlerError,
    HandlerResult, SessionRegistry, ShutdownSignal,
};
use oxidebot_core::{
    event::{kernel::DispatchEnvelope, EventType, MessageEvent},
    source::message::{DeliveryReport, FallbackPolicy, Message},
    BotObject, Event,
};
use std::sync::Arc;

/// Clone-cheap context for one matched bot event.
///
/// `Context` is the single runtime view shared by extractors, guards, hooks,
/// and typed [`crate::EventContext`] values. It owns no copied event payload:
/// every clone points at the same [`DispatchEnvelope`].
pub struct Context<S = ()>
where
    S: Send + Sync + 'static,
{
    pub(crate) envelope: Arc<DispatchEnvelope>,
    pub(crate) state: Arc<S>,
    pub(crate) bot: BotHandle,
    pub(crate) sessions: SessionRegistry,
    pub(crate) shutdown: ShutdownSignal,
    pub(crate) command: Option<CommandResult>,
    pub(crate) authoring: Arc<AuthoringRuntime<S>>,
}

impl<S> Context<S>
where
    S: Send + Sync + 'static,
{
    pub(crate) fn new(
        envelope: Arc<DispatchEnvelope>,
        state: Arc<S>,
        bot: BotHandle,
        sessions: SessionRegistry,
        shutdown: ShutdownSignal,
        command: Option<CommandResult>,
        authoring: Arc<AuthoringRuntime<S>>,
    ) -> Self {
        Self {
            envelope,
            state,
            bot,
            sessions,
            shutdown,
            command,
            authoring,
        }
    }

    #[must_use]
    pub fn event(&self) -> &Event {
        self.envelope.event()
    }

    #[must_use]
    pub fn event_type(&self) -> EventType {
        self.envelope.index.event_type
    }

    #[must_use]
    pub fn message(&self) -> Option<&MessageEvent> {
        match self.event() {
            Event::MessageEvent(event) => Some(event),
            _ => None,
        }
    }

    #[must_use]
    pub fn state(&self) -> &S {
        self.state.as_ref()
    }

    #[must_use]
    pub fn state_arc(&self) -> Arc<S> {
        Arc::clone(&self.state)
    }

    #[must_use]
    pub fn bot_handle(&self) -> &BotHandle {
        &self.bot
    }

    pub fn bot(&self) -> Result<BotObject, crate::CommandError> {
        self.bot.api()
    }

    #[must_use]
    pub fn bot_identity(&self) -> &oxidebot_core::BotIdentity {
        self.bot.identity()
    }

    #[must_use]
    pub fn shutdown(&self) -> &ShutdownSignal {
        &self.shutdown
    }

    #[must_use]
    pub fn command(&self) -> Option<&CommandResult> {
        self.command.as_ref()
    }

    /// Sends one canonical message to an explicit address through the current
    /// handler's delivery middleware and capability planner.
    pub async fn send_to(
        &self,
        address: Address,
        message: impl Into<Message>,
        fallback: FallbackPolicy,
    ) -> HandlerResult<DeliveryReport> {
        match &address.bot {
            BotSelection::Current => {}
            BotSelection::Exact(identity) if identity == self.bot_identity() => {}
            BotSelection::Platform(platform) if platform == &self.bot_identity().platform => {}
            BotSelection::Exact(_) | BotSelection::Platform(_) => {
                return Err(HandlerError::Api(
                    "the selected address requires another bot; use BotDirectory outside the handler context"
                        .into(),
                ));
            }
        }
        let api = self
            .bot()
            .map_err(|error| HandlerError::Api(error.to_string()))?;
        self.authoring
            .deliver(Some(self), &api, address.target, message.into(), fallback)
            .await
    }

    /// Sends with OxideBot's capability-aware automatic fallback policy.
    pub async fn send(
        &self,
        address: Address,
        message: impl Into<Message>,
    ) -> HandlerResult<DeliveryReport> {
        self.send_to(address, message, FallbackPolicy::Auto).await
    }

    #[doc(hidden)]
    #[must_use]
    pub(crate) fn authoring(&self) -> &AuthoringRuntime<S> {
        &self.authoring
    }

    #[doc(hidden)]
    #[must_use]
    pub(crate) fn authoring_arc(&self) -> Arc<AuthoringRuntime<S>> {
        Arc::clone(&self.authoring)
    }

    #[doc(hidden)]
    #[must_use]
    pub fn dispatch_envelope(&self) -> &DispatchEnvelope {
        &self.envelope
    }
}

impl<S> Clone for Context<S>
where
    S: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            envelope: Arc::clone(&self.envelope),
            state: Arc::clone(&self.state),
            bot: self.bot.clone(),
            sessions: self.sessions.clone(),
            shutdown: self.shutdown.clone(),
            command: self.command.clone(),
            authoring: Arc::clone(&self.authoring),
        }
    }
}

impl<S> std::fmt::Debug for Context<S>
where
    S: Send + Sync + 'static,
{
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Context")
            .field("event_id", &self.envelope.id)
            .field("event_type", &self.envelope.index.event_type)
            .field("bot", &self.bot.identity())
            .field("command", &self.command)
            .finish_non_exhaustive()
    }
}
