//! Portable message content, rich text, rich layout, media, poll, checklist,
//! contact, and delivery models.

use std::{collections::BTreeMap, error::Error, fmt, ops::Range, time::Duration};

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use serde_json::Value;

use crate::{
    collaboration::ReactionSummary,
    conversation::{ConversationRef, MessageRef, MessageTarget},
    interaction::{ActionRow, MessageOptions, PlatformNativeData},
    source::{
        message::{File, MessageSegment},
        user::User,
    },
};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RichText {
    pub text: String,
    pub spans: Vec<TextSpan>,
}

impl RichText {
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            spans: Vec::new(),
        }
    }

    pub fn span(mut self, range: Range<usize>, style: TextStyle) -> Self {
        self.spans.push(TextSpan {
            range,
            styles: vec![style],
        });
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextSpan {
    /// Byte offsets into [`RichText::text`]. Adapters must validate UTF-8
    /// boundaries before platform-specific unit conversion (for example,
    /// Telegram and Discord UTF-16 offsets).
    pub range: Range<usize>,
    pub styles: Vec<TextStyle>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TextStyle {
    Bold,
    Italic,
    Underline,
    Strikethrough,
    Spoiler,
    Code,
    Preformatted {
        language: Option<String>,
    },
    Link {
        url: String,
    },
    UserMention {
        user_id: String,
    },
    RoleMention {
        role_id: String,
    },
    ConversationMention {
        conversation_id: String,
    },
    CustomEmoji {
        id: String,
        fallback: Option<String>,
    },
    Quote,
    Marked,
    Subscript,
    Superscript,
    Hashtag,
    Cashtag,
    BotCommand,
    Email,
    Phone,
    DateTime {
        timestamp: Option<DateTime<Utc>>,
    },
    MathematicalExpression,
    Reference {
        message: Option<MessageRef>,
    },
    PlatformNative(PlatformNativeData),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Media {
    pub file: File,
    pub caption: Option<RichText>,
    pub thumbnail: Option<File>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration: Option<Duration>,
    pub spoiler: bool,
    pub alt_text: Option<String>,
    pub waveform: Option<Vec<u8>>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum MediaType {
    #[default]
    Image,
    Video,
    Audio,
    Document,
    Animation,
    VoiceNote,
    VideoNote,
    PlatformNative(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct MediaGalleryItem {
    pub kind: MediaType,
    pub media: Media,
}

impl Media {
    pub fn new(file: File) -> Self {
        Self {
            file,
            ..Default::default()
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LocationContent {
    pub latitude: f64,
    pub longitude: f64,
    pub title: Option<String>,
    pub address: Option<String>,
    pub horizontal_accuracy: Option<f64>,
    pub live_period: Option<Duration>,
    pub heading: Option<u16>,
    pub proximity_alert_radius: Option<u32>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PhoneNumber {
    pub label: Option<String>,
    pub value: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
pub struct CustomEmoji {
    pub id: String,
    pub name: Option<String>,
    pub fallback: Option<String>,
    pub file: Option<File>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sticker {
    pub id: String,
    pub pack_id: Option<String>,
    pub emoji: Option<String>,
    pub file: Option<File>,
    pub animated: bool,
    pub video: bool,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum PollType {
    #[default]
    Regular,
    Quiz,
    PlatformNative(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PollOption {
    pub id: Option<String>,
    pub text: RichText,
    pub voter_count: Option<u64>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
pub struct ChecklistTask {
    pub id: Option<String>,
    pub text: RichText,
    pub completed: bool,
    pub completed_by: Option<User>,
    pub completed_at: Option<DateTime<Utc>>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Checklist {
    pub id: Option<String>,
    pub title: RichText,
    pub tasks: Vec<ChecklistTask>,
    pub can_add_tasks: bool,
    pub can_mark_tasks_done: bool,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChecklistChange {
    pub message: Option<MessageRef>,
    pub added_tasks: Vec<ChecklistTask>,
    pub completed_task_ids: Vec<String>,
    pub reopened_task_ids: Vec<String>,
    pub actor: Option<User>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutStyle {
    pub accent_color: Option<u32>,
    pub spoiler: bool,
    pub collapsible: bool,
    pub initially_collapsed: bool,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutColumn {
    pub width: Option<u16>,
    pub nodes: Vec<LayoutNode>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TableCell {
    pub content: Vec<LayoutNode>,
    pub header: bool,
    pub colspan: Option<u16>,
    pub rowspan: Option<u16>,
}

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RichLayout {
    pub nodes: Vec<LayoutNode>,
    pub fallback_text: Option<String>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MessageContent {
    PlainText(String),
    RichText(RichText),
    Image(Media),
    Video(Media),
    Audio(Media),
    Animation(Media),
    VoiceNote(Media),
    VideoNote(Media),
    File(Media),
    MediaGallery(Vec<MediaGalleryItem>),
    Location(LocationContent),
    Contact(ContactCard),
    CustomEmoji(CustomEmoji),
    Sticker(Sticker),
    Poll(Poll),
    Checklist(Checklist),
    RichLayout(RichLayout),
    PlatformNative(PlatformNativeData),
}

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum NotificationPolicy {
    #[default]
    Default,
    Silent,
    Force,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum MessageVisibility {
    #[default]
    Public,
    Ephemeral,
    PrivateTo(Vec<String>),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkPreviewOptions {
    pub enabled: bool,
    pub prefer_large_media: Option<bool>,
    pub above_text: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum MentionAllowance {
    #[default]
    PlatformDefault,
    None,
    All,
    Only(Vec<String>),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MentionPolicy {
    pub users: MentionAllowance,
    pub roles: MentionAllowance,
    pub conversations: MentionAllowance,
    pub everyone: bool,
    pub reply_author: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum DeliveryTime {
    #[default]
    Immediate,
    Scheduled(DateTime<Utc>),
    Draft,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutgoingMessage {
    pub content: Vec<MessageContent>,
    pub options: MessageOptions,
}

impl OutgoingMessage {
    pub fn new(content: impl IntoIterator<Item = MessageContent>) -> Self {
        Self {
            content: content.into_iter().collect(),
            options: MessageOptions::default(),
        }
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self::new([MessageContent::PlainText(text.into())])
    }

    pub fn options(mut self, options: MessageOptions) -> Self {
        self.options = options;
        self
    }

    pub fn try_into_legacy(
        self,
    ) -> Result<(Vec<MessageSegment>, MessageOptions), ContentConversionError> {
        let mut segments = Vec::with_capacity(self.content.len());
        for content in self.content {
            segments.push(content.try_into()?);
        }
        Ok((segments, self.options))
    }
}

impl TryFrom<MessageContent> for MessageSegment {
    type Error = ContentConversionError;

    fn try_from(content: MessageContent) -> Result<Self, Self::Error> {
        match content {
            MessageContent::PlainText(text) => Ok(Self::text(text)),
            MessageContent::RichText(text) if text.spans.is_empty() => Ok(Self::text(text.text)),
            MessageContent::Image(media) if media_is_legacy(&media) => Ok(Self::image(media.file)),
            MessageContent::Video(media) if media_is_legacy(&media) => Ok(Self::video(
                media.file,
                media
                    .duration
                    .and_then(|value| i32::try_from(value.as_secs()).ok()),
            )),
            MessageContent::Audio(media) if media_is_legacy(&media) => Ok(Self::audio(
                media.file,
                media
                    .duration
                    .and_then(|value| i32::try_from(value.as_secs()).ok()),
            )),
            MessageContent::File(media) if media_is_legacy(&media) => Ok(Self::file(media.file)),
            MessageContent::Location(location)
                if location.horizontal_accuracy.is_none()
                    && location.live_period.is_none()
                    && location.heading.is_none()
                    && location.proximity_alert_radius.is_none()
                    && location.platform_data.is_none() =>
            {
                Ok(Self::location(
                    location.latitude,
                    location.longitude,
                    location.title.unwrap_or_default(),
                    location.address,
                ))
            }
            other => Err(ContentConversionError {
                kind: other.kind_name().to_owned(),
            }),
        }
    }
}

fn media_is_legacy(media: &Media) -> bool {
    media.caption.is_none()
        && media.thumbnail.is_none()
        && media.width.is_none()
        && media.height.is_none()
        && !media.spoiler
        && media.alt_text.is_none()
        && media.waveform.is_none()
        && media.platform_data.is_none()
}

impl MessageContent {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::PlainText(_) => "plain text",
            Self::RichText(_) => "rich text",
            Self::Image(_) => "image",
            Self::Video(_) => "video",
            Self::Audio(_) => "audio",
            Self::Animation(_) => "animation",
            Self::VoiceNote(_) => "voice note",
            Self::VideoNote(_) => "video note",
            Self::File(_) => "file",
            Self::MediaGallery(_) => "media gallery",
            Self::Location(_) => "location",
            Self::Contact(_) => "contact",
            Self::CustomEmoji(_) => "custom emoji",
            Self::Sticker(_) => "sticker",
            Self::Poll(_) => "poll",
            Self::Checklist(_) => "checklist",
            Self::RichLayout(_) => "rich layout",
            Self::PlatformNative(_) => "platform-native content",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentConversionError {
    pub kind: String,
}

impl fmt::Display for ContentConversionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "message content {:?} cannot be represented by the legacy MessageSegment API",
            self.kind
        )
    }
}

impl Error for ContentConversionError {}

#[derive(Clone, Debug, PartialEq)]
pub struct MessageEnvelope {
    pub reference: MessageRef,
    pub sender: Option<User>,
    pub conversation: Option<ConversationRef>,
    pub created_at: Option<DateTime<Utc>>,
    pub edited_at: Option<DateTime<Utc>>,
    pub content: Vec<MessageContent>,
    pub reply_to: Option<MessageRef>,
    pub reply_context: Option<ReplyContext>,
    pub forwarded_from: Option<MessageRef>,
    pub forward_context: Option<ForwardContext>,
    pub reactions: Vec<ReactionSummary>,
    pub pinned: Option<bool>,
    pub metadata: BTreeMap<String, Value>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReplyContext {
    pub message: Option<MessageRef>,
    pub quote: Option<RichText>,
    pub quote_position: Option<u32>,
    pub external_origin: Option<MessageOrigin>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
pub struct ForwardContext {
    pub origin: MessageOrigin,
    pub sent_at: Option<DateTime<Utc>>,
    pub automatically_forwarded: bool,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, Default, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
pub struct BatchMessage {
    pub target: MessageTarget,
    pub message: OutgoingMessage,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BatchItemResult {
    pub index: usize,
    pub message: Option<MessageRef>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BatchSendResult {
    pub items: Vec<BatchItemResult>,
}

#[derive(Clone, Debug, PartialEq)]
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
