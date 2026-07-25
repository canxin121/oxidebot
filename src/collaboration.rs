//! Reactions, pins, transient chat activity, and read state.

use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::{
    conversation::{ConversationRef, MessageRef},
    interaction::PlatformNativeData,
    source::user::User,
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reaction {
    UnicodeEmoji(String),
    CustomEmoji { id: String, name: Option<String> },
    Paid,
    PlatformNative { platform: String, data: String },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReactionOptions {
    pub emphasized: bool,
    pub variant: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReactionSummary {
    pub reaction: Reaction,
    pub count: u64,
    pub reacted_by_bot: bool,
    pub recent_users: Vec<User>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReactionChange {
    pub added: Vec<Reaction>,
    pub removed: Vec<Reaction>,
    pub current: Vec<ReactionSummary>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ChatActivity {
    #[default]
    Typing,
    UploadingPhoto,
    UploadingVideo,
    UploadingAudio,
    UploadingFile,
    RecordingAudio,
    RecordingVideo,
    ChoosingSticker,
    Playing,
    PlatformNative(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActivityState {
    pub conversation: ConversationRef,
    pub user: Option<User>,
    pub activity: ChatActivity,
    pub expires_after: Option<Duration>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PinOptions {
    pub notify_members: Option<bool>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PinnedMessage {
    pub message: MessageRef,
    pub pinned_by: Option<User>,
    pub pinned_at: Option<DateTime<Utc>>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReadReceipt {
    pub conversation: ConversationRef,
    pub user: Option<User>,
    pub through_message: Option<MessageRef>,
    pub read_at: Option<DateTime<Utc>>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CallKind {
    Voice,
    #[default]
    Video,
    Stage,
    LiveStream,
    PlatformNative(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CallState {
    Scheduled,
    #[default]
    Active,
    Ended,
    Canceled,
    PlatformNative(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CallOptions {
    pub kind: CallKind,
    pub title: Option<String>,
    pub scheduled_for: Option<DateTime<Utc>>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CallSession {
    pub id: Option<String>,
    pub conversation: ConversationRef,
    pub kind: CallKind,
    pub state: CallState,
    pub title: Option<String>,
    pub scheduled_for: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub duration: Option<Duration>,
    pub participants: Vec<User>,
    pub platform_data: Option<PlatformNativeData>,
}
