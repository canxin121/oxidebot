use crate::{BotHandle, CommandResult, SessionRegistry, ShutdownSignal};
use oxidebot_core::{
    event::{kernel::DispatchEnvelope, EventType, MessageEvent},
    BotObject, Event,
};
use std::{
    any::{Any, TypeId},
    collections::HashMap,
    sync::Arc,
};

/// Per-dispatch typed values shared by extractors and middleware.
#[derive(Default)]
pub struct Extensions {
    values: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl Extensions {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert<T>(&mut self, value: T) -> Option<T>
    where
        T: Send + Sync + 'static,
    {
        self.values
            .insert(TypeId::of::<T>(), Box::new(value))
            .and_then(|old| old.downcast::<T>().ok())
            .map(|value| *value)
    }

    #[must_use]
    pub fn get<T>(&self) -> Option<&T>
    where
        T: Send + Sync + 'static,
    {
        self.values
            .get(&TypeId::of::<T>())
            .and_then(|value| value.downcast_ref())
    }

    pub fn get_mut<T>(&mut self) -> Option<&mut T>
    where
        T: Send + Sync + 'static,
    {
        self.values
            .get_mut(&TypeId::of::<T>())
            .and_then(|value| value.downcast_mut())
    }

    pub fn remove<T>(&mut self) -> Option<T>
    where
        T: Send + Sync + 'static,
    {
        self.values
            .remove(&TypeId::of::<T>())
            .and_then(|value| value.downcast::<T>().ok())
            .map(|value| *value)
    }

    #[must_use]
    pub fn contains<T>(&self) -> bool
    where
        T: Send + Sync + 'static,
    {
        self.values.contains_key(&TypeId::of::<T>())
    }
}

impl std::fmt::Debug for Extensions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Extensions")
            .field("len", &self.values.len())
            .finish()
    }
}

/// One handler request. It owns only clone-cheap runtime handles and one shared
/// dispatch allocation; extractors borrow from it on demand.
pub struct Request<S = ()>
where
    S: Send + Sync + 'static,
{
    pub(crate) envelope: Arc<DispatchEnvelope>,
    pub(crate) state: Arc<S>,
    pub(crate) bot: BotHandle,
    pub(crate) sessions: SessionRegistry,
    pub(crate) shutdown: ShutdownSignal,
    pub(crate) extensions: Extensions,
    pub(crate) command: Option<CommandResult>,
}

impl<S> Request<S>
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
            extensions: Extensions::new(),
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
        &self.state
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
    pub fn shutdown(&self) -> &ShutdownSignal {
        &self.shutdown
    }

    #[must_use]
    pub fn command(&self) -> Option<&CommandResult> {
        self.command.as_ref()
    }

    #[must_use]
    pub fn extensions(&self) -> &Extensions {
        &self.extensions
    }

    pub fn extensions_mut(&mut self) -> &mut Extensions {
        &mut self.extensions
    }

    #[doc(hidden)]
    #[must_use]
    pub fn dispatch_envelope(&self) -> &DispatchEnvelope {
        &self.envelope
    }
}

impl<S> std::fmt::Debug for Request<S>
where
    S: Send + Sync + 'static,
{
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Request")
            .field("event_id", &self.envelope.id)
            .field("event_type", &self.envelope.index.event_type)
            .field("bot", &self.bot.identity())
            .field("command", &self.command)
            .field("extensions", &self.extensions)
            .finish_non_exhaustive()
    }
}
