use chrono::{DateTime, Utc};

use crate::source::{
    group::{Group, GroupProfile},
    message::{FsNode, MessageSegment},
    user::{User, UserProfile},
};

#[derive(Debug)]
pub struct SendMessageResponse {
    pub sent_message_id: String,
}

#[derive(Debug)]
pub struct GetMessageDetailResponse {
    pub message: Vec<MessageSegment>,
    pub sender: Option<User>,
    pub time: Option<DateTime<Utc>>,
}

#[derive(Debug)]
pub struct GroupMemberListResponse {
    pub members: Vec<User>,
}

#[derive(Debug)]
pub struct GroupGetProfileResponse {
    pub profile: GroupProfile,
}

#[derive(Debug)]
pub struct GroupGetFileCountResponse {
    pub count: u64,
}

#[derive(Debug)]
pub struct GroupGetFsListResponse {
    pub fs_tree: Vec<FsNode>,
}

#[derive(Debug)]
pub struct UserGetProfileResponse {
    pub profile: UserProfile,
}

#[derive(Debug)]
pub struct BotGetProfileResponse {
    pub profile: User,
}

#[derive(Debug)]
pub struct BotGetFriendListResponse {
    pub friends: Vec<User>,
}

#[derive(Debug)]
pub struct BotGetGroupListResponse {
    pub groups: Vec<Group>,
}
