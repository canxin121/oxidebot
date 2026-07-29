//! Reactions, pins, transient chat activity, and read state.

use serde::{Deserialize, Serialize};
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::{
    conversation::{ConversationRef, MessageRef},
    interaction::PlatformNativeData,
    source::user::User,
};

/// A reaction that can be applied to a message.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Reaction {
    /// Unicode emoji reaction.
    UnicodeEmoji(String),
    /// Platform custom emoji with optional human-readable name.
    CustomEmoji {
        /// Platform custom-emoji identifier.
        id: String,
        /// Optional human-readable emoji name.
        name: Option<String>,
    },
    /// Paid or platform-monetized reaction.
    Paid,
    /// Lossless platform-native reaction.
    PlatformNative {
        /// Platform that owns the reaction representation.
        platform: String,
        /// Platform-defined serialized reaction data.
        data: String,
    },
}

/// Options applied when adding a reaction.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReactionOptions {
    /// Whether the platform should emphasize this reaction.
    pub emphasized: bool,
    /// Optional platform-defined reaction variant.
    pub variant: Option<String>,
}

/// Current aggregate state for one reaction on a message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReactionSummary {
    /// Reaction being summarized.
    pub reaction: Reaction,
    /// Total number of users that applied it.
    pub count: u64,
    /// Whether the configured bot applied it.
    pub reacted_by_bot: bool,
    /// Recent users reported by the platform.
    pub recent_users: Vec<User>,
    /// Lossless platform-specific aggregate metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// Delta and current state for reactions on a message.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReactionChange {
    /// Reactions added since the previous state.
    pub added: Vec<Reaction>,
    /// Reactions removed since the previous state.
    pub removed: Vec<Reaction>,
    /// Current reaction aggregates.
    pub current: Vec<ReactionSummary>,
}

/// Transient activity a user or bot presents in a conversation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChatActivity {
    /// User is typing.
    #[default]
    Typing,
    /// User is uploading an image.
    UploadingPhoto,
    /// User is uploading a video.
    UploadingVideo,
    /// User is uploading audio.
    UploadingAudio,
    /// User is uploading a file.
    UploadingFile,
    /// User is recording audio.
    RecordingAudio,
    /// User is recording video.
    RecordingVideo,
    /// User is choosing a sticker.
    ChoosingSticker,
    /// User is playing a game or other activity.
    Playing,
    /// Activity without a portable OxideBot equivalent.
    PlatformNative(String),
}

/// Current transient activity state in a conversation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActivityState {
    /// Conversation in which the activity appears.
    pub conversation: ConversationRef,
    /// User performing the activity, if the platform identifies one.
    pub user: Option<User>,
    /// Activity being displayed.
    pub activity: ChatActivity,
    /// Duration after which the activity expires, if known.
    pub expires_after: Option<Duration>,
    /// Lossless platform-specific activity metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// Options applied when pinning a message.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinOptions {
    /// Whether the platform should notify conversation members.
    pub notify_members: Option<bool>,
}

/// A message currently or previously pinned in a conversation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PinnedMessage {
    /// Referenced message.
    pub message: MessageRef,
    /// User that pinned it, if supplied.
    pub pinned_by: Option<User>,
    /// Time at which it was pinned, if supplied.
    pub pinned_at: Option<DateTime<Utc>>,
    /// Lossless platform-specific pin metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// A user's read position in a conversation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReadReceipt {
    /// Conversation whose read position changed.
    pub conversation: ConversationRef,
    /// User whose position changed, if supplied.
    pub user: Option<User>,
    /// Latest message known to be read, if supplied.
    pub through_message: Option<MessageRef>,
    /// Time reported for the read receipt, if supplied.
    pub read_at: Option<DateTime<Utc>>,
    /// Lossless platform-specific read metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// Kind of a voice, video, or platform-native call.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallKind {
    /// Audio-only call.
    Voice,
    /// Video call.
    #[default]
    Video,
    /// Stage-style audio room.
    Stage,
    /// Broadcast live stream.
    LiveStream,
    /// Call kind without a portable OxideBot equivalent.
    PlatformNative(String),
}

/// Lifecycle state of a call session.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallState {
    /// Call is scheduled for a future time.
    Scheduled,
    /// Call is currently active.
    #[default]
    Active,
    /// Call ended normally.
    Ended,
    /// Call was canceled before or during operation.
    Canceled,
    /// Call state without a portable OxideBot equivalent.
    PlatformNative(String),
}

/// Options used to create or update a call session.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CallOptions {
    /// Requested call kind.
    pub kind: CallKind,
    /// Optional user-visible call title.
    pub title: Option<String>,
    /// Optional scheduled start time.
    pub scheduled_for: Option<DateTime<Utc>>,
    /// Lossless platform-specific call options.
    pub platform_data: Option<PlatformNativeData>,
}

/// A voice, video, stage, or live-stream session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CallSession {
    /// Platform call identifier, if supplied.
    pub id: Option<String>,
    /// Conversation hosting the call.
    pub conversation: ConversationRef,
    /// Call media or presentation kind.
    pub kind: CallKind,
    /// Current lifecycle state.
    pub state: CallState,
    /// Optional user-visible title.
    pub title: Option<String>,
    /// Scheduled start time, if any.
    pub scheduled_for: Option<DateTime<Utc>>,
    /// Actual start time, if known.
    pub started_at: Option<DateTime<Utc>>,
    /// Actual end time, if known.
    pub ended_at: Option<DateTime<Utc>>,
    /// Platform-reported duration, if known.
    pub duration: Option<Duration>,
    /// Participants reported by the platform.
    pub participants: Vec<User>,
    /// Lossless platform-specific session metadata.
    pub platform_data: Option<PlatformNativeData>,
}
