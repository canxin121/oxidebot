use crate::{
    BotSlot, CompactId, ConversationKey, InvalidId, PlatformId, RetainedBytes, RetainedSize,
    UserKey,
};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::{collections::BTreeMap, ops::Range, path::PathBuf, sync::Arc};
use thiserror::Error;

/// Maximum number of metadata entries accepted on one message.
pub const MAX_METADATA_ENTRIES: usize = 64;
/// Maximum combined metadata key and raw JSON bytes.
pub const MAX_METADATA_BYTES: usize = 256 * 1024;
/// Maximum bytes accepted for a message idempotency key.
pub const MAX_IDEMPOTENCY_KEY_BYTES: usize = 256;
/// Maximum number of ordered content items in one message.
pub const MAX_MESSAGE_CONTENT_ITEMS: usize = 4_096;
/// Maximum explicit recipients in one portable message target.
pub const MAX_MESSAGE_RECIPIENTS: usize = 4_096;
/// Maximum UTF-8 bytes in one text or rich-text value.
pub const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;
/// Maximum number of style spans in one rich-text value.
pub const MAX_RICH_TEXT_SPANS: usize = 8_192;
/// Maximum styles attached to one rich-text span.
pub const MAX_STYLES_PER_SPAN: usize = 64;
/// Maximum bytes in one link/contact/media descriptor string.
pub const MAX_DESCRIPTOR_BYTES: usize = 64 * 1024;

/// Structural model validation error.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ModelError {
    #[error(transparent)]
    InvalidId(#[from] InvalidId),
    #[error("value references a different bot slot")]
    WrongBot,
    #[error("native data belongs to a different platform")]
    WrongPlatform,
    #[error("rich-text span is outside valid UTF-8 boundaries")]
    InvalidTextSpan,
    #[error("message metadata exceeds its structural limit")]
    MetadataTooLarge,
    #[error("message idempotency key must contain 1..={MAX_IDEMPOTENCY_KEY_BYTES} bytes")]
    InvalidIdempotencyKey,
    #[error("{0} exceeds its structural item limit")]
    CollectionTooLarge(&'static str),
    #[error("{0} exceeds its structural byte limit")]
    ValueTooLarge(&'static str),
}

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
    Bytes(RetainedBytes),
}

impl MediaSource {
    /// Approximate retained bytes.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        match self {
            Self::PlatformId(value) => value.estimated_bytes(),
            Self::Url(value) => value.len(),
            Self::Path(value) => value.as_os_str().as_encoded_bytes().len(),
            Self::Bytes(value) => value.retained_bytes(),
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

fn validate_descriptor(value: &str, label: &'static str) -> Result<(), ModelError> {
    if value.len() > MAX_DESCRIPTOR_BYTES {
        Err(ModelError::ValueTooLarge(label))
    } else {
        Ok(())
    }
}

impl MessageRef {
    /// Validates ownership and external identifier limits.
    pub fn validate_for(&self, bot: BotSlot) -> Result<(), ModelError> {
        if self.conversation.bot != bot {
            return Err(ModelError::WrongBot);
        }
        self.conversation.validate()?;
        self.id.validate()?;
        Ok(())
    }
}

impl MessageTarget {
    /// Validates that all addresses belong to one bot.
    pub fn validate_for(&self, bot: BotSlot) -> Result<(), ModelError> {
        if self.conversation.bot != bot {
            return Err(ModelError::WrongBot);
        }
        self.conversation.validate()?;
        if self.recipients.len() > MAX_MESSAGE_RECIPIENTS {
            return Err(ModelError::CollectionTooLarge("message recipients"));
        }
        for recipient in &self.recipients {
            if recipient.bot != bot {
                return Err(ModelError::WrongBot);
            }
            recipient.validate()?;
        }
        Ok(())
    }
}

impl NativeData {
    /// Verifies that platform-specific data is used by its owning adapter.
    pub fn validate_for(&self, platform: &PlatformId) -> Result<(), ModelError> {
        if &self.platform == platform {
            Ok(())
        } else {
            Err(ModelError::WrongPlatform)
        }
    }
}

impl TextStyle {
    fn validate_for(&self, bot: BotSlot, platform: &PlatformId) -> Result<(), ModelError> {
        match self {
            Self::UserMention(user) => {
                if user.bot != bot {
                    return Err(ModelError::WrongBot);
                }
                user.validate()?;
            }
            Self::Native(native) => native.validate_for(platform)?,
            Self::Link(value) => validate_descriptor(value, "rich-text link")?,
            Self::Bold
            | Self::Italic
            | Self::Underline
            | Self::Strikethrough
            | Self::Code
            | Self::Spoiler => {}
        }
        Ok(())
    }
}

impl RichText {
    /// Validates UTF-8 span boundaries and nested styles.
    pub fn validate_for(&self, bot: BotSlot, platform: &PlatformId) -> Result<(), ModelError> {
        if self.text.len() > MAX_TEXT_BYTES {
            return Err(ModelError::ValueTooLarge("rich text"));
        }
        if self.spans.len() > MAX_RICH_TEXT_SPANS {
            return Err(ModelError::CollectionTooLarge("rich-text spans"));
        }
        for span in &self.spans {
            if span.styles.len() > MAX_STYLES_PER_SPAN {
                return Err(ModelError::CollectionTooLarge("styles per rich-text span"));
            }
            if span.range.start > span.range.end
                || span.range.end > self.text.len()
                || !self.text.is_char_boundary(span.range.start)
                || !self.text.is_char_boundary(span.range.end)
            {
                return Err(ModelError::InvalidTextSpan);
            }
            for style in &span.styles {
                style.validate_for(bot, platform)?;
            }
        }
        Ok(())
    }
}

impl Media {
    /// Validates nested IDs and native data without performing I/O.
    pub fn validate_for(&self, bot: BotSlot, platform: &PlatformId) -> Result<(), ModelError> {
        match &self.source {
            MediaSource::PlatformId(id) => id.validate()?,
            MediaSource::Url(value) => validate_descriptor(value, "media URL")?,
            MediaSource::Path(value) => {
                if value.as_os_str().as_encoded_bytes().len() > MAX_DESCRIPTOR_BYTES {
                    return Err(ModelError::ValueTooLarge("media path"));
                }
            }
            MediaSource::Bytes(_) => {}
        }
        if let Some(name) = &self.name {
            validate_descriptor(name, "media name")?;
        }
        if let Some(mime) = &self.mime {
            validate_descriptor(mime, "media MIME type")?;
        }
        if let Some(caption) = &self.caption {
            caption.validate_for(bot, platform)?;
        }
        if let Some(native) = &self.native {
            native.validate_for(platform)?;
        }
        Ok(())
    }
}

impl MessageContent {
    /// Validates nested portable and native content.
    pub fn validate_for(&self, bot: BotSlot, platform: &PlatformId) -> Result<(), ModelError> {
        match self {
            Self::Text(text) => {
                if text.len() > MAX_TEXT_BYTES {
                    Err(ModelError::ValueTooLarge("plain text"))
                } else {
                    Ok(())
                }
            }
            Self::RichText(text) => text.validate_for(bot, platform),
            Self::Media(media) => media.validate_for(bot, platform),
            Self::Native(native) => native.validate_for(platform),
            Self::Location { label, .. } => {
                if let Some(label) = label {
                    validate_descriptor(label, "location label")?;
                }
                Ok(())
            }
            Self::Contact { name, phone, email } => {
                validate_descriptor(name, "contact name")?;
                if let Some(phone) = phone {
                    validate_descriptor(phone, "contact phone")?;
                }
                if let Some(email) = email {
                    validate_descriptor(email, "contact email")?;
                }
                Ok(())
            }
        }
    }
}

impl MessageOptions {
    /// Validates reply ownership, native data, and bounded metadata.
    pub fn validate_for(&self, bot: BotSlot, platform: &PlatformId) -> Result<(), ModelError> {
        if let Some(reply) = &self.reply_to {
            reply.validate_for(bot)?;
        }
        if let Some(native) = &self.native {
            native.validate_for(platform)?;
        }
        if self
            .idempotency_key
            .as_ref()
            .is_some_and(|key| key.is_empty() || key.len() > MAX_IDEMPOTENCY_KEY_BYTES)
        {
            return Err(ModelError::InvalidIdempotencyKey);
        }
        if self.metadata.len() > MAX_METADATA_ENTRIES {
            return Err(ModelError::MetadataTooLarge);
        }
        let metadata_bytes = self
            .metadata
            .iter()
            .map(|(key, value)| key.len().saturating_add(value.get().len()))
            .sum::<usize>();
        if metadata_bytes > MAX_METADATA_BYTES {
            return Err(ModelError::MetadataTooLarge);
        }
        Ok(())
    }
}

impl OutgoingMessage {
    /// Validates a command payload before it enters a bot queue.
    pub fn validate_for(&self, bot: BotSlot, platform: &PlatformId) -> Result<(), ModelError> {
        if self.content.len() > MAX_MESSAGE_CONTENT_ITEMS {
            return Err(ModelError::CollectionTooLarge("message content"));
        }
        for content in &self.content {
            content.validate_for(bot, platform)?;
        }
        self.options.validate_for(bot, platform)
    }
}

impl RetainedSize for MessageRef {
    fn retained_bytes(&self) -> usize {
        self.estimated_bytes()
    }
}

impl RetainedSize for MessageTarget {
    fn retained_bytes(&self) -> usize {
        self.estimated_bytes()
    }
}

impl RetainedSize for NativeData {
    fn retained_bytes(&self) -> usize {
        self.estimated_bytes()
            .saturating_add(std::mem::size_of::<Self>())
    }
}

impl RetainedSize for OutgoingMessage {
    fn retained_bytes(&self) -> usize {
        self.estimated_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn platform() -> PlatformId {
        PlatformId::new("test").expect("static platform")
    }

    #[test]
    fn empty_idempotency_keys_are_rejected() {
        let message = OutgoingMessage::text("hello").options(MessageOptions {
            idempotency_key: Some(Arc::from("")),
            ..MessageOptions::default()
        });
        assert_eq!(
            message.validate_for(BotSlot(0), &platform()),
            Err(ModelError::InvalidIdempotencyKey)
        );
    }

    #[test]
    fn text_and_collection_limits_are_enforced() {
        let too_large = MessageContent::Text(Arc::from("x".repeat(MAX_TEXT_BYTES + 1)));
        assert_eq!(
            too_large.validate_for(BotSlot(0), &platform()),
            Err(ModelError::ValueTooLarge("plain text"))
        );

        let target = MessageTarget::new(ConversationKey::new(BotSlot(0), 1_u64)).recipients(
            (0..=MAX_MESSAGE_RECIPIENTS).map(|value| UserKey::new(BotSlot(0), value as u64)),
        );
        assert_eq!(
            target.validate_for(BotSlot(0)),
            Err(ModelError::CollectionTooLarge("message recipients"))
        );
    }
}
