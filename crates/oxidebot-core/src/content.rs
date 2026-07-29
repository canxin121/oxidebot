//! Portable message content, rich text, rich layout, media, poll, checklist,
//! contact, and delivery models.

use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, ops::Range, time::Duration};

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use serde_json::Value;

use crate::{
    collaboration::ReactionSummary,
    conversation::{ConversationRef, MessageRef, MessageTarget},
    interaction::{ActionRow, PlatformNativeData},
    source::{
        message::{File, Message, MessageSegment},
        user::User,
    },
};

/// Text with optional byte-ranged semantic styling.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RichText {
    /// Unstyled UTF-8 text.
    pub text: String,
    /// Semantic style spans over `text`.
    pub spans: Vec<TextSpan>,
}

impl RichText {
    /// Creates rich text with no style spans.
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            spans: Vec::new(),
        }
    }

    /// Adds one styled byte range.
    pub fn span(mut self, range: Range<usize>, style: TextStyle) -> Self {
        self.spans.push(TextSpan {
            range,
            styles: vec![style],
        });
        self
    }
}

/// One byte range and its semantic text styles.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextSpan {
    /// Byte offsets into [`RichText::text`]. Adapters must validate UTF-8
    /// boundaries before platform-specific unit conversion (for example,
    /// Telegram and Discord UTF-16 offsets).
    pub range: Range<usize>,
    /// Semantic styles applied to the range.
    pub styles: Vec<TextStyle>,
}

/// Semantic style applied to a rich-text range.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum TextStyle {
    /// Bold emphasis.
    Bold,
    /// Italic emphasis.
    Italic,
    /// Underline decoration.
    Underline,
    /// Strikethrough decoration.
    Strikethrough,
    /// Spoiler content.
    Spoiler,
    /// Inline code.
    Code,
    /// Preformatted code block.
    Preformatted {
        /// Optional programming language identifier.
        language: Option<String>,
    },
    /// Hyperlink.
    Link {
        /// Destination URL.
        url: String,
    },
    /// Mention of a user.
    UserMention {
        /// Platform user identifier.
        user_id: String,
    },
    /// Mention of a role.
    RoleMention {
        /// Platform role identifier.
        role_id: String,
    },
    /// Mention of a conversation.
    ConversationMention {
        /// Platform conversation identifier.
        conversation_id: String,
    },
    /// Custom emoji.
    CustomEmoji {
        /// Platform emoji identifier.
        id: String,
        /// Text fallback when custom emoji is unsupported.
        fallback: Option<String>,
    },
    /// Quoted text.
    Quote,
    /// Marked or highlighted text.
    Marked,
    /// Subscript text.
    Subscript,
    /// Superscript text.
    Superscript,
    /// Hashtag token.
    Hashtag,
    /// Cashtag token.
    Cashtag,
    /// Bot command token.
    BotCommand,
    /// Email address token.
    Email,
    /// Phone number token.
    Phone,
    /// Date-time token.
    DateTime {
        /// Parsed timestamp, if the platform provides one.
        timestamp: Option<DateTime<Utc>>,
    },
    /// Mathematical expression.
    MathematicalExpression,
    /// Message reference.
    Reference {
        /// Referenced message, if resolvable.
        message: Option<MessageRef>,
    },
    /// Lossless platform-native styling.
    PlatformNative(PlatformNativeData),
}

/// File-backed media and its presentation metadata.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Media {
    /// Required media file descriptor.
    pub file: File,
    /// Optional rich-text caption.
    pub caption: Option<RichText>,
    /// Optional preview thumbnail.
    pub thumbnail: Option<File>,
    /// Pixel width, if known.
    pub width: Option<u32>,
    /// Pixel height, if known.
    pub height: Option<u32>,
    /// Playback duration, if applicable.
    pub duration: Option<Duration>,
    /// Whether the client should conceal the media until revealed.
    pub spoiler: bool,
    /// Text alternative for non-visual rendering.
    pub alt_text: Option<String>,
    /// Optional audio waveform samples.
    pub waveform: Option<Vec<u8>>,
    /// Lossless platform-specific media metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// Portable media category.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaType {
    /// Image media.
    #[default]
    Image,
    /// Video media.
    Video,
    /// Audio media.
    Audio,
    /// General document.
    Document,
    /// Animated image or video.
    Animation,
    /// Voice note.
    VoiceNote,
    /// Short video note.
    VideoNote,
    /// Platform-native media category.
    PlatformNative(String),
}

/// One item in a media gallery.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MediaGalleryItem {
    /// Category used to render the item.
    pub kind: MediaType,
    /// Media content and metadata.
    pub media: Media,
}

impl Media {
    /// Creates media with required file and default metadata.
    pub fn new(file: File) -> Self {
        Self {
            file,
            ..Default::default()
        }
    }
}

/// Geographic location content.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LocationContent {
    /// Latitude in decimal degrees.
    pub latitude: f64,
    /// Longitude in decimal degrees.
    pub longitude: f64,
    /// Optional user-visible location title.
    pub title: Option<String>,
    /// Optional human-readable address.
    pub address: Option<String>,
    /// Horizontal accuracy in meters.
    pub horizontal_accuracy: Option<f64>,
    /// Live-location lifetime.
    pub live_period: Option<Duration>,
    /// Heading in degrees.
    pub heading: Option<u16>,
    /// Proximity alert radius in meters.
    pub proximity_alert_radius: Option<u32>,
    /// Lossless platform-specific location metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// Labeled phone number in a contact card.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhoneNumber {
    /// Optional user-visible label.
    pub label: Option<String>,
    /// Phone number value.
    pub value: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ContactCard {
    pub user_id: Option<String>,
    pub first_name: String,
    pub last_name: Option<String>,
    pub phone_numbers: Vec<PhoneNumber>,
    pub emails: Vec<String>,
    pub organization: Option<String>,
    pub vcard: Option<String>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CustomEmoji {
    pub id: String,
    pub name: Option<String>,
    pub fallback: Option<String>,
    pub file: Option<File>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sticker {
    pub id: String,
    pub pack_id: Option<String>,
    pub emoji: Option<String>,
    pub file: Option<File>,
    pub animated: bool,
    pub video: bool,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum PollType {
    #[default]
    Regular,
    Quiz,
    PlatformNative(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PollOption {
    pub id: Option<String>,
    pub text: RichText,
    pub voter_count: Option<u64>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Poll {
    pub id: Option<String>,
    pub question: RichText,
    pub options: Vec<PollOption>,
    pub allows_multiple_answers: bool,
    pub allows_revoting: Option<bool>,
    pub members_only: Option<bool>,
    pub country_codes: Vec<String>,
    pub anonymous: Option<bool>,
    pub kind: PollType,
    pub open_for: Option<Duration>,
    pub closes_at: Option<DateTime<Utc>>,
    pub total_voter_count: Option<u64>,
    /// Retained as a convenience for platforms limited to one correct answer.
    pub correct_option: Option<usize>,
    pub correct_options: Vec<usize>,
    pub explanation: Option<RichText>,
    pub closed: bool,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChecklistTask {
    pub id: Option<String>,
    pub text: RichText,
    pub completed: bool,
    pub completed_by: Option<User>,
    pub completed_at: Option<DateTime<Utc>>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Checklist {
    pub id: Option<String>,
    pub title: RichText,
    pub tasks: Vec<ChecklistTask>,
    pub can_add_tasks: bool,
    pub can_mark_tasks_done: bool,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ChecklistChange {
    pub message: Option<MessageRef>,
    pub added_tasks: Vec<ChecklistTask>,
    pub completed_task_ids: Vec<String>,
    pub reopened_task_ids: Vec<String>,
    pub actor: Option<User>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LayoutStyle {
    pub accent_color: Option<u32>,
    pub spoiler: bool,
    pub collapsible: bool,
    pub initially_collapsed: bool,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayoutColumn {
    pub width: Option<u16>,
    pub nodes: Vec<LayoutNode>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TableCell {
    pub content: Vec<LayoutNode>,
    pub header: bool,
    pub colspan: Option<u16>,
    pub rowspan: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LayoutNode {
    Text(RichText),
    Section {
        children: Vec<LayoutNode>,
        accessory: Option<Box<LayoutNode>>,
        style: LayoutStyle,
    },
    Container {
        children: Vec<LayoutNode>,
        style: LayoutStyle,
    },
    Columns(Vec<LayoutColumn>),
    Grid {
        columns: u16,
        children: Vec<LayoutNode>,
    },
    Image(Media),
    MediaGallery(Vec<MediaGalleryItem>),
    File(Media),
    List {
        ordered: bool,
        items: Vec<Vec<LayoutNode>>,
    },
    Table {
        rows: Vec<Vec<TableCell>>,
    },
    Quote(RichText),
    Code {
        text: String,
        language: Option<String>,
    },
    Divider,
    Map(LocationContent),
    Details {
        summary: RichText,
        children: Vec<LayoutNode>,
        open: bool,
    },
    Actions(ActionRow),
    PlatformNative(PlatformNativeData),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RichLayout {
    pub nodes: Vec<LayoutNode>,
    pub fallback_text: Option<String>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReplyOptions {
    pub message: MessageRef,
    pub quote: Option<RichText>,
    pub quote_position: Option<u32>,
    pub allow_without_original: bool,
    pub notify_original_sender: Option<bool>,
    pub platform_data: Option<PlatformNativeData>,
}

impl ReplyOptions {
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

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotificationPolicy {
    #[default]
    Default,
    Silent,
    Force,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageVisibility {
    #[default]
    Public,
    Ephemeral,
    PrivateTo(Vec<String>),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkPreviewOptions {
    pub enabled: bool,
    pub prefer_large_media: Option<bool>,
    pub above_text: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MentionAllowance {
    #[default]
    PlatformDefault,
    None,
    All,
    Only(Vec<String>),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MentionPolicy {
    pub users: MentionAllowance,
    pub roles: MentionAllowance,
    pub conversations: MentionAllowance,
    pub everyone: bool,
    pub reply_author: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeliveryTime {
    #[default]
    Immediate,
    Scheduled(DateTime<Utc>),
    Draft,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MessageEnvelope {
    pub reference: MessageRef,
    pub sender: Option<User>,
    pub conversation: Option<ConversationRef>,
    pub created_at: Option<DateTime<Utc>>,
    pub edited_at: Option<DateTime<Utc>>,
    pub content: Vec<MessageSegment>,
    pub reply_to: Option<MessageRef>,
    pub reply_context: Option<ReplyContext>,
    pub forwarded_from: Option<MessageRef>,
    pub forward_context: Option<ForwardContext>,
    pub reactions: Vec<ReactionSummary>,
    pub pinned: Option<bool>,
    pub metadata: BTreeMap<String, Value>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReplyContext {
    pub message: Option<MessageRef>,
    pub quote: Option<RichText>,
    pub quote_position: Option<u32>,
    pub external_origin: Option<MessageOrigin>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MessageOrigin {
    User(User),
    Conversation {
        conversation: ConversationRef,
        message: Option<MessageRef>,
        author_signature: Option<String>,
    },
    HiddenUser {
        name: String,
    },
    PlatformNative(PlatformNativeData),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForwardContext {
    pub origin: MessageOrigin,
    pub sent_at: Option<DateTime<Utc>>,
    pub automatically_forwarded: bool,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MessageQuery {
    pub conversation: Option<ConversationRef>,
    pub thread: Option<ConversationRef>,
    pub before: Option<MessageRef>,
    pub after: Option<MessageRef>,
    pub around: Option<MessageRef>,
    pub limit: Option<u32>,
    pub search: Option<String>,
    pub cursor: Option<String>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForwardOptions {
    pub preserve_author: bool,
    pub preserve_caption: bool,
    pub notification: NotificationPolicy,
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BatchMessage {
    pub target: MessageTarget,
    pub message: Message,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BatchItemResult {
    pub index: usize,
    pub message: Option<MessageRef>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BatchSendResult {
    pub items: Vec<BatchItemResult>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum FormValue {
    Text(String),
    Integer(i64),
    Number(f64),
    Boolean(bool),
    Date(NaiveDate),
    Time(NaiveTime),
    DateTime(DateTime<Utc>),
    User(User),
    Conversation(ConversationRef),
    File(File),
    Json(Value),
}
