//! Telegram wire types and normalization into OxideBot's canonical event model.

use chrono::{DateTime, Utc};
use oxidebot_core::{
    conversation::{ConversationKind, ConversationRef},
    event::MessageEvent,
    source::{
        message::Message,
        user::{User, UserProfile},
    },
    CallError, Event, EventId,
};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub(crate) fn map_telegram_error<T>(
    status: StatusCode,
    envelope: TelegramEnvelope<T>,
) -> CallError {
    let message = envelope
        .description
        .unwrap_or_else(|| format!("Telegram API HTTP {status}"));
    if status == StatusCode::TOO_MANY_REQUESTS || envelope.error_code == Some(429) {
        return CallError::rate_limited(
            message,
            envelope
                .parameters
                .and_then(|parameters| parameters.retry_after)
                .map(Duration::from_secs),
        );
    }
    match envelope.error_code.unwrap_or(status.as_u16()) {
        400 => CallError::invalid_request(message),
        401 | 403 => CallError::permanent(message),
        404 => CallError::not_found(message),
        value if value >= 500 => CallError::temporary(message),
        _ => CallError::permanent(message),
    }
}

#[derive(Deserialize)]
pub(crate) struct TelegramEnvelope<T> {
    pub(crate) ok: bool,
    pub(crate) result: Option<T>,
    pub(crate) error_code: Option<u16>,
    pub(crate) description: Option<String>,
    pub(crate) parameters: Option<TelegramErrorParameters>,
}
#[derive(Deserialize)]
pub(crate) struct TelegramErrorParameters {
    pub(crate) retry_after: Option<u64>,
}
#[derive(Serialize)]
pub(crate) struct GetUpdatesRequest {
    pub(crate) offset: Option<i64>,
    pub(crate) timeout: u16,
    pub(crate) limit: u8,
}
#[derive(Serialize)]
pub(crate) struct SendMessageRequest<'a> {
    pub(crate) chat_id: &'a str,
    pub(crate) text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) parse_mode: Option<&'a str>,
}
#[derive(Deserialize)]
pub(crate) struct TelegramSentMessage {
    pub(crate) message_id: i64,
}
#[derive(Deserialize)]
pub(crate) struct TelegramUpdate {
    pub(crate) update_id: i64,
    message: Option<TelegramMessage>,
}
impl TelegramUpdate {
    pub(crate) fn into_event(self) -> Option<(EventId, Event)> {
        let message = self.message?;
        let text = message.text?;
        let from = message.from?;
        let conversation = match message.chat.kind.as_deref() {
            Some("private") => ConversationRef::direct(message.chat.id),
            Some("channel") => ConversationRef::new(message.chat.id, ConversationKind::Channel),
            _ => ConversationRef::new(message.chat.id, ConversationKind::Group),
        };
        let event = Event::Message(MessageEvent {
            id: message.message_id.to_string(),
            time: DateTime::<Utc>::from_timestamp(message.date, 0),
            sender: User {
                id: from.id.into(),
                profile: Some(UserProfile {
                    display_name: telegram_display_name(&from),
                    ..UserProfile::default()
                }),
            },
            conversation,
            message: Message::text(text),
        });
        let id = EventId::new(format!("telegram:update:{}", self.update_id)).ok()?;
        Some((id, event))
    }
}
fn telegram_display_name(user: &TelegramUser) -> Option<String> {
    let value = [Some(user.first_name.as_str()), user.last_name.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
    (!value.is_empty()).then_some(value)
}
#[derive(Deserialize)]
struct TelegramMessage {
    message_id: i64,
    date: i64,
    from: Option<TelegramUser>,
    chat: TelegramChat,
    text: Option<String>,
}
#[derive(Deserialize)]
struct TelegramUser {
    id: i64,
    first_name: String,
    last_name: Option<String>,
}
#[derive(Deserialize)]
struct TelegramChat {
    id: i64,
    #[serde(rename = "type")]
    kind: Option<String>,
}
