//! Granular platform capability and constraint reporting.

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

use crate::{content::MessageVisibility, interaction::ButtonStyle};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum SupportLevel {
    Native,
    Emulated,
    #[default]
    Unsupported,
}

impl SupportLevel {
    pub const fn is_supported(self) -> bool {
        !matches!(self, Self::Unsupported)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContentCapabilities {
    pub plain_text: SupportLevel,
    pub rich_text: SupportLevel,
    pub rich_layout: SupportLevel,
    pub images: SupportLevel,
    pub video: SupportLevel,
    pub audio: SupportLevel,
    pub animation: SupportLevel,
    pub voice_notes: SupportLevel,
    pub video_notes: SupportLevel,
    pub files: SupportLevel,
    pub media_galleries: SupportLevel,
    pub location: SupportLevel,
    pub contacts: SupportLevel,
    pub stickers: SupportLevel,
    pub custom_emoji: SupportLevel,
    pub polls: SupportLevel,
    pub quizzes: SupportLevel,
    pub checklists: SupportLevel,
    pub platform_native: SupportLevel,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeliveryCapabilities {
    pub replies: SupportLevel,
    pub quoted_replies: SupportLevel,
    pub threads: SupportLevel,
    pub silent: SupportLevel,
    pub forced_notification: SupportLevel,
    pub protected_content: SupportLevel,
    pub link_preview_control: SupportLevel,
    pub mention_control: SupportLevel,
    pub ephemeral: SupportLevel,
    pub private_to_users: SupportLevel,
    pub scheduling: SupportLevel,
    pub drafts: SupportLevel,
    pub idempotency_keys: SupportLevel,
    pub client_message_ids: SupportLevel,
    pub metadata: SupportLevel,
    pub supported_visibilities: Vec<MessageVisibility>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ButtonCapabilities {
    pub callback: SupportLevel,
    pub send_text: SupportLevel,
    pub url: SupportLevel,
    pub web_app: SupportLevel,
    pub login: SupportLevel,
    pub switch_inline_query: SupportLevel,
    pub copy_text: SupportLevel,
    pub game: SupportLevel,
    pub pay: SupportLevel,
    pub disabled: SupportLevel,
    pub icons: SupportLevel,
    pub styles: BTreeSet<ButtonStyle>,
    pub max_callback_bytes: Option<usize>,
    pub max_buttons_per_row: Option<usize>,
    pub max_rows: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ComponentCapabilities {
    pub inline_keyboard: SupportLevel,
    pub reply_keyboard: SupportLevel,
    pub selects: SupportLevel,
    pub text_input: SupportLevel,
    pub number_input: SupportLevel,
    pub checkbox: SupportLevel,
    pub checkbox_group: SupportLevel,
    pub radio_group: SupportLevel,
    pub toggle: SupportLevel,
    pub date_picker: SupportLevel,
    pub time_picker: SupportLevel,
    pub datetime_picker: SupportLevel,
    pub file_upload: SupportLevel,
    pub dynamic_suggestions: SupportLevel,
    pub modals: SupportLevel,
    pub platform_native: SupportLevel,
    pub buttons: ButtonCapabilities,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InteractionLifecycleCapabilities {
    pub acknowledge: SupportLevel,
    pub defer: SupportLevel,
    pub notifications: SupportLevel,
    pub open_url: SupportLevel,
    pub initial_message: SupportLevel,
    pub update_original: SupportLevel,
    pub followups: SupportLevel,
    pub edit_original: SupportLevel,
    pub delete_original: SupportLevel,
    pub validation_errors: SupportLevel,
    pub view_navigation: SupportLevel,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConversationCapabilities {
    pub direct: SupportLevel,
    pub groups: SupportLevel,
    pub channels: SupportLevel,
    pub threads: SupportLevel,
    pub topics: SupportLevel,
    pub forums: SupportLevel,
    pub create_threads: SupportLevel,
    pub manage_threads: SupportLevel,
    pub history: SupportLevel,
    pub search: SupportLevel,
    pub members: SupportLevel,
    pub roles: SupportLevel,
    pub member_tags: SupportLevel,
    pub permissions: SupportLevel,
    pub moderation: SupportLevel,
    pub join_requests: SupportLevel,
    pub invite_links: SupportLevel,
    pub profile: SupportLevel,
    pub leave: SupportLevel,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CollaborationCapabilities {
    pub reactions: SupportLevel,
    pub multiple_reactions: SupportLevel,
    pub reaction_users: SupportLevel,
    pub pins: SupportLevel,
    pub typing: SupportLevel,
    pub read_receipts: SupportLevel,
    pub call_events: SupportLevel,
    pub call_management: SupportLevel,
    pub forwarding: SupportLevel,
    pub copying: SupportLevel,
    pub batch_send: SupportLevel,
    pub bulk_delete: SupportLevel,
    pub broadcast: SupportLevel,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApplicationCapabilities {
    pub commands: SupportLevel,
    pub structured_commands: SupportLevel,
    pub command_localizations: SupportLevel,
    pub autocomplete: SupportLevel,
    pub chat_menu: SupportLevel,
    pub home_surface: SupportLevel,
    pub rich_menu: SupportLevel,
    pub mini_apps: SupportLevel,
    pub payments: SupportLevel,
    pub subscriptions: SupportLevel,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlatformLimits {
    pub max_text_length: Option<usize>,
    pub max_caption_length: Option<usize>,
    pub max_rich_text_length: Option<usize>,
    pub max_media_per_message: Option<usize>,
    pub max_file_bytes: Option<u64>,
    pub max_components: Option<usize>,
    pub max_commands: Option<usize>,
    pub max_poll_options: Option<usize>,
    pub max_batch_size: Option<usize>,
    pub edit_window_seconds: Option<u64>,
    pub supported_mime_types: Vec<String>,
    pub platform_limits: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BotCapabilities {
    pub content: ContentCapabilities,
    pub delivery: DeliveryCapabilities,
    pub components: ComponentCapabilities,
    pub interaction_lifecycle: InteractionLifecycleCapabilities,
    pub conversations: ConversationCapabilities,
    pub collaboration: CollaborationCapabilities,
    pub application: ApplicationCapabilities,
    pub limits: PlatformLimits,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsupportedFeatureError {
    pub feature: String,
}

impl UnsupportedFeatureError {
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
