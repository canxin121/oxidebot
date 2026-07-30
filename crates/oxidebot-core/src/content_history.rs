//! Message history, forwarding, and batch-send models.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{NotificationPolicy, RichText};
use crate::{
    collaboration::ReactionSummary,
    conversation::{ConversationRef, MessageRef, MessageTarget},
    interaction::PlatformNativeData,
    source::{
        message::{Message, MessageSegment},
        user::User,
    },
};

/// Complete message record returned by a platform history API.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MessageEnvelope {
    /// Stable message reference.
    pub reference: MessageRef,
    /// Original sender, if known.
    pub sender: Option<User>,
    /// Conversation containing the message, if known.
    pub conversation: Option<ConversationRef>,
    /// Creation time, if known.
    pub created_at: Option<DateTime<Utc>>,
    /// Most recent edit time, if known.
    pub edited_at: Option<DateTime<Utc>>,
    /// Normalized message segments.
    pub content: Vec<MessageSegment>,
    /// Referenced reply target, if any.
    pub reply_to: Option<MessageRef>,
    /// Reply context retained by the platform.
    pub reply_context: Option<ReplyContext>,
    /// Referenced forwarded message, if any.
    pub forwarded_from: Option<MessageRef>,
    /// Forward origin context.
    pub forward_context: Option<ForwardContext>,
    /// Current reaction aggregates.
    pub reactions: Vec<ReactionSummary>,
    /// Whether the message is pinned, if exposed.
    pub pinned: Option<bool>,
    /// Adapter-independent metadata.
    pub metadata: BTreeMap<String, Value>,
    /// Lossless platform-specific message metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// Context retained for a message reply.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReplyContext {
    /// Original message, if resolvable.
    pub message: Option<MessageRef>,
    /// Quoted text, if any.
    pub quote: Option<RichText>,
    /// Quote byte position in original text.
    pub quote_position: Option<u32>,
    /// Origin outside the current platform or conversation.
    pub external_origin: Option<MessageOrigin>,
    /// Lossless platform-specific reply context.
    pub platform_data: Option<PlatformNativeData>,
}

/// Original source of forwarded or externally referenced content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MessageOrigin {
    /// Originated from one user.
    User(User),
    /// Originated from a conversation message.
    Conversation {
        /// Source conversation.
        conversation: ConversationRef,
        /// Source message, if known.
        message: Option<MessageRef>,
        /// Optional platform author signature.
        author_signature: Option<String>,
    },
    /// Originated from a hidden or anonymized user.
    HiddenUser {
        /// User-visible source name.
        name: String,
    },
    /// Lossless platform-native origin.
    PlatformNative(PlatformNativeData),
}

/// Forwarding metadata for a message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForwardContext {
    /// Source origin.
    pub origin: MessageOrigin,
    /// Original send time, if known.
    pub sent_at: Option<DateTime<Utc>>,
    /// Whether the platform performed an automatic forward.
    pub automatically_forwarded: bool,
    /// Lossless platform-specific forward metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// Query parameters for platform message history.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MessageQuery {
    /// Conversation to search.
    pub conversation: Option<ConversationRef>,
    /// Thread to search.
    pub thread: Option<ConversationRef>,
    /// Return messages before this reference.
    pub before: Option<MessageRef>,
    /// Return messages after this reference.
    pub after: Option<MessageRef>,
    /// Center results around this reference.
    pub around: Option<MessageRef>,
    /// Maximum results requested.
    pub limit: Option<u32>,
    /// Free-text search query.
    pub search: Option<String>,
    /// Opaque pagination cursor.
    pub cursor: Option<String>,
    /// Lossless platform-specific query parameters.
    pub platform_data: Option<PlatformNativeData>,
}

/// Options that control message forwarding or copying.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForwardOptions {
    /// Whether original author attribution is preserved.
    pub preserve_author: bool,
    /// Whether captions are preserved.
    pub preserve_caption: bool,
    /// Notification behavior for the forwarded message.
    pub notification: NotificationPolicy,
    /// Lossless platform-specific forwarding metadata.
    pub platform_data: Option<PlatformNativeData>,
}

impl Default for ForwardOptions {
    fn default() -> Self {
        Self {
            preserve_author: true,
            preserve_caption: true,
            notification: NotificationPolicy::Default,
            platform_data: None,
        }
    }
}

/// One message addressed to a target within a batch send.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BatchMessage {
    /// Destination target.
    pub target: MessageTarget,
    /// Portable message to deliver.
    pub message: Message,
}

/// Result of one input item in a batch send.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BatchItemResult {
    /// Zero-based batch input index.
    pub index: usize,
    /// Created message reference on success.
    pub message: Option<MessageRef>,
    /// Failure detail on error.
    pub error: Option<String>,
}

/// Complete result of a batch send operation.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BatchSendResult {
    /// Per-input delivery results.
    pub items: Vec<BatchItemResult>,
}
