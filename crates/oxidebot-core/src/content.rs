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

/// One node in a structured cross-platform rich layout.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LayoutNode {
    /// Rich-text node.
    Text(RichText),
    /// Section with child nodes and optional accessory.
    Section {
        /// Main section content.
        children: Vec<LayoutNode>,
        /// Optional side accessory.
        accessory: Option<Box<LayoutNode>>,
        /// Section presentation style.
        style: LayoutStyle,
    },
    /// Generic styled container.
    Container {
        /// Contained nodes.
        children: Vec<LayoutNode>,
        /// Container presentation style.
        style: LayoutStyle,
    },
    /// Multiple layout columns.
    Columns(Vec<LayoutColumn>),
    /// Grid layout.
    Grid {
        /// Number of grid columns.
        columns: u16,
        /// Nodes assigned to the grid.
        children: Vec<LayoutNode>,
    },
    /// Image media.
    Image(Media),
    /// Media gallery.
    MediaGallery(Vec<MediaGalleryItem>),
    /// File media.
    File(Media),
    /// Ordered or unordered list.
    List {
        /// Whether the list is ordered.
        ordered: bool,
        /// Nodes for each list item.
        items: Vec<Vec<LayoutNode>>,
    },
    /// Table layout.
    Table {
        /// Table rows and their cells.
        rows: Vec<Vec<TableCell>>,
    },
    /// Quoted rich text.
    Quote(RichText),
    /// Code block.
    Code {
        /// Source text.
        text: String,
        /// Optional programming language identifier.
        language: Option<String>,
    },
    /// Visual divider.
    Divider,
    /// Geographic map location.
    Map(LocationContent),
    /// Collapsible details block.
    Details {
        /// Always-visible summary.
        summary: RichText,
        /// Detail content.
        children: Vec<LayoutNode>,
        /// Whether details start open.
        open: bool,
    },
    /// Interactive action row.
    Actions(ActionRow),
    /// Lossless platform-native layout node.
    PlatformNative(PlatformNativeData),
}

/// Complete rich layout with optional text fallback.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RichLayout {
    /// Root layout nodes.
    pub nodes: Vec<LayoutNode>,
    /// Text used by platforms that cannot render the layout.
    pub fallback_text: Option<String>,
    /// Lossless platform-specific layout metadata.
    pub platform_data: Option<PlatformNativeData>,
}

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

/// Typed value submitted by a command, select, or form field.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum FormValue {
    /// Text value.
    Text(String),
    /// Integer value.
    Integer(i64),
    /// Floating-point value.
    Number(f64),
    /// Boolean value.
    Boolean(bool),
    /// Calendar date.
    Date(NaiveDate),
    /// Clock time.
    Time(NaiveTime),
    /// UTC date-time.
    DateTime(DateTime<Utc>),
    /// Selected user.
    User(User),
    /// Selected conversation.
    Conversation(ConversationRef),
    /// Uploaded or selected file.
    File(File),
    /// Lossless JSON value.
    Json(Value),
}
