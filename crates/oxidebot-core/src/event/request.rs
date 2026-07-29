use crate::{conversation::ConversationRef, source::user::User};

#[derive(Clone, Debug, PartialEq)]
pub enum RequestEvent {
    Friend(FriendRequest),
    GroupJoin(GroupJoinRequest),
    GroupInvite(GroupInviteRequest),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestDecision {
    Approve,
    Decline,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FriendRequest {
    pub id: String,
    pub user: User,
    pub message: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GroupJoinRequest {
    pub id: String,
    pub user: User,
    pub conversation: ConversationRef,
    pub message: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GroupInviteRequest {
    pub id: String,
    pub user: User,
    pub conversation: ConversationRef,
    pub message: Option<String>,
}
