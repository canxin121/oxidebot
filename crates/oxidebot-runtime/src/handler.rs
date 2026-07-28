use crate::{
    session::SessionRegistry, BotHandle, BuildError, HandlerError, HandlerResult, SessionKey,
    ShutdownSignal,
};
use futures_util::future::BoxFuture;
use oxidebot_core::event::kernel::{DispatchEnvelope, DispatchKind};
use oxidebot_core::{
    api::payload::SendMessageTarget,
    event::{tags, EventTag, EventType},
    source::message::MessageSegment,
    BotIdentity, BotObject, PlatformId,
};
use std::{
    future::Future,
    marker::PhantomData,
    panic::{catch_unwind, AssertUnwindSafe},
    str::FromStr,
    sync::Arc,
};

/// Type-level borrowed view over one event category.
#[doc(hidden)]
pub trait EventView: Send + Sync + 'static {
    type Target: Send + Sync + 'static + ?Sized;
    const KIND: DispatchKind;
    fn from_envelope(envelope: &DispatchEnvelope) -> Option<&Self::Target>;
}

/// A typed handler context. The event value is borrowed from the single shared
/// dispatch allocation and is never cloned per handler.
pub struct Context<E, S = ()>
where
    E: EventView,
    S: Send + Sync + 'static,
{
    envelope: Arc<DispatchEnvelope>,
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
    pub(crate) fn from_parts(
        envelope: Arc<DispatchEnvelope>,
        state: Arc<S>,
        bot: BotHandle,
        sessions: SessionRegistry,
        shutdown: ShutdownSignal,
    ) -> Self {
        Self {
            envelope,
            state,
            bot,
            sessions,
            shutdown,
            _event: PhantomData,
        }
    }

    #[must_use]
    pub fn event(&self) -> &E::Target {
        E::from_envelope(&self.envelope)
            .expect("compiled route and event selector must describe the same event")
    }

    #[must_use]
    pub fn state(&self) -> &S {
        &self.state
    }

    /// Returns the bot's single OxideBot API implementation.
    #[must_use]
    pub fn bot(&self) -> BotObject {
        self.bot
            .api()
            .expect("adapters are constructed from a CallApiTrait implementation")
    }

    /// Returns the stable identity of the bot handling this event.
    #[must_use]
    pub fn bot_identity(&self) -> &BotIdentity {
        self.bot.identity()
    }

    #[must_use]
    pub fn shutdown(&self) -> &ShutdownSignal {
        &self.shutdown
    }
}

/// Type-level selector for one stable [`EventType`].
#[derive(Clone, Copy, Debug, Default)]
#[doc(hidden)]
pub struct TaggedEvent<T>(PhantomData<fn() -> T>);

impl<T> EventView for TaggedEvent<T>
where
    T: EventTag,
{
    type Target = T::Event;
    const KIND: DispatchKind = T::TYPE.dispatch_kind();

    fn from_envelope(envelope: &DispatchEnvelope) -> Option<&Self::Target> {
        envelope.event_as::<T>()
    }
}

/// Handler context for a compile-time event tag.
pub type EventContext<T, S = ()> = Context<TaggedEvent<T>, S>;
/// Common message-event context.
pub type MessageContext<S = ()> = EventContext<tags::Message, S>;

impl<S> Context<TaggedEvent<tags::Message>, S>
where
    S: Send + Sync + 'static,
{
    #[must_use]
    pub fn text(&self) -> String {
        self.event().message.get_raw_text()
    }

    pub async fn send(
        &self,
        message: Vec<MessageSegment>,
    ) -> Result<Vec<oxidebot_core::api::response::SendMessageResponse>, HandlerError> {
        let target = self
            .event()
            .group
            .as_ref()
            .map(|group| SendMessageTarget::Group(group.id.clone()))
            .unwrap_or_else(|| SendMessageTarget::Private(self.event().sender.id.clone()));
        self.bot
            .api()?
            .send_message(message, target)
            .await
            .map_err(|error| HandlerError::Api(error.to_string()))
    }

    pub async fn reply(
        &self,
        mut message: Vec<MessageSegment>,
    ) -> Result<Vec<oxidebot_core::api::response::SendMessageResponse>, HandlerError> {
        message.push(MessageSegment::reply(self.event().message.id.clone()));
        self.send(message).await
    }

    pub async fn ask_parse<T>(
        &self,
        prompt: Vec<MessageSegment>,
        options: crate::AskOptions,
    ) -> Result<T, HandlerError>
    where
        T: FromStr,
        T::Err: std::fmt::Display,
    {
        let conversation = self
            .envelope
            .index
            .conversation
            .clone()
            .ok_or_else(|| HandlerError::Parse("message has no conversation key".into()))?;
        let actor = self
            .envelope
            .index
            .actor
            .clone()
            .ok_or_else(|| HandlerError::Parse("message has no actor key".into()))?;
        let key = SessionKey::new(conversation, actor, options.namespace.clone());
        let waiter = self
            .sessions
            .register(key, options.timeout, options.policy)
            .await?;
        self.send(prompt).await?;
        let response = waiter.wait().await?;
        let text = match response.event() {
            oxidebot_core::Event::MessageEvent(event) => event.message.get_raw_text(),
            _ => {
                return Err(HandlerError::Parse(
                    "session response is not a message event".into(),
                ))
            }
        };
        text.parse()
            .map_err(|error: T::Err| HandlerError::Parse(error.to_string()))
    }
}

/// Fully compiled route category used by the router candidate indexes.
#[doc(hidden)]
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum RouteSpec {
    Event(EventType),
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
    #[must_use]
    pub const fn global() -> Self {
        Self {
            platform: None,
            bot: None,
        }
    }

    #[must_use]
    pub fn for_platform(platform: PlatformId) -> Self {
        Self {
            platform: Some(platform),
            bot: None,
        }
    }

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

/// A typed route matcher compiled once during application construction.
pub trait Matcher: Send + Sync + 'static {
    type Event: EventView;
    fn route_spec(&self) -> RouteSpec;
}

/// Matcher for one event tag.
#[derive(Clone, Copy, Debug, Default)]
pub struct EventMatcher<T>(PhantomData<fn() -> T>);

impl<T> Matcher for EventMatcher<T>
where
    T: EventTag,
{
    type Event = TaggedEvent<T>;

    fn route_spec(&self) -> RouteSpec {
        RouteSpec::Event(T::TYPE)
    }
}

/// Selects one event subtype through a compile-time tag.
#[must_use]
pub const fn event<T>() -> EventMatcher<T>
where
    T: EventTag,
{
    EventMatcher(PhantomData)
}

/// Matcher for ordinary message events, optionally indexed by command.
#[derive(Clone, Debug, Default)]
pub struct MessageMatcher {
    command: Option<Arc<str>>,
}

#[must_use]
pub fn message() -> MessageMatcher {
    MessageMatcher::default()
}

impl MessageMatcher {
    #[must_use]
    pub fn command(mut self, value: impl Into<Arc<str>>) -> Self {
        self.command = Some(value.into());
        self
    }
}

impl Matcher for MessageMatcher {
    type Event = TaggedEvent<tags::Message>;

    fn route_spec(&self) -> RouteSpec {
        self.command
            .as_ref()
            .map_or(RouteSpec::Event(EventType::Message), |command| {
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
    type Event = TaggedEvent<tags::Interaction>;

    fn route_spec(&self) -> RouteSpec {
        RouteSpec::Interaction(self.custom_id.clone())
    }
}

/// Matcher for one platform-native lifecycle event identifier.
#[derive(Clone, Debug)]
pub struct NativeMatcher {
    kind: Arc<str>,
}

#[must_use]
pub fn native(kind: impl Into<Arc<str>>) -> NativeMatcher {
    NativeMatcher { kind: kind.into() }
}

impl Matcher for NativeMatcher {
    type Event = TaggedEvent<tags::PlatformNative>;

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
    #[must_use]
    pub fn platform(mut self, platform: PlatformId) -> Self {
        self.scope = RouteScope::for_platform(platform);
        self
    }

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
    fn event_kind(&self) -> DispatchKind;
    fn call(
        &self,
        event: Arc<DispatchEnvelope>,
        state: Arc<S>,
        bot: BotHandle,
        sessions: SessionRegistry,
        shutdown: ShutdownSignal,
    ) -> BoxFuture<'static, HandlerResult>;
}

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

    fn event_kind(&self) -> DispatchKind {
        M::Event::KIND
    }

    fn call(
        &self,
        event: Arc<DispatchEnvelope>,
        state: Arc<S>,
        bot: BotHandle,
        sessions: SessionRegistry,
        shutdown: ShutdownSignal,
    ) -> BoxFuture<'static, HandlerResult> {
        debug_assert!(M::Event::from_envelope(&event).is_some());
        Box::pin((self.function)(Context {
            envelope: event,
            state,
            bot,
            sessions,
            shutdown,
            _event: PhantomData,
        }))
    }
}

pub(crate) fn erase_handler<S, H>(handler: H) -> Arc<dyn ErasedHandler<S>>
where
    S: Send + Sync + 'static,
    H: Handler<S>,
{
    Arc::new(handler)
}

pub(crate) struct PreparedHandler<S>
where
    S: Send + Sync + 'static,
{
    pub(crate) spec: RouteSpec,
    pub(crate) scope: RouteScope,
    pub(crate) event_kind: DispatchKind,
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
