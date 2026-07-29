use crate::{
    conversation::ConversationRef,
    source::{message::Message, user::User},
};
use chrono::{DateTime, Utc};

/// A received message event. Events are data; sending and mutation live on the
/// runtime context and the canonical adapter API.
#[derive(Debug, Clone, PartialEq)]
pub struct MessageEvent {
    pub id: String,
    pub time: Option<DateTime<Utc>>,
    pub sender: User,
    pub conversation: ConversationRef,
    pub message: Message,
}
