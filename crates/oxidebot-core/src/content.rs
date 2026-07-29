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

/// Contact information sent as message content.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ContactCard {
    /// Platform user identifier, if associated with one.
    pub user_id: Option<String>,
    /// Contact first name.
    pub first_name: String,
    /// Contact last name.
    pub last_name: Option<String>,
    /// Labeled phone numbers.
    pub phone_numbers: Vec<PhoneNumber>,
    /// Email addresses.
    pub emails: Vec<String>,
    /// Organization name.
    pub organization: Option<String>,
    /// Raw vCard payload.
    pub vcard: Option<String>,
    /// Lossless platform-specific contact metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// Custom platform emoji and its fallback data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CustomEmoji {
    /// Platform custom-emoji identifier.
    pub id: String,
    /// Optional user-visible name.
    pub name: Option<String>,
    /// Text fallback.
    pub fallback: Option<String>,
    /// Optional emoji asset.
    pub file: Option<File>,
    /// Lossless platform-specific emoji metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// Sticker content and associated asset metadata.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sticker {
    /// Platform sticker identifier.
    pub id: String,
    /// Optional sticker pack identifier.
    pub pack_id: Option<String>,
    /// Optional associated emoji.
    pub emoji: Option<String>,
    /// Optional sticker asset.
    pub file: Option<File>,
    /// Whether the sticker is animated.
    pub animated: bool,
    /// Whether the sticker is video-based.
    pub video: bool,
    /// Lossless platform-specific sticker metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// Poll behavior supported by the platform.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum PollType {
    /// Ordinary poll.
    #[default]
    Regular,
    /// Quiz poll.
    Quiz,
    /// Platform-native poll type.
    PlatformNative(String),
}

/// One selectable answer in a poll.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PollOption {
    /// Platform option identifier, if available.
    pub id: Option<String>,
    /// User-visible option text.
    pub text: RichText,
    /// Current vote count, if exposed.
    pub voter_count: Option<u64>,
    /// Lossless platform-specific option metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// Portable poll state and configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Poll {
    /// Platform poll identifier, if available.
    pub id: Option<String>,
    /// Poll question.
    pub question: RichText,
    /// Available answers.
    pub options: Vec<PollOption>,
    /// Whether multiple answers may be selected.
    pub allows_multiple_answers: bool,
    /// Whether users may change their vote.
    pub allows_revoting: Option<bool>,
    /// Whether only conversation members may vote.
    pub members_only: Option<bool>,
    /// Country availability restrictions.
    pub country_codes: Vec<String>,
    /// Whether votes are anonymous.
    pub anonymous: Option<bool>,
    /// Poll behavior.
    pub kind: PollType,
    /// Duration for which the poll remains open.
    pub open_for: Option<Duration>,
    /// Absolute closing time.
    pub closes_at: Option<DateTime<Utc>>,
    /// Total vote count, if exposed.
    pub total_voter_count: Option<u64>,
    /// Retained as a convenience for platforms limited to one correct answer.
    pub correct_option: Option<usize>,
    /// Correct answer indexes for platforms that support multiple correct answers.
    pub correct_options: Vec<usize>,
    /// Optional answer explanation.
    pub explanation: Option<RichText>,
    /// Whether voting is closed.
    pub closed: bool,
    /// Lossless platform-specific poll metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// One task in a portable checklist.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChecklistTask {
    /// Platform task identifier, if available.
    pub id: Option<String>,
    /// User-visible task text.
    pub text: RichText,
    /// Whether the task is complete.
    pub completed: bool,
    /// User that completed the task, if known.
    pub completed_by: Option<User>,
    /// Completion time, if known.
    pub completed_at: Option<DateTime<Utc>>,
    /// Lossless platform-specific task metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// Portable checklist content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Checklist {
    /// Platform checklist identifier, if available.
    pub id: Option<String>,
    /// Checklist title.
    pub title: RichText,
    /// Ordered tasks.
    pub tasks: Vec<ChecklistTask>,
    /// Whether users may add tasks.
    pub can_add_tasks: bool,
    /// Whether users may mark tasks complete.
    pub can_mark_tasks_done: bool,
    /// Lossless platform-specific checklist metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// Incremental change to a checklist.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ChecklistChange {
    /// Message containing the checklist, if known.
    pub message: Option<MessageRef>,
    /// Tasks added by the change.
    pub added_tasks: Vec<ChecklistTask>,
    /// Task identifiers marked complete.
    pub completed_task_ids: Vec<String>,
    /// Task identifiers reopened by the change.
    pub reopened_task_ids: Vec<String>,
    /// User that made the change, if known.
    pub actor: Option<User>,
    /// Lossless platform-specific change metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// Presentation options for a rich layout node.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LayoutStyle {
    /// Optional RGB accent color.
    pub accent_color: Option<u32>,
    /// Whether content is a spoiler.
    pub spoiler: bool,
    /// Whether the client may collapse the content.
    pub collapsible: bool,
    /// Whether collapsible content starts collapsed.
    pub initially_collapsed: bool,
    /// Lossless platform-specific styling.
    pub platform_data: Option<PlatformNativeData>,
}

/// One column in a multi-column rich layout.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayoutColumn {
    /// Optional relative column width.
    pub width: Option<u16>,
    /// Nodes within the column.
    pub nodes: Vec<LayoutNode>,
}

/// One cell in a rich-layout table.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TableCell {
    /// Nodes rendered in the cell.
    pub content: Vec<LayoutNode>,
    /// Whether the cell is a table header.
    pub header: bool,
    /// Number of columns spanned.
    pub colspan: Option<u16>,
    /// Number of rows spanned.
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
