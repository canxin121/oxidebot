use crate::{conversation::ConversationRef, source::user::User};

/// A platform request that requires an application decision.
#[derive(Clone, Debug, PartialEq)]
pub enum RequestEvent {
    /// Request to become friends or contacts with the bot.
    Friend(FriendRequest),
    /// Request to join a group conversation.
    GroupJoin(GroupJoinRequest),
    /// Invitation for the bot to join a group conversation.
    GroupInvite(GroupInviteRequest),
}

/// Decision returned for an answerable platform request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestDecision {
    /// Accept the request.
    Approve,
    /// Reject the request.
    Decline,
}

/// A request from a user to become friends or contacts with the bot.
#[derive(Clone, Debug, PartialEq)]
pub struct FriendRequest {
    /// Platform request identifier.
    pub id: String,
    /// User who sent the request.
    pub user: User,
    /// Optional user-supplied request message.
    pub message: Option<String>,
}

/// A request from a user to join a group conversation.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupJoinRequest {
    /// Platform request identifier.
    pub id: String,
    /// User who wants to join.
    pub user: User,
    /// Group conversation targeted by the request.
    pub conversation: ConversationRef,
    /// Optional user-supplied request message.
    pub message: Option<String>,
}

/// An invitation for the bot to join a group conversation.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupInviteRequest {
    /// Platform request identifier.
    pub id: String,
    /// User who sent the invitation.
    pub user: User,
    /// Group conversation that invited the bot.
    pub conversation: ConversationRef,
    /// Optional user-supplied invitation message.
    pub message: Option<String>,
}
