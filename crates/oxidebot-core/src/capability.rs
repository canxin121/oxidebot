//! Granular platform capability and constraint reporting.

use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

use crate::{content::MessageVisibility, interaction::ButtonStyle};

/// Degree to which an adapter can provide a portable capability.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SupportLevel {
    /// The platform supports the capability without semantic loss.
    Native,
    /// OxideBot can approximate the capability with a documented fallback.
    Emulated,
    /// The adapter cannot provide the capability.
    #[default]
    Unsupported,
}

impl SupportLevel {
    /// Returns whether this level is native or emulated.
    pub const fn is_supported(self) -> bool {
        !matches!(self, Self::Unsupported)
    }
}

/// Content formats and content-level features supported by an adapter.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentCapabilities {
    /// Plain text messages.
    pub plain_text: SupportLevel,
    /// Styled rich text.
    pub rich_text: SupportLevel,
    /// Structured rich layouts.
    pub rich_layout: SupportLevel,
    /// Image media.
    pub images: SupportLevel,
    /// Video media.
    pub video: SupportLevel,
    /// Audio media.
    pub audio: SupportLevel,
    /// Animation media.
    pub animation: SupportLevel,
    /// Voice-note media.
    pub voice_notes: SupportLevel,
    /// Video-note media.
    pub video_notes: SupportLevel,
    /// General file attachments.
    pub files: SupportLevel,
    /// Multiple media items in one message.
    pub media_galleries: SupportLevel,
    /// Geographic location content.
    pub location: SupportLevel,
    /// Contact-card content.
    pub contacts: SupportLevel,
    /// Sticker content.
    pub stickers: SupportLevel,
    /// Custom emoji content.
    pub custom_emoji: SupportLevel,
    /// User mentions.
    #[serde(default)]
    pub user_mentions: SupportLevel,
    #[serde(default)]
    /// Role mentions.
    pub role_mentions: SupportLevel,
    /// Channel mentions.
    #[serde(default)]
    pub channel_mentions: SupportLevel,
    #[serde(default)]
    /// Everyone or broadcast mentions.
    pub everyone_mentions: SupportLevel,
    /// Share or repost content.
    #[serde(default)]
    pub shares: SupportLevel,
    #[serde(default)]
    /// Poll content.
    pub polls: SupportLevel,
    /// Quiz poll content.
    pub quizzes: SupportLevel,
    /// Checklist content.
    pub checklists: SupportLevel,
    /// Lossless platform-native content.
    pub platform_native: SupportLevel,
}

/// Delivery options and message lifecycle operations supported by an adapter.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryCapabilities {
    /// Replies to a referenced message.
    pub replies: SupportLevel,
    /// Editing previously sent messages.
    pub edit_messages: SupportLevel,
    /// Deleting previously sent messages.
    pub delete_messages: SupportLevel,
    /// Quoted reply presentation.
    pub quoted_replies: SupportLevel,
    /// Delivery into threads or topics.
    pub threads: SupportLevel,
    /// Silent notifications.
    pub silent: SupportLevel,
    /// Forced notifications.
    pub forced_notification: SupportLevel,
    /// Protected or non-forwardable content.
    pub protected_content: SupportLevel,
    /// Link-preview customization.
    pub link_preview_control: SupportLevel,
    /// Mention behavior customization.
    pub mention_control: SupportLevel,
    /// Ephemeral message visibility.
    pub ephemeral: SupportLevel,
    /// Messages visible only to specified users.
    pub private_to_users: SupportLevel,
    /// Scheduled delivery.
    pub scheduling: SupportLevel,
    /// Saving platform drafts.
    pub drafts: SupportLevel,
    /// Caller-supplied idempotency keys.
    pub idempotency_keys: SupportLevel,
    /// Caller-supplied client message identifiers.
    pub client_message_ids: SupportLevel,
    /// Arbitrary delivery metadata.
    pub metadata: SupportLevel,
    /// Message visibility variants accepted by the platform.
    pub supported_visibilities: Vec<MessageVisibility>,
}

/// Interactive button features and platform limits.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ButtonCapabilities {
    /// Callback actions.
    pub callback: SupportLevel,
    /// Send-text actions.
    pub send_text: SupportLevel,
    /// URL actions.
    pub url: SupportLevel,
    /// Web-app actions.
    pub web_app: SupportLevel,
    /// Login actions.
    pub login: SupportLevel,
    /// Inline-query switch actions.
    pub switch_inline_query: SupportLevel,
    /// Copy-text actions.
    pub copy_text: SupportLevel,
    /// Game actions.
    pub game: SupportLevel,
    /// Payment actions.
    pub pay: SupportLevel,
    /// Disabled button state.
    pub disabled: SupportLevel,
    /// Button icons.
    pub icons: SupportLevel,
    /// Native button styles accepted by the platform.
    pub styles: BTreeSet<ButtonStyle>,
    /// Maximum callback payload size in bytes, if limited.
    pub max_callback_bytes: Option<usize>,
    /// Maximum buttons per row, if limited.
    pub max_buttons_per_row: Option<usize>,
    /// Maximum button rows, if limited.
    pub max_rows: Option<usize>,
}

/// General interactive component features supported by an adapter.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentCapabilities {
    /// Inline keyboard components.
    pub inline_keyboard: SupportLevel,
    /// Reply keyboard components.
    pub reply_keyboard: SupportLevel,
    /// Select-menu components.
    pub selects: SupportLevel,
    /// Free-text input components.
    pub text_input: SupportLevel,
    /// Numeric input components.
    pub number_input: SupportLevel,
    /// Single checkbox components.
    pub checkbox: SupportLevel,
    /// Checkbox-group components.
    pub checkbox_group: SupportLevel,
    /// Radio-group components.
    pub radio_group: SupportLevel,
    /// Toggle components.
    pub toggle: SupportLevel,
    /// Date-picker components.
    pub date_picker: SupportLevel,
    /// Time-picker components.
    pub time_picker: SupportLevel,
    /// Combined date-time picker components.
    pub datetime_picker: SupportLevel,
    /// File-upload components.
    pub file_upload: SupportLevel,
    /// Dynamic suggestions while entering a value.
    pub dynamic_suggestions: SupportLevel,
    /// Modal components.
    pub modals: SupportLevel,
    /// Lossless platform-native components.
    pub platform_native: SupportLevel,
    /// Supported button capabilities.
    pub buttons: ButtonCapabilities,
}

/// Interaction acknowledgement and response-lifecycle features.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InteractionLifecycleCapabilities {
    /// Initial interaction acknowledgement.
    pub acknowledge: SupportLevel,
    /// Deferred interaction acknowledgement.
    pub defer: SupportLevel,
    /// Transient interaction notifications.
    pub notifications: SupportLevel,
    /// Opening a URL as an interaction response.
    pub open_url: SupportLevel,
    /// Initial interaction message responses.
    pub initial_message: SupportLevel,
    /// Updating the originating message.
    pub update_original: SupportLevel,
    /// Follow-up messages.
    pub followups: SupportLevel,
    /// Editing the original interaction response.
    pub edit_original: SupportLevel,
    /// Deleting the original interaction response.
    pub delete_original: SupportLevel,
    /// Field-level validation errors.
    pub validation_errors: SupportLevel,
    /// Navigation between interaction views.
    pub view_navigation: SupportLevel,
}

/// Conversation kinds and management operations supported by an adapter.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationCapabilities {
    /// Direct conversations.
    pub direct: SupportLevel,
    /// Group conversations.
    pub groups: SupportLevel,
    /// Channel conversations.
    pub channels: SupportLevel,
    /// Threads nested inside conversations.
    pub threads: SupportLevel,
    /// Topics nested inside conversations.
    pub topics: SupportLevel,
    /// Forum-style conversations.
    pub forums: SupportLevel,
    /// Creating threads or topics.
    pub create_threads: SupportLevel,
    /// Editing, closing, or deleting threads or topics.
    pub manage_threads: SupportLevel,
    /// Reading message history.
    pub history: SupportLevel,
    /// Searching conversation history.
    pub search: SupportLevel,
    /// Listing conversation members.
    pub members: SupportLevel,
    /// Accessing conversation roles.
    pub roles: SupportLevel,
    /// Setting platform member tags or labels.
    pub member_tags: SupportLevel,
    /// Changing conversation permissions.
    pub permissions: SupportLevel,
    /// Moderation operations.
    pub moderation: SupportLevel,
    /// Handling pending join requests.
    pub join_requests: SupportLevel,
    /// Creating and managing invite links.
    pub invite_links: SupportLevel,
    /// Reading and updating conversation profiles.
    pub profile: SupportLevel,
    /// Leaving a conversation.
    pub leave: SupportLevel,
}

/// Collaboration and multi-message operations supported by an adapter.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollaborationCapabilities {
    /// Applying reactions to messages.
    pub reactions: SupportLevel,
    /// Applying multiple reactions from the bot.
    pub multiple_reactions: SupportLevel,
    /// Listing users who reacted.
    pub reaction_users: SupportLevel,
    /// Pinning messages.
    pub pins: SupportLevel,
    /// Typing or activity indicators.
    pub typing: SupportLevel,
    /// Read receipts.
    pub read_receipts: SupportLevel,
    /// Receiving call lifecycle events.
    pub call_events: SupportLevel,
    /// Creating and managing calls.
    pub call_management: SupportLevel,
    /// Forwarding messages.
    pub forwarding: SupportLevel,
    /// Copying messages.
    pub copying: SupportLevel,
    /// Batch message delivery.
    pub batch_send: SupportLevel,
    /// Deleting multiple messages together.
    pub bulk_delete: SupportLevel,
    /// Broadcasting a message to many targets.
    pub broadcast: SupportLevel,
}

/// Application-level platform features supported by an adapter.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplicationCapabilities {
    /// Textual bot commands.
    pub commands: SupportLevel,
    /// Platform-visible structured commands.
    pub structured_commands: SupportLevel,
    /// Localized structured command metadata.
    pub command_localizations: SupportLevel,
    /// Dynamic command suggestions.
    pub autocomplete: SupportLevel,
    /// Chat-menu surfaces.
    pub chat_menu: SupportLevel,
    /// Bot home surfaces.
    pub home_surface: SupportLevel,
    /// Rich-menu surfaces.
    pub rich_menu: SupportLevel,
    /// Embedded mini applications.
    pub mini_apps: SupportLevel,
    /// One-time payments.
    pub payments: SupportLevel,
    /// Recurring subscriptions.
    pub subscriptions: SupportLevel,
}

/// Numeric and content constraints imposed by a platform.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformLimits {
    /// Maximum plain-text length, if limited.
    pub max_text_length: Option<usize>,
    /// Maximum caption length, if limited.
    pub max_caption_length: Option<usize>,
    /// Maximum rich-text length, if limited.
    pub max_rich_text_length: Option<usize>,
    /// Maximum media items in one message, if limited.
    pub max_media_per_message: Option<usize>,
    /// Maximum attachment size in bytes, if limited.
    pub max_file_bytes: Option<u64>,
    /// Maximum interactive components, if limited.
    pub max_components: Option<usize>,
    /// Maximum platform-visible commands, if limited.
    pub max_commands: Option<usize>,
    /// Maximum choices in a poll, if limited.
    pub max_poll_options: Option<usize>,
    /// Maximum messages in a batch, if limited.
    pub max_batch_size: Option<usize>,
    /// Editable-message window in seconds, if limited.
    pub edit_window_seconds: Option<u64>,
    /// MIME types accepted by the platform.
    pub supported_mime_types: Vec<String>,
    /// Lossless platform-specific limits keyed by adapter-defined name.
    pub platform_limits: BTreeMap<String, serde_json::Value>,
}

/// Complete capability and limit declaration for one registered bot.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotCapabilities {
    /// Content-format support.
    pub content: ContentCapabilities,
    /// Delivery and lifecycle support.
    pub delivery: DeliveryCapabilities,
    /// Interactive component support.
    pub components: ComponentCapabilities,
    /// Interaction response-lifecycle support.
    pub interaction_lifecycle: InteractionLifecycleCapabilities,
    /// Conversation support.
    pub conversations: ConversationCapabilities,
    /// Collaboration support.
    pub collaboration: CollaborationCapabilities,
    /// Application-level support.
    pub application: ApplicationCapabilities,
    /// Platform limits.
    pub limits: PlatformLimits,
}

impl BotCapabilities {
    /// A portable baseline for adapters that preserve the standard OxideBot
    /// message model but do not expose platform-native rich features.
    #[must_use]
    pub fn portable() -> Self {
        let native = SupportLevel::Native;
        Self {
            content: ContentCapabilities {
                plain_text: native,
                rich_text: SupportLevel::Emulated,
                rich_layout: SupportLevel::Emulated,
                images: native,
                video: native,
                audio: native,
                animation: SupportLevel::Emulated,
                voice_notes: SupportLevel::Emulated,
                video_notes: SupportLevel::Emulated,
                files: native,
                media_galleries: SupportLevel::Emulated,
                location: native,
                contacts: SupportLevel::Emulated,
                stickers: SupportLevel::Emulated,
                custom_emoji: SupportLevel::Emulated,
                user_mentions: native,
                role_mentions: SupportLevel::Emulated,
                channel_mentions: SupportLevel::Emulated,
                everyone_mentions: native,
                shares: native,
                polls: SupportLevel::Emulated,
                quizzes: SupportLevel::Emulated,
                checklists: SupportLevel::Emulated,
                platform_native: SupportLevel::Unsupported,
            },
            delivery: DeliveryCapabilities {
                replies: native,
                edit_messages: SupportLevel::Unsupported,
                delete_messages: SupportLevel::Unsupported,
                ..DeliveryCapabilities::default()
            },
            conversations: ConversationCapabilities {
                direct: native,
                groups: native,
                channels: SupportLevel::Emulated,
                ..ConversationCapabilities::default()
            },
            collaboration: CollaborationCapabilities {
                forwarding: native,
                ..CollaborationCapabilities::default()
            },
            ..Self::default()
        }
    }
}

/// Error returned when code requests a capability an adapter does not provide.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsupportedFeatureError {
    /// Name of the unavailable feature.
    pub feature: String,
}

impl UnsupportedFeatureError {
    /// Creates an unsupported-feature error for `feature`.
    pub fn new(feature: impl Into<String>) -> Self {
        Self {
            feature: feature.into(),
        }
    }
}

impl fmt::Display for UnsupportedFeatureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "feature {:?} is not supported by this adapter",
            self.feature
        )
    }
}

impl Error for UnsupportedFeatureError {}
