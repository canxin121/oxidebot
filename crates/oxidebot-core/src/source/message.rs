use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, error::Error, fmt};

use super::user::User;
use crate::{
    capability::{BotCapabilities, SupportLevel},
    content::{
        Checklist, ContactCard, CustomEmoji, DeliveryTime, LinkPreviewOptions, LocationContent,
        Media, MediaGalleryItem, MediaType, MentionPolicy, MessageVisibility, NotificationPolicy,
        Poll, ReplyOptions, RichLayout, RichText, Sticker,
    },
    conversation::MessageRef,
    interaction::{ActionRow, Button, InlineKeyboard, MessageComponents, PlatformNativeData},
    ConversationId, MessageId, RoleId, UserId,
};

#[path = "message_file.rs"]
mod message_file;

pub use message_file::File;

#[path = "message_delivery.rs"]
mod message_delivery;

use message_delivery::{
    adapt_options, adapt_segment, render_checklist, render_poll, split_for_media_limit,
    split_for_text_limit,
};

/// The single cross-platform message intermediate representation used for
/// inbound events, handler results, active sends, command parsing, delivery
/// planning, and adapter export.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Message {
    /// Platform message ID for an inbound or already-sent message. It is absent
    /// for a newly constructed outgoing message.
    #[serde(default)]
    pub id: Option<MessageId>,
    /// Ordered, lossless message segments.
    #[serde(default)]
    pub segments: Vec<MessageSegment>,
    /// Message-wide delivery and interaction options.
    #[serde(default)]
    pub options: MessageOptions,
}

impl Message {
    /// Creates a message from ordered portable segments.
    #[must_use]
    pub fn new(segments: impl IntoIterator<Item = MessageSegment>) -> Self {
        Self::from_segments(segments)
    }

    /// Creates a one-segment plain-text message.
    #[must_use]
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            id: None,
            segments: vec![MessageSegment::text(content)],
            options: MessageOptions::default(),
        }
    }

    /// Creates a one-segment rich-text message.
    #[must_use]
    pub fn rich_text(content: RichText) -> Self {
        Self::from(MessageSegment::RichText(content))
    }

    /// Creates a message from segments while merging adjacent text segments.
    #[must_use]
    pub fn from_segments(segments: impl IntoIterator<Item = MessageSegment>) -> Self {
        let mut message = Self::default();
        message.extend(segments);
        message
    }

    /// Replaces all message-wide delivery options.
    #[must_use]
    pub fn options(mut self, options: MessageOptions) -> Self {
        self.options = options;
        self
    }

    /// Replaces the message interactive component container.
    #[must_use]
    pub fn components(mut self, components: MessageComponents) -> Self {
        self.options.components = Some(components);
        self
    }

    /// Appends one row of interactive components. Existing inline-keyboard
    /// rows are preserved; another component surface is intentionally replaced
    /// because platforms expose at most one message component container.
    #[must_use]
    pub fn component_row(mut self, row: ActionRow) -> Self {
        match self.options.components.as_mut() {
            Some(MessageComponents::InlineKeyboard(keyboard)) => keyboard.rows.push(row),
            _ => {
                self.options.components =
                    Some(MessageComponents::InlineKeyboard(InlineKeyboard::new([
                        row,
                    ])));
            }
        }
        self
    }

    /// Appends a row of buttons without manually constructing keyboard wrappers.
    #[must_use]
    pub fn buttons(self, buttons: impl IntoIterator<Item = Button>) -> Self {
        self.component_row(ActionRow::buttons(buttons))
    }

    /// Appends one button as a new inline keyboard row.
    #[must_use]
    pub fn button(self, button: Button) -> Self {
        self.buttons([button])
    }

    /// Appends a URL button as a new inline keyboard row.
    #[must_use]
    pub fn button_url(self, label: impl Into<String>, url: impl Into<String>) -> Self {
        self.button(Button::url(label, url))
    }

    /// Appends a callback-action button as a new inline keyboard row.
    #[must_use]
    pub fn button_action(self, label: impl Into<String>, data: impl Into<String>) -> Self {
        self.button(Button::callback(label, data))
    }

    /// Appends a send-text button as a new inline keyboard row.
    #[must_use]
    pub fn button_text(self, label: impl Into<String>, text: impl Into<String>) -> Self {
        self.button(Button::send_text(label, text))
    }

    /// Appends one segment and returns the changed message.
    #[must_use]
    pub fn then(mut self, segment: impl IntoMessageSegment) -> Self {
        self.push(segment);
        self
    }

    /// Appends one segment, merging it with a preceding text segment when possible.
    pub fn push(&mut self, segment: impl IntoMessageSegment) {
        let segment = segment.into_message_segment();
        match segment {
            MessageSegment::Text { content } => {
                if let Some(MessageSegment::Text { content: previous }) = self.segments.last_mut() {
                    previous.push_str(&content);
                } else {
                    self.segments.push(MessageSegment::Text { content });
                }
            }
            segment => self.segments.push(segment),
        }
    }

    /// Appends all segments while preserving text-segment normalization.
    pub fn extend(&mut self, segments: impl IntoIterator<Item = MessageSegment>) {
        for segment in segments {
            self.push(segment);
        }
    }

    /// Appends a user mention.
    #[must_use]
    pub fn at(self, user_id: impl Into<UserId>) -> Self {
        self.then(MessageSegment::at(user_id))
    }

    /// Appends a role mention.
    #[must_use]
    pub fn at_role(self, role_id: impl Into<RoleId>) -> Self {
        self.then(MessageSegment::AtRole {
            role_id: role_id.into(),
        })
    }

    /// Appends a channel mention.
    #[must_use]
    pub fn at_channel(self, channel_id: impl Into<ConversationId>) -> Self {
        self.then(MessageSegment::AtChannel {
            channel_id: channel_id.into(),
        })
    }

    /// Appends an everyone mention.
    #[must_use]
    pub fn at_all(self) -> Self {
        self.then(MessageSegment::at_all())
    }

    /// Sets a reply target by platform message identifier.
    #[must_use]
    pub fn reply_to(mut self, message_id: impl Into<MessageId>) -> Self {
        self.options.reply = Some(ReplyOptions::new(MessageRef::new(message_id)));
        self
    }

    /// Appends a reference to another message.
    #[must_use]
    pub fn reference(self, message_id: impl Into<MessageId>) -> Self {
        self.then(MessageSegment::reference(message_id))
    }

    /// Appends image media.
    #[must_use]
    pub fn image(self, file: File) -> Self {
        self.then(MessageSegment::image(file))
    }

    /// Appends video media with an optional duration.
    #[must_use]
    pub fn video(self, file: File, duration: Option<std::time::Duration>) -> Self {
        self.then(MessageSegment::video(file, duration))
    }

    /// Appends audio media with an optional duration.
    #[must_use]
    pub fn audio(self, file: File, duration: Option<std::time::Duration>) -> Self {
        self.then(MessageSegment::audio(file, duration))
    }

    /// Appends a document attachment.
    #[must_use]
    pub fn file(self, file: File) -> Self {
        self.then(MessageSegment::file(file))
    }

    /// Appends media of an explicit portable kind.
    #[must_use]
    pub fn media(self, kind: MediaType, media: Media) -> Self {
        self.then(MessageSegment::Media {
            kind,
            media: Box::new(media),
        })
    }

    /// Appends a poll.
    #[must_use]
    pub fn poll(self, poll: Poll) -> Self {
        self.then(MessageSegment::Poll(poll))
    }

    /// Appends a rich layout.
    #[must_use]
    pub fn layout(self, layout: RichLayout) -> Self {
        self.then(MessageSegment::Layout(layout))
    }

    /// Appends custom emoji by its platform identifier.
    #[must_use]
    pub fn emoji(self, id: impl Into<String>) -> Self {
        self.then(MessageSegment::emoji(id))
    }

    /// Iterates image and animation files in segment order.
    pub fn image_files(&self) -> impl DoubleEndedIterator<Item = &File> {
        self.segments.iter().filter_map(|segment| match segment {
            MessageSegment::Media {
                kind: MediaType::Image | MediaType::Animation,
                media,
            } => Some(&media.file),
            _ => None,
        })
    }

    /// Iterates document and platform-native attached files in segment order.
    pub fn attached_files(&self) -> impl DoubleEndedIterator<Item = &File> {
        self.segments.iter().filter_map(|segment| match segment {
            MessageSegment::Media {
                kind: MediaType::Document | MediaType::PlatformNative(_),
                media,
            } => Some(&media.file),
            _ => None,
        })
    }

    /// Iterates explicitly mentioned user identifiers in segment order.
    pub fn mentioned_users(&self) -> impl DoubleEndedIterator<Item = &UserId> {
        self.segments.iter().filter_map(|segment| match segment {
            MessageSegment::At { user_id } => Some(user_id),
            _ => None,
        })
    }

    /// Iterates configured reply target identifiers.
    pub fn reply_ids(&self) -> impl DoubleEndedIterator<Item = &MessageId> {
        self.options.reply.iter().map(|reply| &reply.message.id)
    }

    /// Consumes the message and returns its normalized segments.
    #[must_use]
    pub fn into_segments(self) -> Vec<MessageSegment> {
        self.segments
    }

    /// Returns whether the message has no content and no delivery options.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty() && self.options.is_empty()
    }

    /// Returns the number of normalized segments.
    #[must_use]
    pub fn len(&self) -> usize {
        self.segments.len()
    }

    /// Returns whether the first segment starts with `text`.
    #[must_use]
    pub fn starts_with_text(&self, text: &str) -> bool {
        self.segments.first().is_some_and(|segment| match segment {
            MessageSegment::Text { content } => content.starts_with(text),
            MessageSegment::RichText(content) => content.text.starts_with(text),
            _ => false,
        })
    }

    /// Returns a clone with `text` removed from its first matching text segment.
    #[must_use]
    pub fn trim_head_text(&self, text: &str) -> Vec<MessageSegment> {
        let mut segments = self.segments.clone();
        for segment in &mut segments {
            match segment {
                MessageSegment::Text { content } => {
                    if content.starts_with(text) {
                        *content = content.trim_start_matches(text).to_owned();
                        break;
                    }
                }
                MessageSegment::RichText(content) if content.text.starts_with(text) => {
                    content.text = content.text.trim_start_matches(text).to_owned();
                    content.spans.clear();
                    break;
                }
                _ => {}
            }
        }
        segments
    }

    /// Returns a plain-text rendering of all portable segments.
    #[must_use]
    pub fn get_raw_text(&self) -> String {
        self.extract_plain_text()
    }

    /// Returns a plain-text rendering suitable for command parsing.
    #[must_use]
    pub fn extract_plain_text(&self) -> String {
        let capacity = self
            .segments
            .iter()
            .map(MessageSegment::plain_text_len)
            .sum();
        let mut output = String::with_capacity(capacity);
        for segment in &self.segments {
            segment.write_plain_text(&mut output);
        }
        output
    }

    /// Returns whether at least one segment has `kind`.
    #[must_use]
    pub fn has(&self, kind: SegmentKind) -> bool {
        self.segments.iter().any(|segment| segment.kind() == kind)
    }

    /// Returns the first segment with `kind`.
    #[must_use]
    pub fn first(&self, kind: SegmentKind) -> Option<&MessageSegment> {
        self.segments.iter().find(|segment| segment.kind() == kind)
    }

    /// Iterates all segments with `kind`.
    pub fn select(&self, kind: SegmentKind) -> impl DoubleEndedIterator<Item = &MessageSegment> {
        self.segments
            .iter()
            .filter(move |segment| segment.kind() == kind)
    }

    /// Returns a clone that retains only the requested segment kinds.
    #[must_use]
    pub fn include(&self, kinds: &[SegmentKind]) -> Self {
        Self {
            id: self.id.clone(),
            segments: self
                .segments
                .iter()
                .filter(|segment| kinds.contains(&segment.kind()))
                .cloned()
                .collect(),
            options: self.options.clone(),
        }
    }

    /// Returns a clone that omits the requested segment kinds.
    #[must_use]
    pub fn exclude(&self, kinds: &[SegmentKind]) -> Self {
        Self {
            id: self.id.clone(),
            segments: self
                .segments
                .iter()
                .filter(|segment| !kinds.contains(&segment.kind()))
                .cloned()
                .collect(),
            options: self.options.clone(),
        }
    }

    /// Returns a clone produced by mapping or removing each segment.
    #[must_use]
    pub fn map_segments(
        &self,
        mut mapper: impl FnMut(&MessageSegment) -> Option<MessageSegment>,
    ) -> Self {
        Self {
            id: self.id.clone(),
            segments: self.segments.iter().filter_map(&mut mapper).collect(),
            options: self.options.clone(),
        }
    }

    /// Returns whether the message explicitly mentions `user_id`.
    #[must_use]
    pub fn is_related_to_user(&self, user_id: &UserId) -> bool {
        self.segments.iter().any(|segment| match segment {
            MessageSegment::At { user_id: id } => id == user_id,
            _ => false,
        })
    }

    /// Produces a capability-aware physical delivery plan without performing
    /// I/O. The same plan is used by handler replies and active sends.
    pub fn plan_for(
        &self,
        capabilities: &BotCapabilities,
        policy: FallbackPolicy,
    ) -> Result<DeliveryPlan, DeliveryPlanningError> {
        let mut message = Self {
            id: None,
            segments: Vec::with_capacity(self.segments.len()),
            options: self.options.clone(),
        };
        let mut degradations = Vec::new();

        for (index, segment) in self.segments.iter().enumerate() {
            adapt_segment(
                segment,
                index,
                capabilities,
                policy,
                &mut message.segments,
                &mut degradations,
            )?;
        }
        adapt_options(&mut message, capabilities, policy, &mut degradations)?;

        let messages = split_for_media_limit(message, capabilities.limits.max_media_per_message)
            .into_iter()
            .flat_map(|message| split_for_text_limit(message, capabilities.limits.max_text_length))
            .filter(|message| !message.is_empty())
            .collect::<Vec<_>>();
        if messages.is_empty() {
            return Err(DeliveryPlanningError {
                path: "message".to_owned(),
                feature: "deliverable content".to_owned(),
            });
        }
        if messages.len() > 1 {
            degradations.push(DeliveryDegradation {
                path: "message".to_owned(),
                feature: "platform limits".to_owned(),
                kind: DegradationKind::MessageSplit,
                detail: format!(
                    "logical message was split into {} physical messages",
                    messages.len()
                ),
            });
        }
        Ok(DeliveryPlan {
            messages,
            degradations,
        })
    }

    /// Estimates bytes retained by this message and its owned data.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        self.id
            .as_ref()
            .map_or(0, MessageId::estimated_bytes)
            .saturating_add(
                self.segments
                    .iter()
                    .map(MessageSegment::estimated_bytes)
                    .sum::<usize>(),
            )
            .saturating_add(self.options.estimated_bytes())
            .saturating_add(
                self.segments
                    .capacity()
                    .saturating_mul(std::mem::size_of::<MessageSegment>()),
            )
    }
}

/// One ordered, lossless part of a portable [`Message`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MessageSegment {
    /// Plain UTF-8 text.
    Text {
        /// Textual content.
        content: String,
    },
    /// Styled rich text.
    RichText(RichText),
    /// Mention of a user identifier.
    At {
        /// Platform-local user identifier.
        user_id: UserId,
    },
    /// Mention of a role identifier.
    AtRole {
        /// Platform-local role identifier.
        role_id: RoleId,
    },
    /// Mention of a channel identifier.
    AtChannel {
        /// Platform-local channel identifier.
        channel_id: ConversationId,
    },
    /// Mention of all conversation members.
    AtAll,
    /// Reference to an existing platform message.
    Reference {
        /// Referenced platform message identifier.
        message_id: MessageId,
    },
    /// Share-card content.
    Share {
        /// User-visible title.
        title: String,
        /// Optional descriptive text.
        content: Option<String>,
        /// Shared URL.
        url: String,
        /// Optional preview image.
        image: Option<File>,
    },
    /// Reference to a forwarded platform message.
    ForwardNode {
        /// Referenced platform message identifier.
        message_id: MessageId,
    },
    /// Fully modeled forwarded content.
    ForwardCustomNode {
        /// Optional original sender.
        user: Option<User>,
        /// Forwarded portable message.
        message: Box<Message>,
    },
    /// One typed media item.
    Media {
        /// Portable media kind.
        kind: MediaType,
        /// Media metadata and source file.
        media: Box<Media>,
    },
    /// Multiple media items presented as a gallery.
    MediaGallery(Vec<MediaGalleryItem>),
    /// Geographic location content.
    Location(LocationContent),
    /// Contact-card content.
    Contact(ContactCard),
    /// Custom emoji content.
    Emoji(CustomEmoji),
    /// Sticker content.
    Sticker(Sticker),
    /// Poll content.
    Poll(Poll),
    /// Checklist content.
    Checklist(Checklist),
    /// Rich-layout content.
    Layout(RichLayout),
    /// Lossless platform-native content.
    PlatformNative(PlatformNativeData),
}

/// Coarse category used to query [`MessageSegment`] values.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum SegmentKind {
    /// Plain text.
    Text,
    /// Styled rich text.
    RichText,
    /// Image or image gallery.
    Image,
    /// Video content.
    Video,
    /// Audio content.
    Audio,
    /// General file content.
    File,
    /// User mention.
    MentionUser,
    /// Role mention.
    MentionRole,
    /// Channel mention.
    MentionChannel,
    /// Everyone mention.
    MentionAll,
    /// Existing-message reference.
    Reference,
    /// Share card.
    Share,
    /// Geographic location.
    Location,
    /// Custom emoji.
    Emoji,
    /// Forwarded content.
    Forward,
    /// Contact card.
    Contact,
    /// Sticker.
    Sticker,
    /// Poll.
    Poll,
    /// Checklist.
    Checklist,
    /// Rich layout.
    Layout,
    /// Lossless platform-native content.
    PlatformNative,
}

impl MessageSegment {
    /// Returns the coarse category for this segment.
    #[must_use]
    pub fn kind(&self) -> SegmentKind {
        match self {
            Self::Text { .. } => SegmentKind::Text,
            Self::RichText(_) => SegmentKind::RichText,
            Self::Media {
                kind: MediaType::Image | MediaType::Animation,
                ..
            }
            | Self::MediaGallery(_) => SegmentKind::Image,
            Self::Media {
                kind: MediaType::Video | MediaType::VideoNote,
                ..
            } => SegmentKind::Video,
            Self::Media {
                kind: MediaType::Audio | MediaType::VoiceNote,
                ..
            } => SegmentKind::Audio,
            Self::Media {
                kind: MediaType::Document | MediaType::PlatformNative(_),
                ..
            } => SegmentKind::File,
            Self::At { .. } => SegmentKind::MentionUser,
            Self::AtRole { .. } => SegmentKind::MentionRole,
            Self::AtChannel { .. } => SegmentKind::MentionChannel,
            Self::AtAll => SegmentKind::MentionAll,
            Self::Reference { .. } => SegmentKind::Reference,
            Self::ForwardNode { .. } | Self::ForwardCustomNode { .. } => SegmentKind::Forward,
            Self::Share { .. } => SegmentKind::Share,
            Self::Location(_) => SegmentKind::Location,
            Self::Emoji(_) => SegmentKind::Emoji,
            Self::Contact(_) => SegmentKind::Contact,
            Self::Sticker(_) => SegmentKind::Sticker,
            Self::Poll(_) => SegmentKind::Poll,
            Self::Checklist(_) => SegmentKind::Checklist,
            Self::Layout(_) => SegmentKind::Layout,
            Self::PlatformNative(_) => SegmentKind::PlatformNative,
        }
    }

    /// Creates a plain-text segment.
    #[must_use]
    pub fn text(content: impl Into<String>) -> Self {
        Self::Text {
            content: content.into(),
        }
    }

    /// Creates an image-media segment.
    #[must_use]
    pub fn image(file: File) -> Self {
        Self::Media {
            kind: MediaType::Image,
            media: Box::new(Media::new(file)),
        }
    }

    /// Creates a video-media segment.
    #[must_use]
    pub fn video(file: File, duration: Option<std::time::Duration>) -> Self {
        Self::Media {
            kind: MediaType::Video,
            media: Box::new(Media {
                duration,
                ..Media::new(file)
            }),
        }
    }

    /// Creates an audio-media segment.
    #[must_use]
    pub fn audio(file: File, duration: Option<std::time::Duration>) -> Self {
        Self::Media {
            kind: MediaType::Audio,
            media: Box::new(Media {
                duration,
                ..Media::new(file)
            }),
        }
    }

    /// Creates a document-media segment.
    #[must_use]
    pub fn file(file: File) -> Self {
        Self::Media {
            kind: MediaType::Document,
            media: Box::new(Media::new(file)),
        }
    }

    /// Creates a user-mention segment.
    #[must_use]
    pub fn at(user_id: impl Into<UserId>) -> Self {
        Self::At {
            user_id: user_id.into(),
        }
    }

    /// Creates an everyone-mention segment.
    #[must_use]
    pub const fn at_all() -> Self {
        Self::AtAll
    }

    /// Creates an existing-message reference segment.
    #[must_use]
    pub fn reference(message_id: impl Into<MessageId>) -> Self {
        Self::Reference {
            message_id: message_id.into(),
        }
    }

    /// Creates a share-card segment.
    #[must_use]
    pub fn share<T: Into<String>>(
        title: T,
        url: T,
        content: Option<T>,
        image: Option<File>,
    ) -> Self {
        Self::Share {
            title: title.into(),
            content: content.map(Into::into),
            url: url.into(),
            image,
        }
    }

    /// Creates a location segment.
    #[must_use]
    pub fn location<T: Into<String>>(
        latitude: f64,
        longitude: f64,
        title: T,
        content: Option<T>,
    ) -> Self {
        Self::Location(LocationContent {
            latitude,
            longitude,
            title: Some(title.into()),
            address: content.map(Into::into),
            ..LocationContent::default()
        })
    }

    /// Creates a custom-emoji segment.
    #[must_use]
    pub fn emoji(id: impl Into<String>) -> Self {
        Self::Emoji(CustomEmoji {
            id: id.into(),
            name: None,
            fallback: None,
            file: None,
            platform_data: None,
        })
    }

    /// Creates a forwarded-message reference segment.
    #[must_use]
    pub fn forward_node(message_id: impl Into<MessageId>) -> Self {
        Self::ForwardNode {
            message_id: message_id.into(),
        }
    }

    /// Creates a fully modeled forwarded-message segment.
    #[must_use]
    pub fn forward_custom_node(user: Option<User>, message: Message) -> Self {
        Self::ForwardCustomNode {
            user,
            message: Box::new(message),
        }
    }

    /// Returns a user-facing portable category name.
    #[must_use]
    pub fn kind_name(&self) -> &'static str {
        match self.kind() {
            SegmentKind::Text => "plain text",
            SegmentKind::RichText => "rich text",
            SegmentKind::Image => "image",
            SegmentKind::Video => "video",
            SegmentKind::Audio => "audio",
            SegmentKind::File => "file",
            SegmentKind::MentionUser => "user mention",
            SegmentKind::MentionRole => "role mention",
            SegmentKind::MentionChannel => "channel mention",
            SegmentKind::MentionAll => "everyone mention",
            SegmentKind::Reference => "reference",
            SegmentKind::Share => "share",
            SegmentKind::Location => "location",
            SegmentKind::Emoji => "emoji",
            SegmentKind::Forward => "forward",
            SegmentKind::Contact => "contact",
            SegmentKind::Sticker => "sticker",
            SegmentKind::Poll => "poll",
            SegmentKind::Checklist => "checklist",
            SegmentKind::Layout => "rich layout",
            SegmentKind::PlatformNative => "platform-native content",
        }
    }

    fn plain_text_len(&self) -> usize {
        match self {
            Self::Text { content } => content.len(),
            Self::RichText(content) => content.text.len(),
            _ => self.fallback_text().map_or(0, |text| text.len()),
        }
    }

    fn write_plain_text(&self, output: &mut String) {
        match self {
            Self::Text { content } => output.push_str(content),
            Self::RichText(content) => output.push_str(&content.text),
            _ => {
                if let Some(text) = self.fallback_text() {
                    output.push_str(&text);
                }
            }
        }
    }

    /// Renders a lossy plain-text fallback when one is available.
    #[must_use]
    pub fn fallback_text(&self) -> Option<String> {
        match self {
            Self::Text { content } => Some(content.clone()),
            Self::RichText(content) => Some(content.text.clone()),
            Self::At { user_id } => Some(format!("@{user_id}")),
            Self::AtRole { role_id } => Some(format!("@role:{role_id}")),
            Self::AtChannel { channel_id } => Some(format!("#channel:{channel_id}")),
            Self::AtAll => Some("@all".to_owned()),
            Self::Reference { message_id } | Self::ForwardNode { message_id } => {
                Some(format!("[message:{message_id}]"))
            }
            Self::Share {
                title,
                content,
                url,
                ..
            } => Some(match content {
                Some(content) => format!("{title}\n{content}\n{url}"),
                None => format!("{title}\n{url}"),
            }),
            Self::ForwardCustomNode { message, .. } => Some(message.extract_plain_text()),
            Self::Media { kind, media } => media
                .caption
                .as_ref()
                .map(|caption| caption.text.clone())
                .or_else(|| media.alt_text.clone())
                .or_else(|| Some(format!("[{kind:?}: {}]", media.file.name))),
            Self::MediaGallery(items) => Some(format!("[media gallery: {} items]", items.len())),
            Self::Location(location) => Some(format!(
                "{} ({}, {})",
                location
                    .title
                    .clone()
                    .or_else(|| location.address.clone())
                    .unwrap_or_else(|| "location".to_owned()),
                location.latitude,
                location.longitude
            )),
            Self::Contact(contact) => Some(format!(
                "{}{}",
                contact.first_name,
                contact
                    .last_name
                    .as_ref()
                    .map(|name| format!(" {name}"))
                    .unwrap_or_default()
            )),
            Self::Emoji(emoji) => emoji
                .fallback
                .clone()
                .or_else(|| emoji.name.clone())
                .or_else(|| Some(format!(":{}:", emoji.id))),
            Self::Sticker(sticker) => sticker
                .emoji
                .clone()
                .or_else(|| Some("[sticker]".to_owned())),
            Self::Poll(poll) => Some(render_poll(poll)),
            Self::Checklist(checklist) => Some(render_checklist(checklist)),
            Self::Layout(layout) => layout
                .fallback_text
                .clone()
                .or_else(|| Some("[rich layout]".to_owned())),
            Self::PlatformNative(_) => None,
        }
    }

    /// Estimates bytes retained by this segment and owned content.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        match self {
            Self::Text { content } => content.len(),
            Self::RichText(content) => content.text.len().saturating_add(
                content
                    .spans
                    .len()
                    .saturating_mul(std::mem::size_of::<crate::content::TextSpan>()),
            ),
            Self::Reference { message_id } | Self::ForwardNode { message_id } => {
                message_id.estimated_bytes()
            }
            Self::At { user_id } => user_id.estimated_bytes(),
            Self::AtRole { role_id } => role_id.estimated_bytes(),
            Self::AtChannel { channel_id } => channel_id.estimated_bytes(),
            Self::AtAll => 0,
            Self::Share {
                title,
                content,
                url,
                image,
            } => title
                .len()
                .saturating_add(content.as_ref().map_or(0, String::len))
                .saturating_add(url.len())
                .saturating_add(image.as_ref().map_or(0, File::estimated_bytes)),
            Self::ForwardCustomNode { user, message } => user
                .as_ref()
                .map_or(0, |user| user.id.estimated_bytes())
                .saturating_add(message.estimated_bytes()),
            Self::Media { media, .. } => media.file.estimated_bytes().saturating_add(
                media
                    .caption
                    .as_ref()
                    .map_or(0, |caption| caption.text.len()),
            ),
            Self::MediaGallery(items) => items
                .iter()
                .map(|item| item.media.file.estimated_bytes())
                .sum(),
            other => other.fallback_text().map_or(128, |value| value.len() + 128),
        }
    }
}

/// Converts ergonomic message inputs into a normalized [`MessageSegment`].
pub trait IntoMessageSegment {
    /// Consumes this value and produces one message segment.
    fn into_message_segment(self) -> MessageSegment;
}

impl IntoMessageSegment for MessageSegment {
    fn into_message_segment(self) -> MessageSegment {
        self
    }
}

impl IntoMessageSegment for String {
    fn into_message_segment(self) -> MessageSegment {
        MessageSegment::text(self)
    }
}

impl IntoMessageSegment for &str {
    fn into_message_segment(self) -> MessageSegment {
        MessageSegment::text(self)
    }
}

impl IntoMessageSegment for &String {
    fn into_message_segment(self) -> MessageSegment {
        MessageSegment::text(self.clone())
    }
}

impl IntoMessageSegment for RichText {
    fn into_message_segment(self) -> MessageSegment {
        MessageSegment::RichText(self)
    }
}

impl From<MessageSegment> for Message {
    fn from(segment: MessageSegment) -> Self {
        Self::from_segments([segment])
    }
}

impl From<Vec<MessageSegment>> for Message {
    fn from(segments: Vec<MessageSegment>) -> Self {
        Self::from_segments(segments)
    }
}

impl From<String> for Message {
    fn from(text: String) -> Self {
        Self::text(text)
    }
}

impl From<&str> for Message {
    fn from(text: &str) -> Self {
        Self::text(text)
    }
}

impl From<&String> for Message {
    fn from(text: &String) -> Self {
        Self::text(text.clone())
    }
}

impl From<Message> for Vec<MessageSegment> {
    fn from(message: Message) -> Self {
        message.segments
    }
}

/// Message-wide options are part of the same message IR rather than a parallel
/// outgoing-message type.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MessageOptions {
    /// Optional interactive component container.
    pub components: Option<MessageComponents>,
    /// Optional message being replied to.
    pub reply: Option<ReplyOptions>,
    /// Notification behavior for delivery.
    pub notification: NotificationPolicy,
    /// Intended message visibility.
    pub visibility: MessageVisibility,
    /// Optional link-preview preferences.
    pub link_preview: Option<LinkPreviewOptions>,
    /// Optional mention-delivery preferences.
    pub mentions: Option<MentionPolicy>,
    /// Whether platforms should protect content from forwarding or saving.
    pub protect_content: bool,
    /// Immediate or scheduled delivery time.
    pub delivery_time: DeliveryTime,
    /// Optional caller-supplied idempotency key.
    pub idempotency_key: Option<String>,
    /// Optional caller-supplied client message identifier.
    pub client_message_id: Option<String>,
    /// Adapter-independent metadata associated with the message.
    pub metadata: BTreeMap<String, Value>,
    /// Lossless platform-specific delivery metadata.
    pub platform_data: Option<PlatformNativeData>,
}

impl MessageOptions {
    /// Sets the interactive component container.
    #[must_use]
    pub fn components(mut self, components: MessageComponents) -> Self {
        self.components = Some(components);
        self
    }

    /// Sets the reply options.
    #[must_use]
    pub fn reply(mut self, reply: ReplyOptions) -> Self {
        self.reply = Some(reply);
        self
    }

    /// Sets the notification policy.
    #[must_use]
    pub fn notification(mut self, notification: NotificationPolicy) -> Self {
        self.notification = notification;
        self
    }

    /// Sets the intended message visibility.
    #[must_use]
    pub fn visibility(mut self, visibility: MessageVisibility) -> Self {
        self.visibility = visibility;
        self
    }

    /// Sets link-preview preferences.
    #[must_use]
    pub fn link_preview(mut self, link_preview: LinkPreviewOptions) -> Self {
        self.link_preview = Some(link_preview);
        self
    }

    /// Sets mention-delivery preferences.
    #[must_use]
    pub fn mentions(mut self, mentions: MentionPolicy) -> Self {
        self.mentions = Some(mentions);
        self
    }

    /// Enables or disables protected content.
    #[must_use]
    pub fn protect_content(mut self, protect_content: bool) -> Self {
        self.protect_content = protect_content;
        self
    }

    /// Sets immediate or scheduled delivery time.
    #[must_use]
    pub fn delivery_time(mut self, delivery_time: DeliveryTime) -> Self {
        self.delivery_time = delivery_time;
        self
    }

    /// Returns whether all options are their default empty values.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.components.is_none()
            && self.reply.is_none()
            && self.notification == NotificationPolicy::Default
            && self.visibility == MessageVisibility::Public
            && self.link_preview.is_none()
            && self.mentions.is_none()
            && !self.protect_content
            && self.delivery_time == DeliveryTime::Immediate
            && self.idempotency_key.is_none()
            && self.client_message_id.is_none()
            && self.metadata.is_empty()
            && self.platform_data.is_none()
    }

    /// Estimates bytes retained by options and owned metadata.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        self.idempotency_key
            .as_ref()
            .map_or(0, String::len)
            .saturating_add(self.client_message_id.as_ref().map_or(0, String::len))
            .saturating_add(
                self.metadata
                    .iter()
                    .map(|(key, value)| key.len().saturating_add(value.to_string().len()))
                    .sum::<usize>(),
            )
            .saturating_add(256)
    }
}

/// Policy used when a platform cannot represent portable message semantics.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum FallbackPolicy {
    /// Fail on the first unsupported semantic feature.
    Strict,
    /// Drop unsupported features and record every loss.
    DropUnsupported,
    /// Convert unsupported features to their human-readable text representation.
    ToText,
    /// Prefer child/fallback content, then text, then drop.
    Flatten,
    /// Preserve semantics where possible and refuse silent semantic loss.
    #[default]
    Auto,
}

/// Type of semantic degradation recorded during delivery planning.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DegradationKind {
    /// The adapter can preserve the logical feature through a portable
    /// representation rather than a native platform primitive.
    Emulated,
    /// Content was rendered as a text fallback.
    ConvertedToText,
    /// Content was replaced with a supported child or fallback representation.
    Flattened,
    /// Content could not be represented and was omitted.
    Dropped,
    /// A message option could not be represented and was removed.
    OptionRemoved,
    /// One logical message was split to satisfy platform limits.
    MessageSplit,
}

/// One semantic change made while producing a physical delivery plan.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeliveryDegradation {
    /// Location in the logical portable message.
    pub path: String,
    /// Feature that triggered the degradation.
    pub feature: String,
    /// Class of semantic change.
    pub kind: DegradationKind,
    /// Human-readable explanation of the change.
    pub detail: String,
}

/// Exact physical messages and semantic degradations selected before delivery.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeliveryPlan {
    /// Ordered physical messages to send.
    pub messages: Vec<Message>,
    /// Semantic changes required by platform support or limits.
    pub degradations: Vec<DeliveryDegradation>,
}

impl DeliveryPlan {
    /// Returns whether planning required any semantic degradation.
    #[must_use]
    pub fn degraded(&self) -> bool {
        !self.degradations.is_empty()
    }
}

/// Result of a complete or partially completed physical delivery plan.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeliveryReport {
    /// Successfully created message references.
    pub messages: Vec<MessageRef>,
    /// Semantic changes selected by planning.
    pub degradations: Vec<DeliveryDegradation>,
    /// Per-physical-message outcomes in delivery-plan order. This remains
    /// populated on a partial-delivery error so callers can retry or
    /// compensate without duplicating already successful messages.
    #[serde(default)]
    pub items: Vec<DeliveryItemResult>,
}

impl DeliveryReport {
    /// Returns whether planning required any semantic degradation.
    #[must_use]
    pub fn degraded(&self) -> bool {
        !self.degradations.is_empty()
    }

    /// Returns true when every attempted physical message succeeded.
    #[must_use]
    pub fn completed(&self) -> bool {
        self.items.iter().all(DeliveryItemResult::succeeded)
    }

    /// Iterates the failed physical messages without losing successful refs.
    pub fn failures(&self) -> impl Iterator<Item = &DeliveryItemResult> {
        self.items.iter().filter(|item| !item.succeeded())
    }
}

/// Outcome of one physical message in a logical delivery plan.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeliveryItemResult {
    /// Zero-based physical-message index in the delivery plan.
    pub index: usize,
    /// References created by this physical delivery item.
    pub messages: Vec<MessageRef>,
    /// Failure detail when this physical item did not succeed.
    pub error: Option<String>,
}

impl DeliveryItemResult {
    /// Returns whether this physical delivery item succeeded.
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.error.is_none()
    }
}

/// Error returned after a logical delivery has already produced a structured
/// per-item report. The report may contain successful physical messages.
#[derive(Clone, Debug, PartialEq)]
pub struct PartialDeliveryError {
    /// Complete report preserving every successful and failed physical item.
    pub report: DeliveryReport,
}

impl fmt::Display for PartialDeliveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let completed = self
            .report
            .items
            .iter()
            .filter(|item| item.succeeded())
            .count();
        write!(
            formatter,
            "logical delivery stopped after {completed} of {} physical messages succeeded",
            self.report.items.len()
        )
    }
}

impl Error for PartialDeliveryError {}

/// Error returned when strict delivery planning cannot preserve a feature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveryPlanningError {
    /// Location in the logical portable message.
    pub path: String,
    /// Feature that cannot be represented under the selected fallback policy.
    pub feature: String,
}

impl fmt::Display for DeliveryPlanningError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "message feature {:?} at {} is unsupported and the fallback policy forbids degradation",
            self.feature, self.path
        )
    }
}

impl Error for DeliveryPlanningError {}
