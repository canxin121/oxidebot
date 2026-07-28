use crate::{
    handler::EventContext, router::reply_target, Bot, CommandParseError, CommandResult, Context,
    Dialogue, FromCommandMatch, HandlerError, Outcome, Reply, ShutdownSignal,
};
use oxidebot_core::{
    conversation::MessageTarget,
    event::{EventTag, NoticeEvent, RequestEvent},
    source::{group::Group, message::MessageSegment, user::User},
    BotIdentity, Event, EventId,
};
use std::{fmt, ops::Deref, sync::Arc};

/// A safe, user-facing error produced while extracting a handler argument.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtractError {
    message: Arc<str>,
}

impl ExtractError {
    #[must_use]
    pub fn new(message: impl Into<Arc<str>>) -> Self {
        Self {
            message: message.into(),
        }
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn into_outcome(self) -> Outcome {
        Outcome::new().text(self.message.to_string())
    }
}

impl fmt::Display for ExtractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ExtractError {}

impl From<ExtractError> for HandlerError {
    fn from(error: ExtractError) -> Self {
        Self::user(error.message)
    }
}

/// Extracts one typed value from the common event [`Context`].
///
/// Extraction is synchronous and monomorphized. Interactive command completion
/// is performed once by the command handler before argument extraction, so
/// ordinary message and event handlers do not allocate boxed extractor futures.
pub trait Extract<S>: Sized
where
    S: Send + Sync + 'static,
{
    fn extract(context: &Context<S>) -> Result<Self, ExtractError>;
}

#[derive(Clone, Debug)]
pub struct Text(pub String);

impl Deref for Text {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub struct Segments(pub Vec<MessageSegment>);

impl Deref for Segments {
    type Target = [MessageSegment];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub struct Sender(pub User);

impl Deref for Sender {
    type Target = User;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub struct ChatGroup(pub Group);

impl Deref for ChatGroup {
    type Target = Group;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub struct MaybeGroup(pub Option<Group>);

impl Deref for MaybeGroup {
    type Target = Option<Group>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageId(pub String);

impl Deref for MessageId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub struct Target(pub MessageTarget);

impl Deref for Target {
    type Target = MessageTarget;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// The application's root state.
///
/// Focused application services should be exposed through methods on the root
/// state or through a small custom [`Extract`] implementation. OxideBot does
/// not maintain a Web-style substate conversion registry.
#[derive(Debug)]
pub struct State<S>(pub Arc<S>);

impl<S> Clone for State<S> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<S> Deref for State<S> {
    type Target = S;

    fn deref(&self) -> &Self::Target {
        self.0.as_ref()
    }
}

/// Strongly typed command arguments.
#[derive(Clone, Debug)]
pub struct Args<T>(pub T);

impl<T> Deref for Args<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<S> Extract<S> for Context<S>
where
    S: Send + Sync + 'static,
{
    fn extract(context: &Context<S>) -> Result<Self, ExtractError> {
        Ok(context.clone())
    }
}

impl<S> Extract<S> for Text
where
    S: Send + Sync + 'static,
{
    fn extract(context: &Context<S>) -> Result<Self, ExtractError> {
        context
            .message()
            .map(|event| Self(event.message.get_raw_text()))
            .ok_or_else(|| ExtractError::new("this handler requires a message event"))
    }
}

impl<S> Extract<S> for Segments
where
    S: Send + Sync + 'static,
{
    fn extract(context: &Context<S>) -> Result<Self, ExtractError> {
        context
            .message()
            .map(|event| Self(event.message.segments.clone()))
            .ok_or_else(|| ExtractError::new("this handler requires a message event"))
    }
}

impl<S> Extract<S> for Sender
where
    S: Send + Sync + 'static,
{
    fn extract(context: &Context<S>) -> Result<Self, ExtractError> {
        sender_for_event(context.event())
            .cloned()
            .map(Self)
            .ok_or_else(|| ExtractError::new("this event has no sender"))
    }
}

impl<S> Extract<S> for ChatGroup
where
    S: Send + Sync + 'static,
{
    fn extract(context: &Context<S>) -> Result<Self, ExtractError> {
        group_for_event(context.event())
            .cloned()
            .map(Self)
            .ok_or_else(|| ExtractError::new("this event is not scoped to a group"))
    }
}

impl<S> Extract<S> for MaybeGroup
where
    S: Send + Sync + 'static,
{
    fn extract(context: &Context<S>) -> Result<Self, ExtractError> {
        Ok(Self(group_for_event(context.event()).cloned()))
    }
}

impl<S> Extract<S> for MessageId
where
    S: Send + Sync + 'static,
{
    fn extract(context: &Context<S>) -> Result<Self, ExtractError> {
        context
            .message()
            .map(|event| Self(event.message.id.clone()))
            .ok_or_else(|| ExtractError::new("this event has no message id"))
    }
}

impl<S> Extract<S> for Target
where
    S: Send + Sync + 'static,
{
    fn extract(context: &Context<S>) -> Result<Self, ExtractError> {
        reply_target(context.event())
            .map(Self)
            .ok_or_else(|| ExtractError::new("this event has no natural reply target"))
    }
}

impl<S> Extract<S> for State<S>
where
    S: Send + Sync + 'static,
{
    fn extract(context: &Context<S>) -> Result<Self, ExtractError> {
        Ok(Self(context.state_arc()))
    }
}

impl<S> Extract<S> for Bot
where
    S: Send + Sync + 'static,
{
    fn extract(context: &Context<S>) -> Result<Self, ExtractError> {
        context
            .bot()
            .map(Self)
            .map_err(|_| ExtractError::new("the current adapter does not expose the OxideBot API"))
    }
}

impl<S> Extract<S> for Reply
where
    S: Send + Sync + 'static,
{
    fn extract(context: &Context<S>) -> Result<Self, ExtractError> {
        let api = context.bot().map_err(|_| {
            ExtractError::new("the current adapter does not expose the OxideBot API")
        })?;
        let target = reply_target(context.event())
            .ok_or_else(|| ExtractError::new("this event has no natural reply target"))?;
        let reply_to = context.message().map(|event| event.message.id.clone());
        let pipeline: Arc<dyn crate::authoring::ErasedDeliveryPipeline> =
            Arc::new(crate::authoring::BoundDeliveryPipeline {
                runtime: context.authoring_arc(),
                context: context.clone(),
            });
        Ok(Self::new(api, target, reply_to, Some(pipeline)))
    }
}

impl<S> Extract<S> for Dialogue
where
    S: Send + Sync + 'static,
{
    fn extract(context: &Context<S>) -> Result<Self, ExtractError> {
        dialogue_from_context(context)
    }
}

impl<S> Extract<S> for CommandResult
where
    S: Send + Sync + 'static,
{
    fn extract(context: &Context<S>) -> Result<Self, ExtractError> {
        context
            .command()
            .cloned()
            .ok_or_else(|| ExtractError::new("this handler requires a command"))
    }
}

impl<S, T> Extract<S> for Args<T>
where
    S: Send + Sync + 'static,
    T: FromCommandMatch,
{
    fn extract(context: &Context<S>) -> Result<Self, ExtractError> {
        let result = context
            .command()
            .cloned()
            .ok_or_else(|| ExtractError::new("this handler requires a command"))?;
        T::from_match(&result)
            .map(Self)
            .map_err(|error| command_extract_error(&result, error))
    }
}

impl<S, T> Extract<S> for EventContext<T>
where
    S: Send + Sync + 'static,
    T: EventTag,
{
    fn extract(context: &Context<S>) -> Result<Self, ExtractError> {
        EventContext::from_context(context).ok_or_else(|| {
            ExtractError::new(format!("this handler requires event type {:?}", T::TYPE))
        })
    }
}

impl<S> Extract<S> for BotIdentity
where
    S: Send + Sync + 'static,
{
    fn extract(context: &Context<S>) -> Result<Self, ExtractError> {
        Ok(context.bot_identity().clone())
    }
}

impl<S> Extract<S> for EventId
where
    S: Send + Sync + 'static,
{
    fn extract(context: &Context<S>) -> Result<Self, ExtractError> {
        Ok(context.envelope.id.clone())
    }
}

impl<S> Extract<S> for ShutdownSignal
where
    S: Send + Sync + 'static,
{
    fn extract(context: &Context<S>) -> Result<Self, ExtractError> {
        Ok(context.shutdown.clone())
    }
}

fn command_extract_error(result: &CommandResult, error: CommandParseError) -> ExtractError {
    ExtractError::new(format!("{error}\n\n用法：{}", result.command().usage()))
}

fn dialogue_from_context<S>(context: &Context<S>) -> Result<Dialogue, ExtractError>
where
    S: Send + Sync + 'static,
{
    let conversation = context
        .envelope
        .index
        .conversation
        .clone()
        .ok_or_else(|| ExtractError::new("this event has no conversation scope"))?;
    let actor = context
        .envelope
        .index
        .actor
        .clone()
        .ok_or_else(|| ExtractError::new("this event has no actor scope"))?;
    let target = reply_target(context.event())
        .ok_or_else(|| ExtractError::new("this event has no natural reply target"))?;
    let api = context
        .bot()
        .map_err(|_| ExtractError::new("the current adapter does not expose the OxideBot API"))?;
    Ok(Dialogue::new(
        api,
        context.sessions.clone(),
        conversation,
        actor,
        target,
        Some(Arc::new(crate::authoring::BoundDeliveryPipeline {
            runtime: context.authoring_arc(),
            context: context.clone(),
        })),
    ))
}

fn sender_for_event(event: &Event) -> Option<&User> {
    match event {
        Event::MessageEvent(event) => Some(&event.sender),
        Event::NoticeEvent(event) => match event {
            NoticeEvent::GroupMemberIncreaseEvent(event) => Some(&event.user),
            NoticeEvent::GroupMemberDecreaseEvent(event) => Some(&event.user),
            NoticeEvent::GroupAdminChangeEvent(event) => Some(&event.user),
            NoticeEvent::GroupMuteChangeEvent(event) => event.operator.as_ref(),
            NoticeEvent::GroupMemberMuteChangeEvent(event) => Some(&event.user),
            NoticeEvent::GroupHighlightChangeEvent(event) => {
                event.sender.as_ref().or(event.operator.as_ref())
            }
            NoticeEvent::GroupMemberAliasChangeEvent(event) => Some(&event.user),
            NoticeEvent::MessageReactionsEvent(event) => Some(&event.user),
            NoticeEvent::MessageDeletedEvent(event) => {
                event.user.as_ref().or(event.operator.as_ref())
            }
            NoticeEvent::MessageEditedEvent(event) => Some(&event.user),
        },
        Event::RequestEvent(event) => match event {
            RequestEvent::FriendAddEvent(event) => Some(&event.user),
            RequestEvent::GroupAddEvent(event) => Some(&event.user),
            RequestEvent::GroupInviteEvent(event) => Some(&event.user),
        },
        Event::InteractionEvent(event) => Some(&event.user),
        Event::LifecycleEvent(_) | Event::MetaEvent(_) | Event::AnyEvent(_) => None,
    }
}

fn group_for_event(event: &Event) -> Option<&Group> {
    match event {
        Event::MessageEvent(event) => event.group.as_ref(),
        Event::NoticeEvent(event) => match event {
            NoticeEvent::GroupMemberIncreaseEvent(event) => Some(&event.group),
            NoticeEvent::GroupMemberDecreaseEvent(event) => Some(&event.group),
            NoticeEvent::GroupAdminChangeEvent(event) => Some(&event.group),
            NoticeEvent::GroupMuteChangeEvent(event) => Some(&event.group),
            NoticeEvent::GroupMemberMuteChangeEvent(event) => Some(&event.group),
            NoticeEvent::GroupHighlightChangeEvent(event) => Some(&event.group),
            NoticeEvent::GroupMemberAliasChangeEvent(event) => Some(&event.group),
            NoticeEvent::MessageReactionsEvent(event) => event.group.as_ref(),
            NoticeEvent::MessageDeletedEvent(event) => event.group.as_ref(),
            NoticeEvent::MessageEditedEvent(event) => event.group.as_ref(),
        },
        Event::RequestEvent(RequestEvent::GroupAddEvent(event)) => Some(&event.group),
        Event::InteractionEvent(event) => event.group.as_ref(),
        Event::RequestEvent(
            RequestEvent::FriendAddEvent(_) | RequestEvent::GroupInviteEvent(_),
        )
        | Event::LifecycleEvent(_)
        | Event::MetaEvent(_)
        | Event::AnyEvent(_) => None,
    }
}
