use crate::{BotHandle, CommandResult, SessionRegistry, ShutdownSignal};
use oxidebot_core::{
    event::{kernel::DispatchEnvelope, EventType, MessageEvent},
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
    ) -> Self {
        Self {
            envelope,
            state,
            bot,
            sessions,
            shutdown,
            command,
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
