use crate::{
    session::SessionRegistry, BotHandle, BuildError, HandlerError, HandlerResult, SessionKey,
    ShutdownSignal,
};
use futures_util::future::BoxFuture;
use oxidebot_core::{
    BotIdentity, ConversationChanged, EventBody, EventEnvelope, EventKind, FileChanged,
    Interaction, MemberChanged, MessageCreated, MessageTarget, MessageUpdated, MessagesDeleted,
    NativeEvent, OutgoingMessage, PaymentChanged, PlatformId, ReactionChanged,
};
use std::{
    future::Future,
    marker::PhantomData,
    panic::{catch_unwind, AssertUnwindSafe},
    str::FromStr,
    sync::Arc,
};

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

/// Type-level view over one canonical event body.
pub trait EventView: Send + Sync + 'static {
    /// Canonical category represented by this borrowed view.
    const KIND: EventKind;

    /// Returns a borrowed view when the envelope contains this event type.
    fn from_envelope(envelope: &EventEnvelope) -> Option<&Self>;
}

macro_rules! impl_event_view {
    ($type:ty, $variant:ident) => {
        impl EventView for $type {
            const KIND: EventKind = EventKind::$variant;

            fn from_envelope(envelope: &EventEnvelope) -> Option<&Self> {
                match &envelope.body {
                    EventBody::$variant(event) => Some(event.as_ref()),
                    _ => None,
                }
            }
        }
    };
}

impl_event_view!(MessageCreated, MessageCreated);
impl_event_view!(MessageUpdated, MessageUpdated);
impl_event_view!(MessagesDeleted, MessagesDeleted);
impl_event_view!(Interaction, Interaction);
impl_event_view!(ReactionChanged, ReactionChanged);
impl_event_view!(MemberChanged, MemberChanged);
impl_event_view!(ConversationChanged, ConversationChanged);
impl_event_view!(FileChanged, FileChanged);
impl_event_view!(PaymentChanged, PaymentChanged);
impl_event_view!(NativeEvent, Native);

/// Typed context passed to one route handler. The typed event is a borrowed view
/// of the single shared envelope; matching multiple handlers never clones the
/// canonical event body or its vectors. Context is intentionally not `Clone`;
/// retaining events outside a handler must be an explicit application decision.
pub struct Context<E, S = ()>
where
    E: EventView,
    S: Send + Sync + 'static,
{
    envelope: Arc<EventEnvelope>,
    state: Arc<S>,
    bot: BotHandle,
    sessions: SessionRegistry,
    shutdown: ShutdownSignal,
    _event: PhantomData<fn() -> E>,
}

impl<E, S> Context<E, S>
where
    E: EventView,
    S: Send + Sync + 'static,
{
    /// Returns the typed canonical event by reference.
    #[must_use]
    pub fn event(&self) -> &E {
        E::from_envelope(&self.envelope)
            .expect("compiled route and event view must describe the same event kind")
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

    /// Read-only structured-shutdown signal. Handlers cannot cancel siblings or
    /// the application root.
    #[must_use]
    pub fn shutdown(&self) -> &ShutdownSignal {
        &self.shutdown
    }
}

impl<S> Context<MessageCreated, S>
where
    S: Send + Sync + 'static,
{
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        self.event().text.as_deref()
    }

    pub async fn reply(
        &self,
        message: impl Into<OutgoingMessage>,
    ) -> Result<oxidebot_core::MessageReceipt, crate::CommandError> {
        self.bot
            .send(
                MessageTarget::new(self.event().reference.conversation.clone()),
                message,
            )
            .await
    }

    /// Registers an exclusive one-shot session before sending the prompt. The
    /// registration is synchronously cancelled if sending or the handler future
    /// is cancelled.
    pub async fn ask_parse<T>(
        &self,
        prompt: impl Into<OutgoingMessage>,
        options: crate::AskOptions,
    ) -> Result<T, HandlerError>
    where
        T: FromStr,
        T::Err: std::fmt::Display,
    {
        let conversation = self.event().reference.conversation.clone();
        let actor = self
            .event()
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

/// Fully compiled route category used by the router's candidate indexes.
///
/// This type is public only so advanced users can implement [`Matcher`]; normal
/// applications should use the built-in matcher constructors.
#[doc(hidden)]
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum RouteSpec {
    Generic(EventKind),
    Command(Arc<str>),
    Interaction(Arc<str>),
    Native(Arc<str>),
}

/// Optional adapter scope attached to one compiled route.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct RouteScope {
    pub(crate) platform: Option<PlatformId>,
    pub(crate) bot: Option<BotIdentity>,
}

impl RouteScope {
    /// Creates an unrestricted route scope.
    #[must_use]
    pub const fn global() -> Self {
        Self {
            platform: None,
            bot: None,
        }
    }

    /// Restricts a route to one platform.
    #[must_use]
    pub fn for_platform(platform: PlatformId) -> Self {
        Self {
            platform: Some(platform),
            bot: None,
        }
    }

    /// Restricts a route to one exact bot identity.
    #[must_use]
    pub fn for_bot(bot: BotIdentity) -> Self {
        Self {
            platform: Some(bot.platform.clone()),
            bot: Some(bot),
        }
    }

    pub(crate) fn matches(&self, identity: &BotIdentity) -> bool {
        self.platform
            .as_ref()
            .is_none_or(|platform| platform == &identity.platform)
            && self.bot.as_ref().is_none_or(|bot| bot == identity)
    }
}

/// A typed route matcher. Matching is compiled once; it is not invoked by every
/// event at runtime.
pub trait Matcher: Send + Sync + 'static {
    type Event: EventView;
    fn route_spec(&self) -> RouteSpec;
}

/// Generic matcher for one canonical event category.
#[derive(Clone, Copy, Debug)]
pub struct KindMatcher<E> {
    kind: EventKind,
    _event: PhantomData<fn() -> E>,
}

impl<E> Matcher for KindMatcher<E>
where
    E: EventView,
{
    type Event = E;

    fn route_spec(&self) -> RouteSpec {
        RouteSpec::Generic(self.kind)
    }
}

const fn kind_matcher<E>(kind: EventKind) -> KindMatcher<E> {
    KindMatcher {
        kind,
        _event: PhantomData,
    }
}

/// Matches all message-updated events.
#[must_use]
pub const fn message_updated() -> KindMatcher<MessageUpdated> {
    kind_matcher(EventKind::MessageUpdated)
}

/// Matches all message-deletion batches.
#[must_use]
pub const fn messages_deleted() -> KindMatcher<MessagesDeleted> {
    kind_matcher(EventKind::MessagesDeleted)
}

/// Matches every interaction, irrespective of custom ID.
#[must_use]
pub const fn any_interaction() -> KindMatcher<Interaction> {
    kind_matcher(EventKind::Interaction)
}

/// Matches all reaction changes.
#[must_use]
pub const fn reaction_changed() -> KindMatcher<ReactionChanged> {
    kind_matcher(EventKind::ReactionChanged)
}

/// Matches all membership changes.
#[must_use]
pub const fn member_changed() -> KindMatcher<MemberChanged> {
    kind_matcher(EventKind::MemberChanged)
}

/// Matches all conversation state changes.
#[must_use]
pub const fn conversation_changed() -> KindMatcher<ConversationChanged> {
    kind_matcher(EventKind::ConversationChanged)
}

/// Matches all file lifecycle changes.
#[must_use]
pub const fn file_changed() -> KindMatcher<FileChanged> {
    kind_matcher(EventKind::FileChanged)
}

/// Matches all payment/subscription changes.
#[must_use]
pub const fn payment_changed() -> KindMatcher<PaymentChanged> {
    kind_matcher(EventKind::PaymentChanged)
}

/// Matches all platform-native events.
#[must_use]
pub const fn any_native() -> KindMatcher<NativeEvent> {
    kind_matcher(EventKind::Native)
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

    fn route_spec(&self) -> RouteSpec {
        self.command
            .as_ref()
            .map_or(RouteSpec::Generic(EventKind::MessageCreated), |command| {
                RouteSpec::Command(command.clone())
            })
    }
}

/// Matcher for one interaction custom identifier.
#[derive(Clone, Debug)]
pub struct InteractionMatcher {
    custom_id: Arc<str>,
}

#[must_use]
pub fn interaction(custom_id: impl Into<Arc<str>>) -> InteractionMatcher {
    InteractionMatcher {
        custom_id: custom_id.into(),
    }
}

impl Matcher for InteractionMatcher {
    type Event = Interaction;

    fn route_spec(&self) -> RouteSpec {
        RouteSpec::Interaction(self.custom_id.clone())
    }
}

/// Matcher for one platform-native event type.
#[derive(Clone, Debug)]
pub struct NativeMatcher {
    kind: Arc<str>,
}

#[must_use]
pub fn native(kind: impl Into<Arc<str>>) -> NativeMatcher {
    NativeMatcher { kind: kind.into() }
}

impl Matcher for NativeMatcher {
    type Event = NativeEvent;

    fn route_spec(&self) -> RouteSpec {
        RouteSpec::Native(self.kind.clone())
    }
}

/// Concrete pairing of a typed matcher and async handler function.
pub struct On<M, F, S> {
    matcher: M,
    function: F,
    scope: RouteScope,
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
        scope: RouteScope::default(),
        _state: PhantomData,
    }
}

impl<M, F, S> On<M, F, S> {
    /// Restricts this route to one platform.
    #[must_use]
    pub fn platform(mut self, platform: PlatformId) -> Self {
        self.scope = RouteScope::for_platform(platform);
        self
    }

    /// Restricts this route to one exact bot identity.
    #[must_use]
    pub fn bot(mut self, bot: BotIdentity) -> Self {
        self.scope = RouteScope::for_bot(bot);
        self
    }
}

#[doc(hidden)]
pub trait ErasedHandler<S>: Send + Sync + 'static
where
    S: Send + Sync + 'static,
{
    fn route_spec(&self) -> RouteSpec;
    fn route_scope(&self) -> RouteScope;
    fn event_kind(&self) -> EventKind;
    fn call(
        &self,
        event: Arc<EventEnvelope>,
        state: Arc<S>,
        bot: BotHandle,
        sessions: SessionRegistry,
        shutdown: ShutdownSignal,
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
    fn route_spec(&self) -> RouteSpec {
        self.matcher.route_spec()
    }

    fn route_scope(&self) -> RouteScope {
        self.scope.clone()
    }

    fn event_kind(&self) -> EventKind {
        M::Event::KIND
    }

    fn call(
        &self,
        event: Arc<EventEnvelope>,
        state: Arc<S>,
        bot: BotHandle,
        sessions: SessionRegistry,
        shutdown: ShutdownSignal,
    ) -> BoxFuture<'static, HandlerResult> {
        debug_assert!(M::Event::from_envelope(&event).is_some());
        let future = (self.function)(Context {
            envelope: event,
            state,
            bot,
            sessions,
            shutdown,
            _event: PhantomData,
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

/// Build-time snapshot of a dynamic handler's immutable routing metadata.
pub(crate) struct PreparedHandler<S>
where
    S: Send + Sync + 'static,
{
    pub(crate) spec: RouteSpec,
    pub(crate) scope: RouteScope,
    pub(crate) event_kind: EventKind,
    pub(crate) handler: Arc<dyn ErasedHandler<S>>,
}

pub(crate) fn prepare_handler<S>(
    handler: Arc<dyn ErasedHandler<S>>,
) -> Result<PreparedHandler<S>, BuildError>
where
    S: Send + Sync + 'static,
{
    let (spec, scope, event_kind) = catch_unwind(AssertUnwindSafe(|| {
        (
            handler.route_spec(),
            handler.route_scope(),
            handler.event_kind(),
        )
    }))
    .map_err(|_| BuildError::InvalidRoute("handler routing metadata panicked".into()))?;
    Ok(PreparedHandler {
        spec,
        scope,
        event_kind,
        handler,
    })
}
