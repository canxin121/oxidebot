use crate::{CompactId, ConversationKey, PlatformId, UserKey};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::{collections::BTreeMap, ops::Range, path::PathBuf, sync::Arc};

/// Reference to one platform message.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct MessageRef {
    /// Conversation containing the message.
    pub conversation: ConversationKey,
    /// Platform message identifier.
    pub id: CompactId,
}

impl MessageRef {
    /// Creates a message reference.
    #[must_use]
    pub fn new(conversation: ConversationKey, id: impl Into<CompactId>) -> Self {
        Self {
            conversation,
            id: id.into(),
        }
    }

    /// Approximate retained bytes.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        self.conversation
            .estimated_bytes()
            .saturating_add(self.id.estimated_bytes())
            .saturating_add(32)
    }
}

/// Receipt returned after one platform message is accepted.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct MessageReceipt {
    /// Reference assigned by the platform.
    pub reference: MessageRef,
}

impl MessageReceipt {
    /// Creates a receipt.
    #[must_use]
    pub fn new(reference: MessageRef) -> Self {
        Self { reference }
    }
}

/// Destination for an outgoing message.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct MessageTarget {
    /// Destination conversation.
    pub conversation: ConversationKey,
    /// Optional explicit recipients inside that conversation.
    pub recipients: Vec<UserKey>,
}

impl MessageTarget {
    /// Creates a target for an entire conversation.
    #[must_use]
    pub fn new(conversation: ConversationKey) -> Self {
        Self {
            conversation,
            recipients: Vec::new(),
        }
    }

    /// Adds explicit recipients.
    #[must_use]
    pub fn recipients(mut self, recipients: impl IntoIterator<Item = UserKey>) -> Self {
        self.recipients.extend(recipients);
        self
    }

    /// Approximate retained bytes.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        self.conversation
            .estimated_bytes()
            .saturating_add(
                self.recipients
                    .iter()
                    .map(UserKey::estimated_bytes)
                    .sum::<usize>(),
            )
            .saturating_add(self.recipients.capacity() * std::mem::size_of::<UserKey>())
            .saturating_add(32)
    }
}

/// Lossless platform-specific extension data.
#[derive(Clone, Debug)]
pub struct NativeData {
    /// Platform that owns the data.
    pub platform: PlatformId,
    /// Raw JSON retained without conversion to a `Value` tree.
    pub data: Arc<RawValue>,
}

impl PartialEq for NativeData {
    fn eq(&self, other: &Self) -> bool {
        self.platform == other.platform && self.data.get() == other.data.get()
    }
}

impl NativeData {
    /// Creates lossless platform-native data.
    #[must_use]
    pub fn new(platform: PlatformId, data: Arc<RawValue>) -> Self {
        Self { platform, data }
    }

    /// Approximate retained bytes.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        self.data
            .get()
            .len()
            .saturating_add(self.platform.as_str().len())
    }
}

/// Source of a media object. Describing a source never performs I/O.
#[derive(Clone, Debug, PartialEq)]
pub enum MediaSource {
    /// Existing platform file identifier.
    PlatformId(CompactId),
    /// Remote URL resolved by an adapter or media service.
    Url(Arc<str>),
    /// Local path opened only by an adapter or media service.
    Path(Arc<PathBuf>),
    /// Shared in-memory bytes.
    Bytes(Bytes),
}

impl MediaSource {
    /// Approximate retained bytes.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        match self {
            Self::PlatformId(value) => value.estimated_bytes(),
            Self::Url(value) => value.len(),
            Self::Path(value) => value.as_os_str().as_encoded_bytes().len(),
            Self::Bytes(value) => value.len(),
        }
    }
}

/// Media metadata and source.
#[derive(Clone, Debug, PartialEq)]
pub struct Media {
    /// Source of the content.
    pub source: MediaSource,
    /// Optional filename.
    pub name: Option<Arc<str>>,
    /// Optional MIME type.
    pub mime: Option<Arc<str>>,
    /// Optional known size.
    pub size: Option<u64>,
    /// Optional caption.
    pub caption: Option<RichText>,
    /// Optional platform-native fields.
    pub native: Option<NativeData>,
}

impl Media {
    /// Approximate retained bytes.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        self.source
            .estimated_bytes()
            .saturating_add(self.name.as_ref().map_or(0, |value| value.len()))
            .saturating_add(self.mime.as_ref().map_or(0, |value| value.len()))
            .saturating_add(self.caption.as_ref().map_or(0, RichText::estimated_bytes))
            .saturating_add(self.native.as_ref().map_or(0, NativeData::estimated_bytes))
            .saturating_add(128)
    }
}

/// Rich-text style.
#[derive(Clone, Debug, PartialEq)]
pub enum TextStyle {
    /// Bold text.
    Bold,
    /// Italic text.
    Italic,
    /// Underlined text.
    Underline,
    /// Struck-through text.
    Strikethrough,
    /// Inline code.
    Code,
    /// Spoiler text.
    Spoiler,
    /// Hyperlink.
    Link(Arc<str>),
    /// Mention one user.
    UserMention(UserKey),
    /// Platform-native text style.
    Native(NativeData),
}

impl TextStyle {
    fn estimated_bytes(&self) -> usize {
        match self {
            Self::Link(value) => value.len(),
            Self::UserMention(value) => value.estimated_bytes(),
            Self::Native(value) => value.estimated_bytes(),
            Self::Bold
            | Self::Italic
            | Self::Underline
            | Self::Strikethrough
            | Self::Code
            | Self::Spoiler => 0,
        }
    }
}

/// A styled byte range in UTF-8 text.
#[derive(Clone, Debug, PartialEq)]
pub struct TextSpan {
    /// UTF-8 byte range.
    pub range: Range<usize>,
    /// Styles applied to the range.
    pub styles: Vec<TextStyle>,
}

/// Portable rich text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RichText {
    /// UTF-8 text.
    pub text: Arc<str>,
    /// Styled ranges.
    pub spans: Vec<TextSpan>,
}

impl RichText {
    /// Creates unstyled rich text.
    #[must_use]
    pub fn plain(text: impl Into<Arc<str>>) -> Self {
        Self {
            text: text.into(),
            spans: Vec::new(),
        }
    }

    /// Approximate retained bytes.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        self.text
            .len()
            .saturating_add(self.spans.capacity() * std::mem::size_of::<TextSpan>())
            .saturating_add(
                self.spans
                    .iter()
                    .flat_map(|span| span.styles.iter())
                    .map(TextStyle::estimated_bytes)
                    .sum::<usize>(),
            )
            .saturating_add(
                self.spans
                    .iter()
                    .map(|span| span.styles.capacity() * std::mem::size_of::<TextStyle>())
                    .sum::<usize>(),
            )
    }
}

/// Ordered content inside a message.
#[derive(Clone, Debug, PartialEq)]
pub enum MessageContent {
    /// Plain text.
    Text(Arc<str>),
    /// Styled text.
    RichText(RichText),
    /// Media attachment.
    Media(Media),
    /// Geographic location.
    Location {
        /// Latitude.
        latitude: f64,
        /// Longitude.
        longitude: f64,
        /// Optional label.
        label: Option<Arc<str>>,
    },
    /// Contact card.
    Contact {
        /// Display name.
        name: Arc<str>,
        /// Optional phone number.
        phone: Option<Arc<str>>,
        /// Optional email address.
        email: Option<Arc<str>>,
    },
    /// Platform-native content.
    Native(NativeData),
}

impl MessageContent {
    /// Approximate retained bytes used by bounded queues.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        match self {
            Self::Text(text) => text.len(),
            Self::RichText(text) => text.estimated_bytes(),
            Self::Media(media) => media.estimated_bytes(),
            Self::Location { label, .. } => label.as_ref().map_or(32, |label| label.len() + 32),
            Self::Contact { name, phone, email } => name
                .len()
                .saturating_add(phone.as_ref().map_or(0, |value| value.len()))
                .saturating_add(email.as_ref().map_or(0, |value| value.len())),
            Self::Native(native) => native.estimated_bytes(),
        }
    }
}

/// Notification behavior for an outgoing message.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NotificationPolicy {
    /// Platform default.
    #[default]
    Default,
    /// Suppress notifications where supported.
    Silent,
    /// Force a notification where supported.
    Force,
}

/// Message-wide delivery options.
#[derive(Clone, Debug, Default)]
pub struct MessageOptions {
    /// Optional reply target.
    pub reply_to: Option<MessageRef>,
    /// Notification behavior.
    pub notification: NotificationPolicy,
    /// Prevent forwarding or saving where supported.
    pub protected: bool,
    /// Optional idempotency key.
    pub idempotency_key: Option<Arc<str>>,
    /// Portable metadata.
    pub metadata: BTreeMap<Arc<str>, Arc<RawValue>>,
    /// Optional platform-native fields.
    pub native: Option<NativeData>,
}

impl PartialEq for MessageOptions {
    fn eq(&self, other: &Self) -> bool {
        self.reply_to == other.reply_to
            && self.notification == other.notification
            && self.protected == other.protected
            && self.idempotency_key == other.idempotency_key
            && self.native == other.native
            && self.metadata.len() == other.metadata.len()
            && self.metadata.iter().all(|(key, value)| {
                other
                    .metadata
                    .get(key)
                    .is_some_and(|other_value| value.get() == other_value.get())
            })
    }
}

impl MessageOptions {
    /// Approximate retained bytes.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        self.reply_to
            .as_ref()
            .map_or(0, MessageRef::estimated_bytes)
            .saturating_add(self.idempotency_key.as_ref().map_or(0, |value| value.len()))
            .saturating_add(
                self.metadata
                    .iter()
                    .map(|(key, value)| {
                        key.len()
                            .saturating_add(value.get().len())
                            .saturating_add(64)
                    })
                    .sum::<usize>(),
            )
            .saturating_add(self.native.as_ref().map_or(0, NativeData::estimated_bytes))
            .saturating_add(64)
    }
}

/// Portable outgoing message.
#[derive(Clone, Debug, PartialEq)]
pub struct OutgoingMessage {
    /// Ordered content.
    pub content: Vec<MessageContent>,
    /// Message-wide options.
    pub options: MessageOptions,
}

impl OutgoingMessage {
    /// Creates a message from ordered content.
    #[must_use]
    pub fn new(content: impl IntoIterator<Item = MessageContent>) -> Self {
        Self {
            content: content.into_iter().collect(),
            options: MessageOptions::default(),
        }
    }

    /// Creates a plain text message.
    #[must_use]
    pub fn text(text: impl Into<Arc<str>>) -> Self {
        Self::new([MessageContent::Text(text.into())])
    }

    /// Replaces message-wide options.
    #[must_use]
    pub fn options(mut self, options: MessageOptions) -> Self {
        self.options = options;
        self
    }

    /// Approximate retained bytes used by bounded command queues.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        self.content
            .iter()
            .map(MessageContent::estimated_bytes)
            .sum::<usize>()
            .saturating_add(self.content.capacity() * std::mem::size_of::<MessageContent>())
            .saturating_add(self.options.estimated_bytes())
            .saturating_add(128)
    }
}

impl From<&str> for OutgoingMessage {
    fn from(value: &str) -> Self {
        Self::text(value)
    }
}

impl From<String> for OutgoingMessage {
    fn from(value: String) -> Self {
        Self::text(Arc::<str>::from(value))
    }
}
