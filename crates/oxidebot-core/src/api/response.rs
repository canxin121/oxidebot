use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::source::{
    group::{Group, GroupProfile},
    message::{FsNode, MessageSegment},
    user::{User, UserProfile},
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SendMessageResponse {
    pub sent_message_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GetMessageDetailResponse {
    pub message: Vec<MessageSegment>,
    pub sender: Option<User>,
    pub time: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GroupMemberListResponse {
    pub members: Vec<User>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GroupGetProfileResponse {
    pub profile: GroupProfile,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GroupGetFileCountResponse {
    pub count: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GroupGetFsListResponse {
    pub fs_tree: Vec<FsNode>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UserGetProfileResponse {
    pub profile: UserProfile,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BotGetProfileResponse {
    pub profile: UserProfile,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BotGetFriendListResponse {
    pub friends: Vec<User>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BotGetGroupListResponse {
    pub groups: Vec<Group>,
}
