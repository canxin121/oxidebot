use std::sync::Arc;

use crate::{
    api::SendMessageResponse,
    bot::BotObject,
    content::{OutgoingMessage, ReplyOptions},
    conversation::{ConversationKind, ConversationRef, MessageRef, MessageTarget},
    event::{self, Event, EventObject},
    interaction::{InteractionResponse, MessageOptions},
    source::{
        group::Group,
        message::{Message, MessageSegment},
        user::User,
    },
};

use anyhow::Result;

/// Matcher is a struct that contains the eventObject, event and the bot.
/// It implements some methods to get the user, message, group and so on.
#[derive(Clone, Debug)]
pub struct Matcher {
    pub event_object: EventObject,
    pub event: Arc<Event>,
    pub bot: BotObject,
}

impl Matcher {
    pub fn new(event_object: EventObject, bot: BotObject) -> Vec<Self> {
        let event = event_object.get_events();
        let mut matchers = Vec::new();
        for event in event.into_iter() {
            matchers.push(Self {
                event_object: event_object.clone(),
                event: Arc::new(event),
                bot: bot.clone(),
            });
        }
        matchers
    }

    pub fn try_get_user(&self) -> Option<&User> {
        event_user(self.event.as_ref())
    }

    pub fn try_get_message(&self) -> Option<&Message> {
        event_message(self.event.as_ref())
    }

    pub fn try_get_group(&self) -> Option<&Group> {
        event_group(self.event.as_ref())
    }

    /// Returns the delivery envelopes emitted by the underlying adapter.
    /// This parallel accessor preserves the public 0.1 `Matcher` layout while
    /// exposing stable update IDs, timestamps, retries, and raw payloads.
    pub fn event_envelopes(&self) -> Vec<crate::event::EventEnvelope> {
        self.event_object.get_event_envelopes()
    }

    /// Returns the portable conversation address for legacy and v2 events.
    pub fn try_get_conversation(&self) -> Option<ConversationRef> {
        event_conversation(self.event.as_ref())
    }

    /// Sends a v2 message to the conversation that produced this event.
    pub async fn try_send_outgoing_message(
        &self,
        message: OutgoingMessage,
    ) -> Result<Vec<MessageRef>> {
        let conversation = self
            .try_get_conversation()
            .ok_or_else(|| anyhow::anyhow!("this event has no portable message destination"))?;
        self.bot
            .send_outgoing_message(MessageTarget::new(conversation), message)
            .await
    }

    /// Replies to the message that produced this event through the v2 API.
    pub async fn try_reply_outgoing_message(
        &self,
        mut message: OutgoingMessage,
    ) -> Result<Vec<MessageRef>> {
        anyhow::ensure!(
            message.options.reply.is_none(),
            "outgoing message already contains reply options"
        );
        let reference = event_message_ref(self.event.as_ref())
            .ok_or_else(|| anyhow::anyhow!("this event has no portable message to reply to"))?;
        let conversation = reference
            .conversation
            .clone()
            .or_else(|| self.try_get_conversation())
            .ok_or_else(|| anyhow::anyhow!("this event has no portable message destination"))?;
        message.options.reply = Some(ReplyOptions::new(reference));
        self.bot
            .send_outgoing_message(MessageTarget::new(conversation), message)
            .await
    }

    pub async fn is_related_to_bot(&self) -> bool {
        if let Some(bot_id) = self.bot.bot_info().await.id {
            self.is_related_to_user(&bot_id)
        } else {
            tracing::error!("Failed to get bot id.");
            false
        }
    }

    pub fn is_related_to_user(&self, user_id: &str) -> bool {
        match self.event.as_ref() {
            Event::MessageEvent(event) => event.message.is_related_to_user(user_id),
            Event::NoticeEvent(_) | Event::RequestEvent(_) | Event::InteractionEvent(_) => self
                .try_get_user()
                .map(|user| user.id == user_id)
                .unwrap_or(false),
            _ => false,
        }
    }

    pub async fn try_send_message(
        &self,
        message: Vec<MessageSegment>,
    ) -> Result<Vec<SendMessageResponse>> {
        match self.event.as_ref() {
            Event::MessageEvent(event) => match event.group.as_ref() {
                Some(group) => {
                    self.bot
                        .send_message(
                            message,
                            crate::api::payload::SendMessageTarget::Group(group.id.clone()),
                        )
                        .await
                }
                None => {
                    self.bot
                        .send_message(
                            message,
                            crate::api::payload::SendMessageTarget::Private(
                                event.sender.id.clone(),
                            ),
                        )
                        .await
                }
            },
            Event::NoticeEvent(event) => event.send_message(self.bot.clone(), message).await,
            Event::RequestEvent(event) => match event {
                event::RequestEvent::GroupAddEvent(event) => {
                    self.bot
                        .send_message(
                            message,
                            crate::api::payload::SendMessageTarget::Group(event.group.id.clone()),
                        )
                        .await
                }
                _ => Err(anyhow::anyhow!("Other RequestEvent not support")),
            },
            Event::InteractionEvent(event) => match event.group.as_ref() {
                Some(group) => {
                    self.bot
                        .send_message(
                            message,
                            crate::api::payload::SendMessageTarget::Group(group.id.clone()),
                        )
                        .await
                }
                None => {
                    self.bot
                        .send_message(
                            message,
                            crate::api::payload::SendMessageTarget::Private(event.user.id.clone()),
                        )
                        .await
                }
            },
            _ => Err(anyhow::anyhow!("Other Event not support")),
        }
    }

    /// Sends a message with interactive components to the conversation that
    /// produced this event.
    pub async fn try_send_message_with_options(
        &self,
        message: Vec<MessageSegment>,
        options: MessageOptions,
    ) -> Result<Vec<SendMessageResponse>> {
        let target = match self.event.as_ref() {
            Event::MessageEvent(event) => event
                .group
                .as_ref()
                .map(|group| crate::api::payload::SendMessageTarget::Group(group.id.clone()))
                .unwrap_or_else(|| {
                    crate::api::payload::SendMessageTarget::Private(event.sender.id.clone())
                }),
            Event::NoticeEvent(_) => {
                if let Some(group) = self.try_get_group() {
                    crate::api::payload::SendMessageTarget::Group(group.id.clone())
                } else if let Some(user) = self.try_get_user() {
                    crate::api::payload::SendMessageTarget::Private(user.id.clone())
                } else {
                    return Err(anyhow::anyhow!(
                        "this NoticeEvent has no message destination"
                    ));
                }
            }
            Event::RequestEvent(event::RequestEvent::GroupAddEvent(event)) => {
                crate::api::payload::SendMessageTarget::Group(event.group.id.clone())
            }
            Event::InteractionEvent(event) => event
                .group
                .as_ref()
                .map(|group| crate::api::payload::SendMessageTarget::Group(group.id.clone()))
                .unwrap_or_else(|| {
                    crate::api::payload::SendMessageTarget::Private(event.user.id.clone())
                }),
            _ => return Err(anyhow::anyhow!("this event has no message destination")),
        };
        self.bot
            .send_message_with_options(message, target, options)
            .await
    }

    /// Answers the current interaction event.
    pub async fn try_answer_interaction(&self, response: InteractionResponse) -> Result<()> {
        let Event::InteractionEvent(event) = self.event.as_ref() else {
            return Err(anyhow::anyhow!("current event is not an interaction"));
        };
        self.bot
            .answer_interaction(event.id.clone(), response)
            .await
    }

    pub async fn try_reply_message(
        &self,
        message: Vec<MessageSegment>,
    ) -> Result<Vec<SendMessageResponse>> {
        let message_id = self
            .try_get_message()
            .ok_or(anyhow::anyhow!("No message"))?
            .id
            .clone();
        let mut message = message;
        message.push(MessageSegment::reply(message_id));
        self.try_send_message(message).await
    }

    pub async fn try_reply_message_with_options(
        &self,
        message: Vec<MessageSegment>,
        options: MessageOptions,
    ) -> Result<Vec<SendMessageResponse>> {
        let message_id = self
            .try_get_message()
            .ok_or(anyhow::anyhow!("No message"))?
            .id
            .clone();
        let mut message = message;
        message.push(MessageSegment::reply(message_id));
        self.try_send_message_with_options(message, options).await
    }

    pub async fn try_delete_msg(&self) -> Result<()> {
        let message_id = self
            .try_get_message()
            .ok_or(anyhow::anyhow!("No message"))?
            .id
            .clone();
        self.bot.delete_message(message_id).await
    }

    pub async fn is_group(&self) -> bool {
        self.try_get_group().is_some()
    }

    pub async fn is_private(&self) -> bool {
        self.try_get_group().is_none()
    }
}

fn event_user(event: &Event) -> Option<&User> {
    match event {
        Event::MessageEvent(event) => Some(&event.sender),
        Event::NoticeEvent(event) => match event {
            event::NoticeEvent::GroupMemberIncreseEvent(event) => Some(&event.user),
            event::NoticeEvent::GroupMemberDecreaseEvent(event) => Some(&event.user),
            event::NoticeEvent::GroupAdminChangeEvent(event) => Some(&event.user),
            event::NoticeEvent::GroupMuteChangeEvent(_) => None,
            event::NoticeEvent::GroupMemberMuteChangeEvent(event) => Some(&event.user),
            event::NoticeEvent::GroupHightLightChangeEvent(event) => event.sender.as_ref(),
            event::NoticeEvent::GroupMemberAliasChangeEvent(event) => Some(&event.user),
            event::NoticeEvent::MessageReactionsEvent(event) => Some(&event.user),
            event::NoticeEvent::MessageDeletedEvent(event) => event.user.as_ref(),
            event::NoticeEvent::MessageEditedEvent(event) => Some(&event.user),
        },
        Event::RequestEvent(event) => match event {
            event::RequestEvent::FriendAddEvent(event) => Some(&event.user),
            event::RequestEvent::GroupAddEvent(event) => Some(&event.user),
            event::RequestEvent::GroupInviteEvent(event) => Some(&event.user),
        },
        Event::InteractionEvent(event) => Some(&event.user),
        Event::LifecycleEvent(event) => match event {
            event::LifecycleEvent::MessageCreated(event)
            | event::LifecycleEvent::MessageUpdated(event) => event.sender.as_ref(),
            event::LifecycleEvent::ActivityChanged(event) => event.user.as_ref(),
            event::LifecycleEvent::ReactionsChanged { actor, .. } => actor.as_ref(),
            event::LifecycleEvent::MessagePinned(event)
            | event::LifecycleEvent::MessageUnpinned(event) => event.pinned_by.as_ref(),
            event::LifecycleEvent::PollVoteChanged { user, .. } => user.as_ref(),
            event::LifecycleEvent::ChecklistChanged(event) => event.actor.as_ref(),
            event::LifecycleEvent::MemberUpdated { new_member, .. } => Some(&new_member.user),
            event::LifecycleEvent::JoinRequested(event) => Some(&event.user),
            event::LifecycleEvent::PermissionsChanged { user, .. } => user.as_ref(),
            event::LifecycleEvent::FileShared { user, .. } => user.as_ref(),
            event::LifecycleEvent::SuggestionRequested(event) => Some(&event.user),
            event::LifecycleEvent::SuggestionSelected(event) => Some(&event.user),
            event::LifecycleEvent::MiniApp(event) => Some(&event.user),
            event::LifecycleEvent::ShippingRequested(event) => Some(&event.user),
            event::LifecycleEvent::CheckoutRequested(event) => Some(&event.user),
            event::LifecycleEvent::PaymentUpdated(event) => event.payer.as_ref(),
            event::LifecycleEvent::SubscriptionUpdated(event) => Some(&event.user),
            _ => None,
        },
        Event::MetaEvent(_) | Event::AnyEvent(_) => None,
    }
}

fn event_message(event: &Event) -> Option<&Message> {
    match event {
        Event::MessageEvent(event) => Some(&event.message),
        Event::NoticeEvent(event) => match event {
            event::NoticeEvent::GroupHightLightChangeEvent(event) => Some(&event.message),
            event::NoticeEvent::MessageReactionsEvent(event) => Some(&event.message),
            event::NoticeEvent::MessageDeletedEvent(event) => event.message.as_ref(),
            event::NoticeEvent::MessageEditedEvent(event) => event.new_message.as_ref(),
            _ => None,
        },
        Event::InteractionEvent(event) => event.message.as_ref(),
        _ => None,
    }
}

fn event_group(event: &Event) -> Option<&Group> {
    match event {
        Event::MessageEvent(event) => event.group.as_ref(),
        Event::NoticeEvent(event) => match event {
            event::NoticeEvent::GroupMemberIncreseEvent(event) => Some(&event.group),
            event::NoticeEvent::GroupMemberDecreaseEvent(event) => Some(&event.group),
            event::NoticeEvent::GroupAdminChangeEvent(event) => Some(&event.group),
            event::NoticeEvent::GroupMuteChangeEvent(event) => Some(&event.group),
            event::NoticeEvent::GroupMemberMuteChangeEvent(event) => Some(&event.group),
            event::NoticeEvent::GroupHightLightChangeEvent(event) => Some(&event.group),
            event::NoticeEvent::GroupMemberAliasChangeEvent(event) => Some(&event.group),
            event::NoticeEvent::MessageReactionsEvent(event) => event.group.as_ref(),
            event::NoticeEvent::MessageDeletedEvent(event) => event.group.as_ref(),
            event::NoticeEvent::MessageEditedEvent(event) => event.group.as_ref(),
        },
        Event::RequestEvent(event::RequestEvent::GroupAddEvent(event)) => Some(&event.group),
        Event::InteractionEvent(event) => event.group.as_ref(),
        Event::RequestEvent(_)
        | Event::LifecycleEvent(_)
        | Event::MetaEvent(_)
        | Event::AnyEvent(_) => None,
    }
}

fn legacy_conversation(group: Option<&Group>, user: Option<&User>) -> Option<ConversationRef> {
    group
        .map(|group| ConversationRef::new(group.id.clone(), ConversationKind::Group))
        .or_else(|| user.map(|user| ConversationRef::direct(user.id.clone())))
}

fn event_conversation(event: &Event) -> Option<ConversationRef> {
    match event {
        Event::MessageEvent(event) => {
            legacy_conversation(event.group.as_ref(), Some(&event.sender))
        }
        Event::NoticeEvent(_) | Event::RequestEvent(_) | Event::InteractionEvent(_) => {
            legacy_conversation(event_group(event), event_user(event))
        }
        Event::LifecycleEvent(event) => match event {
            event::LifecycleEvent::MessageCreated(event)
            | event::LifecycleEvent::MessageUpdated(event) => event.conversation.clone(),
            event::LifecycleEvent::MessagesDeleted { conversation, .. } => conversation.clone(),
            event::LifecycleEvent::ActivityChanged(event) => Some(event.conversation.clone()),
            event::LifecycleEvent::ReadReceiptUpdated(event) => Some(event.conversation.clone()),
            event::LifecycleEvent::CallUpdated(event) => Some(event.conversation.clone()),
            event::LifecycleEvent::ReactionsChanged { message, .. }
            | event::LifecycleEvent::ScheduledMessageSent(message) => message.conversation.clone(),
            event::LifecycleEvent::ChecklistChanged(event) => event
                .message
                .as_ref()
                .and_then(|message| message.conversation.clone()),
            event::LifecycleEvent::MessagePinned(event)
            | event::LifecycleEvent::MessageUnpinned(event) => event.message.conversation.clone(),
            event::LifecycleEvent::ThreadCreated(event)
            | event::LifecycleEvent::ThreadUpdated(event)
            | event::LifecycleEvent::ThreadClosed(event)
            | event::LifecycleEvent::ThreadDeleted(event) => Some(event.conversation.clone()),
            event::LifecycleEvent::ConversationCreated { conversation, .. }
            | event::LifecycleEvent::ConversationUpdated { conversation, .. }
            | event::LifecycleEvent::ConversationArchived(conversation)
            | event::LifecycleEvent::MemberUpdated { conversation, .. }
            | event::LifecycleEvent::PermissionsChanged { conversation, .. }
            | event::LifecycleEvent::FileShared { conversation, .. } => Some(conversation.clone()),
            event::LifecycleEvent::JoinRequested(event) => Some(event.conversation.clone()),
            event::LifecycleEvent::FileDeleted { conversation, .. } => conversation.clone(),
            event::LifecycleEvent::SuggestionRequested(event) => event.conversation.clone(),
            event::LifecycleEvent::SuggestionSelected(event) => event.conversation.clone(),
            event::LifecycleEvent::MiniApp(event) => event.conversation.clone(),
            event::LifecycleEvent::PaymentUpdated(event) => event.conversation.clone(),
            _ => None,
        },
        Event::MetaEvent(_) | Event::AnyEvent(_) => None,
    }
}

fn event_message_ref(event: &Event) -> Option<MessageRef> {
    match event {
        Event::MessageEvent(event) => Some(MessageRef::new(event.id.clone()).in_conversation(
            legacy_conversation(event.group.as_ref(), Some(&event.sender))?,
        )),
        Event::InteractionEvent(event) => event.message.as_ref().map(|message| {
            let mut reference = MessageRef::new(message.id.clone());
            reference.conversation = legacy_conversation(event.group.as_ref(), Some(&event.user));
            reference
        }),
        Event::LifecycleEvent(event::LifecycleEvent::MessageCreated(event))
        | Event::LifecycleEvent(event::LifecycleEvent::MessageUpdated(event)) => {
            Some(event.reference.clone())
        }
        Event::LifecycleEvent(event::LifecycleEvent::ReactionsChanged { message, .. })
        | Event::LifecycleEvent(event::LifecycleEvent::ScheduledMessageSent(message)) => {
            Some(message.clone())
        }
        Event::LifecycleEvent(event::LifecycleEvent::MessagePinned(event))
        | Event::LifecycleEvent(event::LifecycleEvent::MessageUnpinned(event)) => {
            Some(event.message.clone())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        event::{
            notice::{GroupMemberIncreseEvent, GroupMemberIncreseReason, NoticeEvent},
            request::{GroupAddEvent, RequestEvent},
        },
        source::{group::Group, user::User},
    };

    use super::*;

    #[test]
    fn member_increase_exposes_its_user_and_group() {
        let event = Event::NoticeEvent(NoticeEvent::GroupMemberIncreseEvent(
            GroupMemberIncreseEvent {
                group: Group {
                    id: "group".to_owned(),
                    ..Default::default()
                },
                user: User {
                    id: "user".to_owned(),
                    ..Default::default()
                },
                reason: GroupMemberIncreseReason::Unknown,
            },
        ));
        assert_eq!(event_user(&event).unwrap().id, "user");
        assert_eq!(event_group(&event).unwrap().id, "group");
    }

    #[test]
    fn group_join_request_exposes_its_user_and_group() {
        let event = Event::RequestEvent(RequestEvent::GroupAddEvent(GroupAddEvent {
            id: "request".to_owned(),
            user: User {
                id: "user".to_owned(),
                ..Default::default()
            },
            group: Group {
                id: "group".to_owned(),
                ..Default::default()
            },
            message: None,
        }));
        assert_eq!(event_user(&event).unwrap().id, "user");
        assert_eq!(event_group(&event).unwrap().id, "group");
    }
}
