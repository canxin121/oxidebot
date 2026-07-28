use crate::{
    authoring::{BoundDeliveryPipeline, ErasedDeliveryPipeline},
    session::SessionRegistry,
    BotHandle, BuildError, Context, HandlerError, HandlerResult, Outcome, SessionKey,
    ShutdownSignal,
};
use futures_util::future::BoxFuture;
use oxidebot_core::event::kernel::{DispatchEnvelope, DispatchKind};
use oxidebot_core::{
    conversation::{ConversationRef, MessageTarget},
    event::{tags, EventTag, EventType},
    source::message::{DeliveryReport, FallbackPolicy, Message, MessageSegment},
    BotIdentity, BotObject, EventId, PlatformId,
};
use std::{
    marker::PhantomData,
    ops::Deref,
    panic::{catch_unwind, AssertUnwindSafe},
    str::FromStr,
    sync::Arc,
};

/// A strongly typed, clone-cheap view of one shared event.
///
/// The application state is intentionally not hidden inside this value. A
/// handler that needs application state asks for `State<S>` separately, while
/// `EventContext<T>` provides the complete public event payload and Bot runtime handles.
pub struct EventContext<T>
where
    T: EventTag,
{
    envelope: Arc<DispatchEnvelope>,
    bot: BotHandle,
    sessions: SessionRegistry,
    shutdown: ShutdownSignal,
    pipeline: Option<Arc<dyn ErasedDeliveryPipeline>>,
    _tag: PhantomData<fn() -> T>,
}

impl<T> EventContext<T>
where
    T: EventTag,
{
    pub(crate) fn from_context<S>(context: &Context<S>) -> Option<Self>
    where
        S: Send + Sync + 'static,
    {
        T::get(context.event())?;
        Some(Self {
            envelope: Arc::clone(&context.envelope),
            bot: context.bot.clone(),
            sessions: context.sessions.clone(),
            shutdown: context.shutdown.clone(),
            pipeline: Some(Arc::new(BoundDeliveryPipeline {
                runtime: context.authoring_arc(),
                context: context.clone(),
            })),
            _tag: PhantomData,
        })
    }

    #[must_use]
    pub fn event(&self) -> &T::Event {
        T::get(self.envelope.event())
            .expect("compiled handler and event tag must describe the same event")
    }

    #[must_use]
    pub fn event_id(&self) -> &EventId {
        &self.envelope.id
    }

    /// Returns the bot's complete unified API implementation.
    pub fn bot(&self) -> Result<BotObject, HandlerError> {
        self.bot.api().map_err(HandlerError::from)
    }

    #[must_use]
    pub fn bot_identity(&self) -> &BotIdentity {
        self.bot.identity()
    }

    #[must_use]
    pub fn shutdown(&self) -> &ShutdownSignal {
        &self.shutdown
    }

    #[doc(hidden)]
    #[must_use]
    pub fn dispatch_envelope(&self) -> &DispatchEnvelope {
        &self.envelope
    }
}

impl<T> Clone for EventContext<T>
where
    T: EventTag,
{
    fn clone(&self) -> Self {
        Self {
            envelope: Arc::clone(&self.envelope),
            bot: self.bot.clone(),
            sessions: self.sessions.clone(),
            shutdown: self.shutdown.clone(),
            pipeline: self.pipeline.clone(),
            _tag: PhantomData,
        }
    }
}

impl<T> Deref for EventContext<T>
where
    T: EventTag,
{
    type Target = T::Event;

    fn deref(&self) -> &Self::Target {
        self.event()
    }
}

impl<T> std::fmt::Debug for EventContext<T>
where
    T: EventTag,
{
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EventContext")
            .field("event_id", &self.envelope.id)
            .field("event_type", &T::TYPE)
            .field("bot", self.bot.identity())
            .finish_non_exhaustive()
    }
}

/// Common message-event context.
pub type MessageContext = EventContext<tags::Message>;

impl EventContext<tags::Message> {
    #[must_use]
    pub fn text(&self) -> String {
        self.event().message.get_raw_text()
    }

    pub async fn send(&self, message: impl Into<Message>) -> Result<DeliveryReport, HandlerError> {
        let target = self
            .event()
            .group
            .as_ref()
            .map(|group| MessageTarget::new(ConversationRef::group(group.id.clone())))
            .unwrap_or_else(|| {
                MessageTarget::new(ConversationRef::direct(self.event().sender.id.clone()))
            });
        let api = self.bot()?;
        if let Some(pipeline) = &self.pipeline {
            pipeline
                .deliver(&api, target, message.into(), FallbackPolicy::Auto)
                .await
        } else {
            api.send_outgoing_message_with(target, message.into(), FallbackPolicy::Auto)
                .await
                .map_err(|error| HandlerError::Api(error.to_string()))
        }
    }

    pub async fn reply(&self, message: impl Into<Message>) -> Result<DeliveryReport, HandlerError> {
        self.send(message.into().reply_to(self.event().message.id.clone()))
            .await
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

/// Fully compiled handler category used by the candidate indexes.
#[doc(hidden)]
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum RouteSpec {
    Event(EventType),
    Command(Arc<str>),
    Interaction(Arc<str>),
    Native(Arc<str>),
}

/// Optional adapter scope attached to one compiled handler.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct RouteScope {
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

    pub(crate) fn is_global(&self) -> bool {
        self.platform.is_none() && self.bot.is_none()
    }

    pub(crate) fn overlaps(&self, other: &Self) -> bool {
        self.intersect(other).is_ok()
    }

    pub(crate) fn intersect(&self, other: &Self) -> Result<Self, String> {
        match (&self.bot, &other.bot) {
            (Some(left), Some(right)) if left != right => {
                return Err(format!(
                    "module scopes target different bots: {}:{} and {}:{}",
                    left.platform, left.bot, right.platform, right.bot
                ));
            }
            _ => {}
        }

        let bot = self.bot.as_ref().or(other.bot.as_ref()).cloned();
        let platform = match (&self.platform, &other.platform) {
            (Some(left), Some(right)) if left != right => {
                return Err(format!(
                    "module scopes target different platforms: {left} and {right}"
                ));
            }
            (Some(platform), _) | (_, Some(platform)) => Some(platform.clone()),
            (None, None) => None,
        };

        if let (Some(platform), Some(bot)) = (&platform, &bot) {
            if platform != &bot.platform {
                return Err(format!(
                    "module platform scope {platform} does not contain bot {}:{}",
                    bot.platform, bot.bot
                ));
            }
        }

        Ok(Self { platform, bot })
    }
}

#[doc(hidden)]
pub(crate) struct HandlerCall<S>
where
    S: Send + Sync + 'static,
{
    pub(crate) event: Arc<DispatchEnvelope>,
    pub(crate) state: Arc<S>,
    pub(crate) bot: BotHandle,
    pub(crate) sessions: SessionRegistry,
    pub(crate) shutdown: ShutdownSignal,
    pub(crate) authoring: Arc<crate::authoring::AuthoringRuntime<S>>,
    pub(crate) command_input: Option<Arc<tokio::sync::OnceCell<crate::RewriteInput>>>,
}

#[doc(hidden)]
pub(crate) trait ErasedHandler<S>: Send + Sync + 'static
where
    S: Send + Sync + 'static,
{
    fn route_spec(&self) -> RouteSpec;
    fn route_scope(&self) -> RouteScope;
    fn event_kind(&self) -> DispatchKind;
    fn default_block(&self) -> bool;
    fn command_id(&self) -> Option<crate::CommandId>;
    fn call(&self, call: HandlerCall<S>) -> BoxFuture<'static, HandlerResult<Outcome>>;
}

pub(crate) struct PreparedHandler<S>
where
    S: Send + Sync + 'static,
{
    pub(crate) spec: RouteSpec,
    pub(crate) scope: RouteScope,
    pub(crate) event_kind: DispatchKind,
    pub(crate) default_block: bool,
    pub(crate) command_id: Option<crate::CommandId>,
    pub(crate) handler: Arc<dyn ErasedHandler<S>>,
}

pub(crate) fn prepare_handler<S>(
    handler: Arc<dyn ErasedHandler<S>>,
) -> Result<PreparedHandler<S>, BuildError>
where
    S: Send + Sync + 'static,
{
    let (spec, scope, event_kind, default_block, command_id) =
        catch_unwind(AssertUnwindSafe(|| {
            (
                handler.route_spec(),
                handler.route_scope(),
                handler.event_kind(),
                handler.default_block(),
                handler.command_id(),
            )
        }))
        .map_err(|_| BuildError::InvalidRoute("handler routing metadata panicked".into()))?;
    Ok(PreparedHandler {
        spec,
        scope,
        event_kind,
        default_block,
        command_id,
        handler,
    })
}
