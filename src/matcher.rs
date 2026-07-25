use std::sync::Arc;

use crate::{
    api::SendMessageResponse,
    bot::BotObject,
    event::{self, Event, EventObject},
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
            Event::NoticeEvent(_) | Event::RequestEvent(_) => self
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
            _ => Err(anyhow::anyhow!("Other Event not support")),
        }
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
        Event::RequestEvent(_) | Event::MetaEvent(_) | Event::AnyEvent(_) => None,
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
