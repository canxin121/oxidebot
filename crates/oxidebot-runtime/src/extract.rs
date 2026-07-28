use crate::{
    command::tokenize_segments,
    handler::{Context, TaggedEvent},
    router::reply_target,
    Bot, CommandArgs, CommandParseError, CommandResult, Dialogue, Receipt, Reply, Request,
    Response, ShutdownSignal,
};
use async_trait::async_trait;
use oxidebot_core::{
    api::payload::SendMessageTarget,
    event::{EventTag, NoticeEvent, RequestEvent},
    source::{group::Group, message::MessageSegment, user::User},
    BotIdentity, Event, EventId,
};
use std::{ops::Deref, sync::Arc};

/// Axum-style asynchronous extraction from one matched bot event.
#[async_trait]
pub trait FromRequest<S>: Sized
where
    S: Send + Sync + 'static,
{
    async fn from_request(request: &mut Request<S>) -> Result<Self, Response>;
}

/// Builds a focused state value from the application's root state.
pub trait FromRef<T>: Sized {
    fn from_ref(input: &T) -> Self;
}

impl<T> FromRef<T> for T
where
    T: Clone,
{
    fn from_ref(input: &T) -> Self {
        input.clone()
    }
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageId(pub String);

impl Deref for MessageId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub struct Target(pub SendMessageTarget);

impl Deref for Target {
    type Target = SendMessageTarget;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub struct State<T>(pub T);

impl<T> Deref for State<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub struct Extension<T>(pub T);

impl<T> Deref for Extension<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Strongly typed command arguments.
#[derive(Clone, Debug)]
pub struct Parsed<T>(pub T);

impl<T> Deref for Parsed<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[async_trait]
impl<S> FromRequest<S> for Text
where
    S: Send + Sync + 'static,
{
    async fn from_request(request: &mut Request<S>) -> Result<Self, Response> {
        request
            .message()
            .map(|event| Self(event.message.get_raw_text()))
            .ok_or_else(|| Response::error("this handler requires a message event"))
    }
}

#[async_trait]
impl<S> FromRequest<S> for Segments
where
    S: Send + Sync + 'static,
{
    async fn from_request(request: &mut Request<S>) -> Result<Self, Response> {
        request
            .message()
            .map(|event| Self(event.message.segments.clone()))
            .ok_or_else(|| Response::error("this handler requires a message event"))
    }
}

#[async_trait]
impl<S> FromRequest<S> for Sender
where
    S: Send + Sync + 'static,
{
    async fn from_request(request: &mut Request<S>) -> Result<Self, Response> {
        sender_for_event(request.event())
            .cloned()
            .map(Self)
            .ok_or_else(|| Response::error("this event has no sender"))
    }
}

#[async_trait]
impl<S> FromRequest<S> for ChatGroup
where
    S: Send + Sync + 'static,
{
    async fn from_request(request: &mut Request<S>) -> Result<Self, Response> {
        group_for_event(request.event())
            .cloned()
            .map(Self)
            .ok_or_else(|| Response::error("this event is not scoped to a group"))
    }
}

#[async_trait]
impl<S> FromRequest<S> for MaybeGroup
where
    S: Send + Sync + 'static,
{
    async fn from_request(request: &mut Request<S>) -> Result<Self, Response> {
        Ok(Self(group_for_event(request.event()).cloned()))
    }
}

#[async_trait]
impl<S> FromRequest<S> for MessageId
where
    S: Send + Sync + 'static,
{
    async fn from_request(request: &mut Request<S>) -> Result<Self, Response> {
        request
            .message()
            .map(|event| Self(event.message.id.clone()))
            .ok_or_else(|| Response::error("this event has no message id"))
    }
}

#[async_trait]
impl<S> FromRequest<S> for Target
where
    S: Send + Sync + 'static,
{
    async fn from_request(request: &mut Request<S>) -> Result<Self, Response> {
        reply_target(request.event())
            .map(Self)
            .ok_or_else(|| Response::error("this event has no natural reply target"))
    }
}

#[async_trait]
impl<S, T> FromRequest<S> for State<T>
where
    S: Send + Sync + 'static,
    T: FromRef<S> + Send + 'static,
{
    async fn from_request(request: &mut Request<S>) -> Result<Self, Response> {
        Ok(Self(T::from_ref(request.state())))
    }
}

#[async_trait]
impl<S, T> FromRequest<S> for Extension<T>
where
    S: Send + Sync + 'static,
    T: Clone + Send + Sync + 'static,
{
    async fn from_request(request: &mut Request<S>) -> Result<Self, Response> {
        request
            .extensions()
            .get::<T>()
            .cloned()
            .map(Self)
            .ok_or_else(|| {
                Response::error(format!(
                    "required extension `{}` is missing",
                    std::any::type_name::<T>()
                ))
            })
    }
}

#[async_trait]
impl<S> FromRequest<S> for Bot
where
    S: Send + Sync + 'static,
{
    async fn from_request(request: &mut Request<S>) -> Result<Self, Response> {
        request
            .bot()
            .map(Self)
            .map_err(|error| Response::error(error.to_string()))
    }
}

#[async_trait]
impl<S> FromRequest<S> for Reply
where
    S: Send + Sync + 'static,
{
    async fn from_request(request: &mut Request<S>) -> Result<Self, Response> {
        let api = request
            .bot()
            .map_err(|error| Response::error(error.to_string()))?;
        let target = reply_target(request.event())
            .ok_or_else(|| Response::error("this event has no natural reply target"))?;
        let reply_to = request.message().map(|event| event.message.id.clone());
        Ok(Self::new(api, target, reply_to))
    }
}

#[async_trait]
impl<S> FromRequest<S> for Receipt
where
    S: Send + Sync + 'static,
{
    async fn from_request(request: &mut Request<S>) -> Result<Self, Response> {
        let api = request
            .bot()
            .map_err(|error| Response::error(error.to_string()))?;
        let message_id = request
            .message()
            .map(|event| event.message.id.clone())
            .ok_or_else(|| Response::error("this event has no editable message"))?;
        Ok(Self::from_message_id(api, message_id))
    }
}

#[async_trait]
impl<S> FromRequest<S> for Dialogue
where
    S: Send + Sync + 'static,
{
    async fn from_request(request: &mut Request<S>) -> Result<Self, Response> {
        dialogue_from_request(request).map_err(Response::error)
    }
}

#[async_trait]
impl<S> FromRequest<S> for CommandResult
where
    S: Send + Sync + 'static,
{
    async fn from_request(request: &mut Request<S>) -> Result<Self, Response> {
        request
            .command()
            .cloned()
            .ok_or_else(|| Response::error("this handler requires a command route"))
    }
}

#[async_trait]
impl<S, T> FromRequest<S> for Parsed<T>
where
    S: Send + Sync + 'static,
    T: CommandArgs,
{
    async fn from_request(request: &mut Request<S>) -> Result<Self, Response> {
        let mut result = request
            .command()
            .cloned()
            .ok_or_else(|| Response::error("this handler requires a command route"))?;
        let schema = T::schema();
        let parse = |result: &CommandResult| {
            result
                .parse_with(&schema)
                .and_then(|arguments| T::from_arguments(&arguments))
        };

        let first_error = match parse(&result) {
            Ok(value) => return Ok(Self(value)),
            Err(error) if error.missing_prompt().is_none() => {
                return Err(command_error_response(&result, error));
            }
            Err(error) => error,
        };

        let Some(completion) = result.command().completion_ref().cloned() else {
            return Err(command_error_response(&result, first_error));
        };
        let dialogue = dialogue_from_request(request)
            .map_err(Response::error)?
            .timeout(completion.timeout)
            .namespace(format!("command:{}", result.command().name()))
            .map_err(|error| Response::error(error.to_string()))?;

        for _ in 0..completion.max_rounds {
            let error = match parse(&result) {
                Ok(value) => return Ok(Self(value)),
                Err(error) => error,
            };
            let Some(prompt) = error.missing_prompt() else {
                return Err(command_error_response(&result, error));
            };
            let missing_name = error
                .missing_name()
                .expect("a missing prompt always belongs to a missing argument")
                .to_owned();
            let response = dialogue
                .ask_message(prompt)
                .await
                .map_err(|error| Response::error(error.to_string()))?;
            let Event::MessageEvent(message) = response.event() else {
                return Err(Response::error("command completion requires a message"));
            };
            let raw_text = message.message.get_raw_text();
            if completion.is_cancelled(&raw_text) {
                return Err(command_error_response(
                    &result,
                    CommandParseError::Cancelled,
                ));
            }
            let values = tokenize_segments(&message.message.segments)
                .map_err(|error| command_error_response(&result, error))?;
            result = result.with_answer(&schema, &missing_name, values);
        }

        Err(command_error_response(
            &result,
            CommandParseError::CompletionExhausted,
        ))
    }
}

#[async_trait]
impl<S, T> FromRequest<S> for Option<T>
where
    S: Send + Sync + 'static,
    T: FromRequest<S> + Send,
{
    async fn from_request(request: &mut Request<S>) -> Result<Self, Response> {
        Ok(T::from_request(request).await.ok())
    }
}

#[async_trait]
impl<S, T> FromRequest<S> for Context<TaggedEvent<T>, S>
where
    S: Send + Sync + 'static,
    T: EventTag,
{
    async fn from_request(request: &mut Request<S>) -> Result<Self, Response> {
        if T::get(request.event()).is_none() {
            return Err(Response::error(format!(
                "this handler requires event type {:?}",
                T::TYPE
            )));
        }
        Ok(Context::from_parts(
            Arc::clone(&request.envelope),
            Arc::clone(&request.state),
            request.bot.clone(),
            request.sessions.clone(),
            request.shutdown.clone(),
        ))
    }
}

#[async_trait]
impl<S> FromRequest<S> for BotIdentity
where
    S: Send + Sync + 'static,
{
    async fn from_request(request: &mut Request<S>) -> Result<Self, Response> {
        Ok(request.bot.identity().clone())
    }
}

#[async_trait]
impl<S> FromRequest<S> for EventId
where
    S: Send + Sync + 'static,
{
    async fn from_request(request: &mut Request<S>) -> Result<Self, Response> {
        Ok(request.envelope.id.clone())
    }
}

#[async_trait]
impl<S> FromRequest<S> for ShutdownSignal
where
    S: Send + Sync + 'static,
{
    async fn from_request(request: &mut Request<S>) -> Result<Self, Response> {
        Ok(request.shutdown.clone())
    }
}

fn command_error_response(result: &CommandResult, error: CommandParseError) -> Response {
    Response::error(format!("{error}\n\n用法：{}", result.command().usage()))
}

fn dialogue_from_request<S>(request: &Request<S>) -> Result<Dialogue, String>
where
    S: Send + Sync + 'static,
{
    let conversation = request
        .envelope
        .index
        .conversation
        .clone()
        .ok_or_else(|| "this event has no conversation scope".to_owned())?;
    let actor = request
        .envelope
        .index
        .actor
        .clone()
        .ok_or_else(|| "this event has no actor scope".to_owned())?;
    let target = reply_target(request.event())
        .ok_or_else(|| "this event has no natural reply target".to_owned())?;
    let api = request.bot().map_err(|error| error.to_string())?;
    Ok(Dialogue::new(
        api,
        request.sessions.clone(),
        conversation,
        actor,
        target,
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
