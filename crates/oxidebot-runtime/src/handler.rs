use crate::{
    adapter::InterestPlan, session::SessionRegistry, BotHandle, HandlerError, HandlerResult,
    SessionKey,
};
use futures_util::future::BoxFuture;
use oxidebot_core::{
    EventEnvelope, EventIndex, EventKind, Interaction, MessageCreated, MessageTarget, NativeEvent,
    OutgoingMessage,
};
use std::{future::Future, marker::PhantomData, str::FromStr, sync::Arc};
use tokio_util::sync::CancellationToken;

/// Result of a route, including deferred platform actions.
#[derive(Clone, Debug, Default)]
pub struct Outcome {
    pub(crate) stop: bool,
    pub(crate) replies: Vec<OutgoingMessage>,
}

impl Outcome {
    #[must_use]
    pub const fn continue_() -> Self {
        Self {
            stop: false,
            replies: Vec::new(),
        }
    }
    #[must_use]
    pub const fn stop() -> Self {
        Self {
            stop: true,
            replies: Vec::new(),
        }
    }
    #[must_use]
    pub fn reply(mut self, message: impl Into<OutgoingMessage>) -> Self {
        self.replies.push(message.into());
        self
    }
}

/// Typed context passed to one route handler.
#[derive(Clone)]
pub struct Context<E, S = ()>
where
    S: Send + Sync + 'static,
{
    event: E,
    envelope: Arc<EventEnvelope>,
    state: Arc<S>,
    bot: BotHandle,
    sessions: SessionRegistry,
    cancellation: CancellationToken,
}

impl<E, S> Context<E, S>
where
    S: Send + Sync + 'static,
{
    #[must_use]
    pub fn event(&self) -> &E {
        &self.event
    }
    #[must_use]
    pub fn envelope(&self) -> &EventEnvelope {
        &self.envelope
    }
    #[must_use]
    pub fn state(&self) -> &S {
        &self.state
    }
    #[must_use]
    pub fn bot(&self) -> &BotHandle {
        &self.bot
    }
    #[must_use]
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }
}

impl<S> Context<MessageCreated, S>
where
    S: Send + Sync + 'static,
{
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        self.event.text.as_deref()
    }

    pub async fn reply(
        &self,
        message: impl Into<OutgoingMessage>,
    ) -> Result<oxidebot_core::MessageReceipt, crate::CommandError> {
        self.bot
            .send(
                MessageTarget::new(self.event.reference.conversation.clone()),
                message,
            )
            .await
    }

    pub async fn ask_parse<T>(
        &self,
        prompt: impl Into<OutgoingMessage>,
        options: crate::AskOptions,
    ) -> Result<T, HandlerError>
    where
        T: FromStr,
        T::Err: std::fmt::Display,
    {
        let conversation = self.event.reference.conversation.clone();
        let actor = self
            .event
            .sender
            .clone()
            .ok_or_else(|| HandlerError::Parse("message has no sender".into()))?;
        let key = SessionKey::new(conversation.clone(), actor, options.namespace.clone());
        let waiter = self
            .sessions
            .register(key, options.timeout, options.policy)
            .await?;
        self.bot
            .send(MessageTarget::new(conversation), prompt)
            .await?;
        let response = waiter.wait().await?;
        let text = response
            .event()
            .message_created()
            .and_then(|message| message.text.as_deref())
            .ok_or_else(|| HandlerError::Parse("session response has no text".into()))?;
        text.parse()
            .map_err(|error: T::Err| HandlerError::Parse(error.to_string()))
    }
}

/// A typed route matcher.
pub trait Matcher: Send + Sync + 'static {
    type Event: Clone + Send + Sync + 'static;
    fn matches(&self, index: &EventIndex) -> bool;
    fn event(&self, envelope: &EventEnvelope) -> Option<Self::Event>;
    fn add_interest(&self, plan: &mut InterestPlan);
}

/// Matcher for canonical message-created events.
#[derive(Clone, Debug, Default)]
pub struct MessageMatcher {
    command: Option<Arc<str>>,
}

/// Matches all incoming messages.
#[must_use]
pub fn message() -> MessageMatcher {
    MessageMatcher::default()
}

impl MessageMatcher {
    /// Restricts this matcher to a normalized command name.
    #[must_use]
    pub fn command(mut self, value: impl Into<Arc<str>>) -> Self {
        self.command = Some(value.into());
        self
    }
}

impl Matcher for MessageMatcher {
    type Event = MessageCreated;
    fn matches(&self, index: &EventIndex) -> bool {
        index.kind == EventKind::MessageCreated
            && self
                .command
                .as_ref()
                .is_none_or(|command| index.command.as_ref() == Some(command))
    }
    fn event(&self, envelope: &EventEnvelope) -> Option<Self::Event> {
        envelope.message_created().cloned()
    }
    fn add_interest(&self, plan: &mut InterestPlan) {
        if let Some(command) = &self.command {
            plan.add_command(command.clone());
        } else {
            plan.add_generic(EventKind::MessageCreated);
        }
    }
}

/// Matcher for one interaction custom identifier.
#[derive(Clone, Debug)]
pub struct InteractionMatcher {
    custom_id: Arc<str>,
}

/// Matches interaction events with the given custom identifier.
#[must_use]
pub fn interaction(custom_id: impl Into<Arc<str>>) -> InteractionMatcher {
    InteractionMatcher {
        custom_id: custom_id.into(),
    }
}

impl Matcher for InteractionMatcher {
    type Event = Interaction;
    fn matches(&self, index: &EventIndex) -> bool {
        index.kind == EventKind::Interaction && index.interaction.as_ref() == Some(&self.custom_id)
    }
    fn event(&self, envelope: &EventEnvelope) -> Option<Self::Event> {
        envelope.interaction().cloned()
    }
    fn add_interest(&self, plan: &mut InterestPlan) {
        plan.add_interaction(self.custom_id.clone());
    }
}

/// Matcher for one platform-native event type.
#[derive(Clone, Debug)]
pub struct NativeMatcher {
    kind: Arc<str>,
}

/// Matches native events with the given stable platform type.
#[must_use]
pub fn native(kind: impl Into<Arc<str>>) -> NativeMatcher {
    NativeMatcher { kind: kind.into() }
}

impl Matcher for NativeMatcher {
    type Event = NativeEvent;
    fn matches(&self, index: &EventIndex) -> bool {
        index.kind == EventKind::Native && index.native_type.as_ref() == Some(&self.kind)
    }
    fn event(&self, envelope: &EventEnvelope) -> Option<Self::Event> {
        match &envelope.body {
            oxidebot_core::EventBody::Native(event) => Some((**event).clone()),
            _ => None,
        }
    }
    fn add_interest(&self, plan: &mut InterestPlan) {
        plan.add_native_type(self.kind.clone());
    }
}

/// Concrete pairing of a typed matcher and async handler function.
pub struct On<M, F, S> {
    matcher: M,
    function: F,
    _state: PhantomData<fn(S)>,
}

/// Creates a typed handler registration.
#[must_use]
pub fn on<M, F, Fut, S>(matcher: M, function: F) -> On<M, F, S>
where
    M: Matcher,
    F: Fn(Context<M::Event, S>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = HandlerResult> + Send + 'static,
    S: Send + Sync + 'static,
{
    On {
        matcher,
        function,
        _state: PhantomData,
    }
}

#[doc(hidden)]
pub trait ErasedHandler<S>: Send + Sync + 'static
where
    S: Send + Sync + 'static,
{
    fn add_interest(&self, plan: &mut InterestPlan);
    fn matches(&self, index: &EventIndex) -> bool;
    fn call(
        &self,
        event: Arc<EventEnvelope>,
        state: Arc<S>,
        bot: BotHandle,
        sessions: SessionRegistry,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, HandlerResult>;
}

/// Marker implemented by values accepted by [`crate::OxideBot::handler`].
pub trait Handler<S>: ErasedHandler<S>
where
    S: Send + Sync + 'static,
{
}
impl<S, T> Handler<S> for T
where
    S: Send + Sync + 'static,
    T: ErasedHandler<S>,
{
}

impl<M, F, Fut, S> ErasedHandler<S> for On<M, F, S>
where
    M: Matcher,
    F: Fn(Context<M::Event, S>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = HandlerResult> + Send + 'static,
    S: Send + Sync + 'static,
{
    fn add_interest(&self, plan: &mut InterestPlan) {
        self.matcher.add_interest(plan);
    }
    fn matches(&self, index: &EventIndex) -> bool {
        self.matcher.matches(index)
    }
    fn call(
        &self,
        event: Arc<EventEnvelope>,
        state: Arc<S>,
        bot: BotHandle,
        sessions: SessionRegistry,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, HandlerResult> {
        let Some(typed) = self.matcher.event(&event) else {
            return Box::pin(async { Ok(Outcome::continue_()) });
        };
        let future = (self.function)(Context {
            event: typed,
            envelope: event,
            state,
            bot,
            sessions,
            cancellation,
        });
        Box::pin(future)
    }
}

pub(crate) fn erase_handler<S, H>(handler: H) -> Arc<dyn ErasedHandler<S>>
where
    S: Send + Sync + 'static,
    H: Handler<S>,
{
    Arc::new(handler)
}
