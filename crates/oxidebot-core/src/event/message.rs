use crate::{
    conversation::ConversationRef,
    source::{message::Message, user::User},
};
use chrono::{DateTime, Utc};

/// A received message event. Events are data; sending and mutation live on the
/// runtime context and the canonical adapter API.
#[derive(Debug, Clone, PartialEq)]
pub struct MessageEvent {
    /// Platform event identifier.
    pub id: String,
    /// Time reported by the platform.
    pub time: Option<DateTime<Utc>>,
    /// User that sent the message.
    pub sender: User,
    /// Conversation in which the message was received.
    pub conversation: ConversationRef,
    /// Normalized portable message content.
    pub message: Message,
}
