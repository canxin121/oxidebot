use anyhow::Result;
use std::time::Duration;

use crate::{
    api::response,
    bot::BotObject,
    source::{
        group::Group,
        message::{Message, MessageSegment},
        user::User,
    },
};

#[allow(clippy::large_enum_variant)] // Kept inline for public API compatibility.
#[derive(Debug, Clone, PartialEq)]
pub enum NoticeEvent {
    GroupMemberIncreaseEvent(GroupMemberIncreaseEvent),
    GroupMemberDecreaseEvent(GroupMemberDecreaseEvent),
    GroupAdminChangeEvent(GroupAdminChangeEvent),
    GroupMuteChangeEvent(GroupMuteChangeEvent),
    GroupMemberMuteChangeEvent(GroupMemberMuteChangeEvent),
    GroupHighlightChangeEvent(GroupHighlightChangeEvent),
    GroupMemberAliasChangeEvent(GroupMemberAliasChangeEvent),
    MessageReactionsEvent(MessageReactionsEvent),
    MessageDeletedEvent(MessageDeletedEvent),
    MessageEditedEvent(MessageEditedEvent),
}

impl NoticeEvent {
    pub async fn send_message(
        &self,
        bot: BotObject,
        message: Vec<MessageSegment>,
    ) -> Result<Vec<response::SendMessageResponse>> {
        async fn send_group_message_helper(
            bot: BotObject,
            message: Vec<MessageSegment>,
            group_id: String,
        ) -> Result<Vec<response::SendMessageResponse>> {
            bot.send_message(
                message,
                crate::api::payload::SendMessageTarget::Group(group_id),
            )
            .await
        }
        async fn send_private_message_helper(
            bot: BotObject,
            message: Vec<MessageSegment>,
            user_id: String,
        ) -> Result<Vec<response::SendMessageResponse>> {
            bot.send_message(
                message,
                crate::api::payload::SendMessageTarget::Private(user_id),
            )
            .await
        }
        match self {
            NoticeEvent::GroupAdminChangeEvent(GroupAdminChangeEvent { group, .. })
            | NoticeEvent::GroupHighlightChangeEvent(GroupHighlightChangeEvent { group, .. })
            | NoticeEvent::GroupMemberAliasChangeEvent(GroupMemberAliasChangeEvent {
                group, ..
            })
            | NoticeEvent::GroupMemberIncreaseEvent(GroupMemberIncreaseEvent { group, .. })
            | NoticeEvent::GroupMemberDecreaseEvent(GroupMemberDecreaseEvent { group, .. })
            | NoticeEvent::GroupMemberMuteChangeEvent(GroupMemberMuteChangeEvent {
                group, ..
            }) => send_group_message_helper(bot, message, group.id.clone()).await,
            NoticeEvent::MessageEditedEvent(MessageEditedEvent { user, group, .. })
            | NoticeEvent::MessageReactionsEvent(MessageReactionsEvent { user, group, .. }) => {
                if let Some(group) = group {
                    send_group_message_helper(bot, message, group.id.clone()).await
                } else {
                    send_private_message_helper(bot, message, user.id.clone()).await
                }
            }
            NoticeEvent::GroupMuteChangeEvent(GroupMuteChangeEvent { group, r#type, .. }) => {
                if let MuteType::Mute { .. } = r#type {
                    Err(anyhow::anyhow!("Group is muted, can't send message"))
                } else {
                    send_group_message_helper(bot, message, group.id.clone()).await
                }
            }
            NoticeEvent::MessageDeletedEvent(MessageDeletedEvent { user, group, .. }) => {
                if let Some(group) = group {
                    send_group_message_helper(bot, message, group.id.clone()).await
                } else if let Some(user) = user {
                    send_private_message_helper(bot, message, user.id.clone()).await
                } else {
                    Err(anyhow::anyhow!("Can't send message to unknown user"))
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GroupMemberIncreaseEvent {
    pub group: Group,
    pub user: User,
    pub reason: GroupMemberIncreaseReason,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GroupMemberDecreaseEvent {
    pub group: Group,
    pub user: User,
    pub reason: GroupMemberDecreaseReason,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GroupAdminChangeEvent {
    pub group: Group,
    pub user: User,
    pub r#type: GroupAdminChangeType,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GroupMuteChangeEvent {
    pub group: Group,
    pub operator: Option<User>,
    pub r#type: MuteType,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GroupMemberMuteChangeEvent {
    pub group: Group,
    pub user: User,
    pub operator: Option<User>,
    pub r#type: MuteType,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GroupHighlightChangeEvent {
    pub group: Group,
    pub r#type: GroupHighlightChangeType,
    pub message: Message,
    pub sender: Option<User>,
    pub operator: Option<User>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GroupMemberAliasChangeEvent {
    pub group: Group,
    pub user: User,
    pub operator: Option<User>,
    pub old_alias: Option<String>,
    pub new_alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessageDeletedEvent {
    pub user: Option<User>,
    pub operator: Option<User>,
    pub group: Option<Group>,
    pub message: Option<Message>,
}

#[allow(clippy::large_enum_variant)] // Kept inline for public API compatibility.
#[derive(Debug, Clone, PartialEq)]
pub enum GroupMemberIncreaseReason {
    Approve {
        operator: Option<User>,
    },
    Invite {
        inviter: Option<User>,
        operator: Option<User>,
    },
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GroupMemberDecreaseReason {
    Kick { operator: Option<User> },
    KickMe { operator: Option<User> },
    Leave,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GroupAdminChangeType {
    Set,
    Unset,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GroupHighlightChangeType {
    Set,
    Unset,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MuteType {
    Mute { duration: Option<Duration> },
    UnMute,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessageEditedEvent {
    pub user: User,
    pub group: Option<Group>,
    pub new_message: Option<Message>,
    pub operator: Option<User>,
    pub old_message: Option<Message>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessageReactionsEvent {
    pub user: User,
    pub group: Option<Group>,
    pub message: Message,
    pub reactions: Vec<String>,
}
