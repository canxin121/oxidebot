use std::sync::Arc;

use crate::{
    api::SendMessageResponse,
    bot::BotObject,
    event::{
        self,
        notice::{
            GroupAdminChangeEvent, GroupHightLightChangeEvent, GroupMemberAliasChangeEvent,
            GroupMemberDecreaseEvent, GroupMemberIncreseEvent, GroupMemberMuteChangeEvent,
            GroupMuteChangeEvent,
        },
        Event, EventObject,
    },
    source::{
        group::Group,
        message::{Message, MessageSegment},
        user::User,
    },
};

use anyhow::Result;

#[derive(Clone, Debug)]
pub struct Matcher {
    pub event_object: EventObject,
    pub event: Arc<Event>,
    pub bot: BotObject,
}

impl Matcher {
    pub fn new(event_object: EventObject, bot: BotObject) -> Result<Self> {
        let event = Arc::new(event_object.into_event()?);
        Ok(Self {
            event_object,
            event,
            bot,
        })
    }

    pub fn try_get_user(&self) -> Option<&User> {
        match self.event.as_ref() {
            Event::MessageEvent(event) => Some(&event.sender),
            Event::NoticeEvent(event) => match event {
                event::NoticeEvent::GroupAdminChangeEvent(event) => Some(&event.user),
                event::NoticeEvent::GroupMuteChangeEvent(_) => None,
                event::NoticeEvent::GroupMemberMuteChangeEvent(event) => Some(&event.user),
                event::NoticeEvent::GroupHightLightChangeEvent(event) => event.sender.as_ref(),
                event::NoticeEvent::GroupMemberAliasChangeEvent(event) => Some(&event.user),
                event::NoticeEvent::MessageDeletedEvent(event) => Some(&event.user),
                _ => None,
            },
            Event::RequestEvent(event) => match event {
                event::RequestEvent::FriendAddEvent(event) => Some(&event.user),
                event::RequestEvent::GroupAddEvent(_) => None,
                event::RequestEvent::GroupInviteEvent(event) => Some(&event.user),
            },
            _ => None,
        }
    }

    pub fn try_get_message(&self) -> Option<&Message> {
        match self.event.as_ref() {
            Event::MessageEvent(event) => Some(&event.message),
            _ => None,
        }
    }

    pub fn try_get_group(&self) -> Option<&Group> {
        match self.event.as_ref() {
            Event::MessageEvent(event) => event.group.as_ref(),
            Event::NoticeEvent(event) => match event {
                event::NoticeEvent::GroupAdminChangeEvent(event) => Some(&event.group),
                event::NoticeEvent::GroupMuteChangeEvent(event) => Some(&event.group),
                event::NoticeEvent::GroupMemberMuteChangeEvent(event) => Some(&event.group),
                event::NoticeEvent::GroupHightLightChangeEvent(event) => Some(&event.group),
                event::NoticeEvent::GroupMemberAliasChangeEvent(event) => Some(&event.group),
                event::NoticeEvent::MessageDeletedEvent(event) => event.group.as_ref(),
                _ => None,
            },
            Event::RequestEvent(event) => match event {
                event::RequestEvent::FriendAddEvent(_) => None,
                event::RequestEvent::GroupAddEvent(_) => None,
                event::RequestEvent::GroupInviteEvent(_) => None,
            },
            _ => None,
        }
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
            Event::NoticeEvent(event) => match event {
                event::NoticeEvent::GroupAdminChangeEvent(event) => event.user.id == user_id,
                event::NoticeEvent::GroupMuteChangeEvent(_) => false,
                event::NoticeEvent::GroupMemberMuteChangeEvent(event) => event.user.id == user_id,
                event::NoticeEvent::GroupHightLightChangeEvent(event) => event
                    .sender
                    .as_ref()
                    .and_then(|s| Some(s.id == user_id))
                    .unwrap_or(false),
                event::NoticeEvent::GroupMemberAliasChangeEvent(event) => event.user.id == user_id,
                event::NoticeEvent::MessageDeletedEvent(event) => event.user.id == user_id,
                _ => false,
            },
            Event::RequestEvent(event) => match event {
                event::RequestEvent::FriendAddEvent(_) => true,
                event::RequestEvent::GroupAddEvent(_) => false,
                event::RequestEvent::GroupInviteEvent(_) => true,
            },
            _ => false,
        }
    }

    pub async fn try_send_message(
        &self,
        message: Vec<MessageSegment>,
    ) -> Result<SendMessageResponse> {
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
            Event::NoticeEvent(event) => match event {
                crate::event::NoticeEvent::GroupMemberIncreseEvent(GroupMemberIncreseEvent {
                    group: Group { id, .. },
                    ..
                })
                | crate::event::NoticeEvent::GroupMemberDecreaseEvent(GroupMemberDecreaseEvent {
                    group: Group { id, .. },
                    ..
                })
                | crate::event::NoticeEvent::GroupAdminChangeEvent(GroupAdminChangeEvent {
                    group: Group { id, .. },
                    ..
                })
                | crate::event::NoticeEvent::GroupMuteChangeEvent(GroupMuteChangeEvent {
                    group: Group { id, .. },
                    ..
                })
                | crate::event::NoticeEvent::GroupMemberMuteChangeEvent(
                    GroupMemberMuteChangeEvent {
                        group: Group { id, .. },
                        ..
                    },
                )
                | crate::event::NoticeEvent::GroupHightLightChangeEvent(
                    GroupHightLightChangeEvent {
                        group: Group { id, .. },
                        ..
                    },
                )
                | crate::event::NoticeEvent::GroupMemberAliasChangeEvent(
                    GroupMemberAliasChangeEvent {
                        group: Group { id, .. },
                        ..
                    },
                ) => {
                    self.bot
                        .send_message(
                            message,
                            crate::api::payload::SendMessageTarget::Group(id.clone()),
                        )
                        .await
                }
                event::NoticeEvent::MessageDeletedEvent(event) => match event.group.as_ref() {
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
                                    event.user.id.clone(),
                                ),
                            )
                            .await
                    }
                },
            },
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
    ) -> Result<SendMessageResponse> {
        let message_id = self
            .try_get_message()
            .ok_or(anyhow::anyhow!("No message"))?
            .id
            .clone();
        let mut message = message;
        message.push(MessageSegment::reply(message_id));
        self.try_send_message(message).await
    }
}
