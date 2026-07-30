//! Portable message delivery option models.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::RichText;
use crate::{conversation::MessageRef, interaction::PlatformNativeData};

/// Delivery options for replying to an existing message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReplyOptions {
    /// Referenced original message.
    pub message: MessageRef,
    /// Optional quoted text.
    pub quote: Option<RichText>,
    /// Quote byte offset in the original message.
    pub quote_position: Option<u32>,
    /// Whether delivery may proceed if the original is unavailable.
    pub allow_without_original: bool,
    /// Whether the original sender should be notified.
    pub notify_original_sender: Option<bool>,
    /// Lossless platform-specific reply metadata.
    pub platform_data: Option<PlatformNativeData>,
}

impl ReplyOptions {
    /// Creates reply options for a referenced original message.
    pub fn new(message: MessageRef) -> Self {
        Self {
            message,
            quote: None,
            quote_position: None,
            allow_without_original: false,
            notify_original_sender: None,
            platform_data: None,
        }
    }
}

/// Notification behavior requested for message delivery.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotificationPolicy {
    /// Platform default behavior.
    #[default]
    Default,
    /// Suppress notifications where supported.
    Silent,
    /// Force notifications where supported.
    Force,
}

/// Audience allowed to see a delivered message.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageVisibility {
    /// Normal conversation audience.
    #[default]
    Public,
    /// Only the interacting user, where supported.
    Ephemeral,
    /// Explicit set of platform user identifiers.
    PrivateTo(Vec<String>),
}

/// Link-preview presentation options.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkPreviewOptions {
    /// Whether previews are enabled.
    pub enabled: bool,
    /// Whether large media is preferred.
    pub prefer_large_media: Option<bool>,
    /// Whether preview appears above text.
    pub above_text: Option<bool>,
}

/// Set of entities permitted to receive mentions.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MentionAllowance {
    /// Use platform default mention behavior.
    #[default]
    PlatformDefault,
    /// Permit no mentions of this entity type.
    None,
    /// Permit all mentions of this entity type.
    All,
    /// Permit only listed platform identifiers.
    Only(Vec<String>),
}

/// Per-entity mention delivery policy.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MentionPolicy {
    /// User mention allowance.
    pub users: MentionAllowance,
    /// Role mention allowance.
    pub roles: MentionAllowance,
    /// Conversation mention allowance.
    pub conversations: MentionAllowance,
    /// Whether everyone mentions are allowed.
    pub everyone: bool,
    /// Whether replying may notify the original author.
    pub reply_author: bool,
}

/// Time at which a message should be delivered.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeliveryTime {
    /// Deliver immediately.
    #[default]
    Immediate,
    /// Deliver at the specified time.
    Scheduled(DateTime<Utc>),
    /// Save as a platform draft.
    Draft,
}
