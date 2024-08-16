use crate::{
    api::payload::RequestResponse,
    bot::BotObject,
    source::{group::Group, user::User},
};
use anyhow::Result;

#[derive(Clone, Debug, PartialEq)]
pub enum RequestEvent {         
    FriendAddEvent(FriendAddEvent),
    GroupAddEvent(GroupAddEvent),
    GroupInviteEvent(GroupInviteEvent),
}

#[derive(Clone, Debug, PartialEq)]
pub struct FriendAddEvent {
    pub id: String,
    pub user: User,
    pub message: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GroupAddEvent {
    pub id: String,
    pub user: User,
    pub group: Group,
    pub message: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GroupInviteEvent {
    pub id: String,
    pub user: User,
    pub group_id: String,
    pub message: Option<String>,
}

impl RequestEvent {
    pub async fn approve(&self, bot: BotObject) -> Result<()> {
        match self {
            RequestEvent::FriendAddEvent(FriendAddEvent { id, .. }) => {
                bot.handle_add_friend_request(id.clone(), RequestResponse::Approve)
                    .await
            }
            RequestEvent::GroupAddEvent(GroupAddEvent { id, .. }) => {
                bot.handle_add_group_request(id.to_string(), RequestResponse::Approve)
                    .await
            }
            RequestEvent::GroupInviteEvent(GroupInviteEvent { id, .. }) => {
                bot.handle_invite_group_request(id.to_string(), RequestResponse::Approve)
                    .await
            }
        }
    }

    pub async fn reject(&self, bot: BotObject) -> Result<()> {
        match self {
            RequestEvent::FriendAddEvent(FriendAddEvent { id, .. }) => {
                bot.handle_add_friend_request(id.clone(), RequestResponse::Reject)
                    .await
            }
            RequestEvent::GroupAddEvent(GroupAddEvent { id, .. }) => {
                bot.handle_add_group_request(id.to_string(), RequestResponse::Reject)
                    .await
            }
            RequestEvent::GroupInviteEvent(GroupInviteEvent { id, .. }) => {
                bot.handle_invite_group_request(id.to_string(), RequestResponse::Reject)
                    .await
            }
        }
    }
}
