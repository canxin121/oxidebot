use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

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
};

/// The single cross-platform message intermediate representation used for
/// inbound events, handler results, active sends, command parsing, delivery
/// planning, and adapter export.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Message {
    /// Platform message id for an inbound or already-sent message. It is empty
    /// for a newly constructed outgoing message.
    #[serde(default)]
    pub id: String,
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
            id: String::new(),
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
    pub fn at(self, user_id: impl Into<String>) -> Self {
        self.then(MessageSegment::at(user_id))
    }

    /// Appends a role mention.
    #[must_use]
    pub fn at_role(self, role_id: impl Into<String>) -> Self {
        self.then(MessageSegment::AtRole {
            role_id: role_id.into(),
        })
    }

    /// Appends a channel mention.
    #[must_use]
    pub fn at_channel(self, channel_id: impl Into<String>) -> Self {
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
    pub fn reply_to(mut self, message_id: impl Into<String>) -> Self {
        self.options.reply = Some(ReplyOptions::new(MessageRef::new(message_id.into())));
        self
    }

    /// Appends a reference to another message.
    #[must_use]
    pub fn reference(self, message_id: impl Into<String>) -> Self {
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
    pub fn mentioned_users(&self) -> impl DoubleEndedIterator<Item = &str> {
        self.segments.iter().filter_map(|segment| match segment {
            MessageSegment::At { user_id } => Some(user_id.as_str()),
            _ => None,
        })
    }

    /// Iterates configured reply target identifiers.
    pub fn reply_ids(&self) -> impl DoubleEndedIterator<Item = &str> {
        self.options
            .reply
            .iter()
            .map(|reply| reply.message.id.as_str())
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
    pub fn is_related_to_user(&self, user_id: &str) -> bool {
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
            id: String::new(),
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
            .len()
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MessageSegment {
    Text {
        content: String,
    },
    RichText(RichText),
    At {
        user_id: String,
    },
    AtRole {
        role_id: String,
    },
    AtChannel {
        channel_id: String,
    },
    AtAll,
    Reference {
        message_id: String,
    },
    Share {
        title: String,
        content: Option<String>,
        url: String,
        image: Option<File>,
    },
    ForwardNode {
        message_id: String,
    },
    ForwardCustomNode {
        user: Option<User>,
        message: Box<Message>,
    },
    Media {
        kind: MediaType,
        media: Box<Media>,
    },
    MediaGallery(Vec<MediaGalleryItem>),
    Location(LocationContent),
    Contact(ContactCard),
    Emoji(CustomEmoji),
    Sticker(Sticker),
    Poll(Poll),
    Checklist(Checklist),
    Layout(RichLayout),
    PlatformNative(PlatformNativeData),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum SegmentKind {
    Text,
    RichText,
    Image,
    Video,
    Audio,
    File,
    MentionUser,
    MentionRole,
    MentionChannel,
    MentionAll,
    Reference,
    Share,
    Location,
    Emoji,
    Forward,
    Contact,
    Sticker,
    Poll,
    Checklist,
    Layout,
    PlatformNative,
}

impl MessageSegment {
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

    #[must_use]
    pub fn text(content: impl Into<String>) -> Self {
        Self::Text {
            content: content.into(),
        }
    }

    #[must_use]
    pub fn image(file: File) -> Self {
        Self::Media {
            kind: MediaType::Image,
            media: Box::new(Media::new(file)),
        }
    }

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

    #[must_use]
    pub fn file(file: File) -> Self {
        Self::Media {
            kind: MediaType::Document,
            media: Box::new(Media::new(file)),
        }
    }

    #[must_use]
    pub fn at(user_id: impl Into<String>) -> Self {
        Self::At {
            user_id: user_id.into(),
        }
    }

    #[must_use]
    pub const fn at_all() -> Self {
        Self::AtAll
    }

    #[must_use]
    pub fn reference(message_id: impl Into<String>) -> Self {
        Self::Reference {
            message_id: message_id.into(),
        }
    }

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

    #[must_use]
    pub fn forward_node(message_id: impl Into<String>) -> Self {
        Self::ForwardNode {
            message_id: message_id.into(),
        }
    }

    #[must_use]
    pub fn forward_custom_node(user: Option<User>, message: Message) -> Self {
        Self::ForwardCustomNode {
            user,
            message: Box::new(message),
        }
    }

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
            Self::Reference { message_id }
            | Self::ForwardNode { message_id }
            | Self::At {
                user_id: message_id,
            }
            | Self::AtRole {
                role_id: message_id,
            }
            | Self::AtChannel {
                channel_id: message_id,
            } => message_id.len(),
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
                .map_or(0, |user| user.id.len())
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

pub trait IntoMessageSegment {
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
    pub components: Option<MessageComponents>,
    pub reply: Option<ReplyOptions>,
    pub notification: NotificationPolicy,
    pub visibility: MessageVisibility,
    pub link_preview: Option<LinkPreviewOptions>,
    pub mentions: Option<MentionPolicy>,
    pub protect_content: bool,
    pub delivery_time: DeliveryTime,
    pub idempotency_key: Option<String>,
    pub client_message_id: Option<String>,
    pub metadata: BTreeMap<String, Value>,
    pub platform_data: Option<PlatformNativeData>,
}

impl MessageOptions {
    #[must_use]
    pub fn components(mut self, components: MessageComponents) -> Self {
        self.components = Some(components);
        self
    }

    #[must_use]
    pub fn reply(mut self, reply: ReplyOptions) -> Self {
        self.reply = Some(reply);
        self
    }

    #[must_use]
    pub fn notification(mut self, notification: NotificationPolicy) -> Self {
        self.notification = notification;
        self
    }

    #[must_use]
    pub fn visibility(mut self, visibility: MessageVisibility) -> Self {
        self.visibility = visibility;
        self
    }

    #[must_use]
    pub fn link_preview(mut self, link_preview: LinkPreviewOptions) -> Self {
        self.link_preview = Some(link_preview);
        self
    }

    #[must_use]
    pub fn mentions(mut self, mentions: MentionPolicy) -> Self {
        self.mentions = Some(mentions);
        self
    }

    #[must_use]
    pub fn protect_content(mut self, protect_content: bool) -> Self {
        self.protect_content = protect_content;
        self
    }

    #[must_use]
    pub fn delivery_time(mut self, delivery_time: DeliveryTime) -> Self {
        self.delivery_time = delivery_time;
        self
    }

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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DegradationKind {
    /// The adapter can preserve the logical feature through a portable
    /// representation rather than a native platform primitive.
    Emulated,
    ConvertedToText,
    Flattened,
    Dropped,
    OptionRemoved,
    MessageSplit,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeliveryDegradation {
    pub path: String,
    pub feature: String,
    pub kind: DegradationKind,
    pub detail: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeliveryPlan {
    pub messages: Vec<Message>,
    pub degradations: Vec<DeliveryDegradation>,
}

impl DeliveryPlan {
    #[must_use]
    pub fn degraded(&self) -> bool {
        !self.degradations.is_empty()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeliveryReport {
    pub messages: Vec<MessageRef>,
    pub degradations: Vec<DeliveryDegradation>,
    /// Per-physical-message outcomes in delivery-plan order. This remains
    /// populated on a partial-delivery error so callers can retry or
    /// compensate without duplicating already successful messages.
    #[serde(default)]
    pub items: Vec<DeliveryItemResult>,
}

impl DeliveryReport {
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
    pub index: usize,
    pub messages: Vec<MessageRef>,
    pub error: Option<String>,
}

impl DeliveryItemResult {
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.error.is_none()
    }
}

/// Error returned after a logical delivery has already produced a structured
/// per-item report. The report may contain successful physical messages.
#[derive(Clone, Debug, PartialEq)]
pub struct PartialDeliveryError {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveryPlanningError {
    pub path: String,
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

fn adapt_segment(
    segment: &MessageSegment,
    index: usize,
    capabilities: &BotCapabilities,
    policy: FallbackPolicy,
    output: &mut Vec<MessageSegment>,
    degradations: &mut Vec<DeliveryDegradation>,
) -> Result<(), DeliveryPlanningError> {
    let limit_violation = segment_limit_violation(segment, capabilities);
    let support = if limit_violation.is_some() {
        SupportLevel::Unsupported
    } else {
        segment_support(segment, capabilities)
    };
    let path = format!("segments[{index}]");
    let feature = limit_violation.unwrap_or_else(|| segment.kind_name().to_owned());
    match support {
        SupportLevel::Native => {
            output.push(segment.clone());
            return Ok(());
        }
        SupportLevel::Emulated => {
            output.push(segment.clone());
            degradations.push(DeliveryDegradation {
                path,
                feature,
                kind: DegradationKind::Emulated,
                detail: "feature will use the adapter's portable representation".to_owned(),
            });
            return Ok(());
        }
        SupportLevel::Unsupported => {}
    }

    let fallback = segment.fallback_text();
    match policy {
        FallbackPolicy::Strict => Err(DeliveryPlanningError { path, feature }),
        FallbackPolicy::DropUnsupported => {
            degradations.push(DeliveryDegradation {
                path,
                feature,
                kind: DegradationKind::Dropped,
                detail: "unsupported segment was removed".to_owned(),
            });
            Ok(())
        }
        FallbackPolicy::ToText | FallbackPolicy::Flatten | FallbackPolicy::Auto => {
            if let Some(text) = fallback.filter(|value| !value.is_empty()) {
                output.push(MessageSegment::text(text));
                degradations.push(DeliveryDegradation {
                    path,
                    feature,
                    kind: if matches!(policy, FallbackPolicy::Flatten) {
                        DegradationKind::Flattened
                    } else {
                        DegradationKind::ConvertedToText
                    },
                    detail: "unsupported segment was represented as text".to_owned(),
                });
                Ok(())
            } else if matches!(policy, FallbackPolicy::Auto) {
                Err(DeliveryPlanningError { path, feature })
            } else {
                degradations.push(DeliveryDegradation {
                    path,
                    feature,
                    kind: DegradationKind::Dropped,
                    detail: "unsupported segment had no portable fallback".to_owned(),
                });
                Ok(())
            }
        }
    }
}

fn option_is_supported(
    support: SupportLevel,
    path: &str,
    feature: &str,
    degradations: &mut Vec<DeliveryDegradation>,
) -> bool {
    match support {
        SupportLevel::Native => true,
        SupportLevel::Emulated => {
            degradations.push(DeliveryDegradation {
                path: path.to_owned(),
                feature: feature.to_owned(),
                kind: DegradationKind::Emulated,
                detail: "delivery option will use the adapter's portable representation".to_owned(),
            });
            true
        }
        SupportLevel::Unsupported => false,
    }
}

fn adapt_options(
    message: &mut Message,
    capabilities: &BotCapabilities,
    policy: FallbackPolicy,
    degradations: &mut Vec<DeliveryDegradation>,
) -> Result<(), DeliveryPlanningError> {
    if let Some(components) = message.options.components.take() {
        let within_limit = capabilities
            .limits
            .max_components
            .is_none_or(|limit| component_count(&components) <= limit);
        let support = match &components {
            MessageComponents::InlineKeyboard(_) => capabilities.components.inline_keyboard,
            MessageComponents::ReplyKeyboard(_)
            | MessageComponents::RemoveReplyKeyboard { .. }
            | MessageComponents::ForceReply { .. } => capabilities.components.reply_keyboard,
            MessageComponents::PlatformNative(_) => capabilities.components.platform_native,
        };
        let supported = within_limit
            && option_is_supported(
                support,
                "options.components",
                "message components",
                degradations,
            );
        if supported {
            message.options.components = Some(components);
        } else {
            let fallback = render_components(&components);
            if matches!(policy, FallbackPolicy::Strict)
                || (matches!(policy, FallbackPolicy::Auto) && fallback.is_empty())
            {
                return Err(DeliveryPlanningError {
                    path: "options.components".to_owned(),
                    feature: "message components".to_owned(),
                });
            }
            if !fallback.is_empty()
                && matches!(
                    policy,
                    FallbackPolicy::ToText | FallbackPolicy::Flatten | FallbackPolicy::Auto
                )
            {
                message.push(format!("\n{fallback}"));
            }
            degradations.push(DeliveryDegradation {
                path: "options.components".to_owned(),
                feature: "message components".to_owned(),
                kind: if fallback.is_empty() {
                    DegradationKind::Dropped
                } else {
                    DegradationKind::ConvertedToText
                },
                detail: "interactive components are unsupported by the target adapter".to_owned(),
            });
        }
    }

    if message.options.reply.is_some() {
        if !option_is_supported(
            capabilities.delivery.replies,
            "options.reply",
            "reply",
            degradations,
        ) {
            remove_option_or_error(policy, "options.reply", "reply", true, degradations)?;
            message.options.reply = None;
        } else if let Some(reply) = message.options.reply.as_mut() {
            if reply.quote.is_some()
                && !option_is_supported(
                    capabilities.delivery.quoted_replies,
                    "options.reply.quote",
                    "quoted reply",
                    degradations,
                )
            {
                remove_option_or_error(
                    policy,
                    "options.reply.quote",
                    "quoted reply",
                    false,
                    degradations,
                )?;
                reply.quote = None;
                reply.quote_position = None;
            }
            if reply.platform_data.is_some()
                && !option_is_supported(
                    capabilities.content.platform_native,
                    "options.reply.platform_data",
                    "platform-native reply data",
                    degradations,
                )
            {
                remove_option_or_error(
                    policy,
                    "options.reply.platform_data",
                    "platform-native reply data",
                    true,
                    degradations,
                )?;
                reply.platform_data = None;
            }
        }
    }

    match message.options.visibility.clone() {
        MessageVisibility::Public => {}
        MessageVisibility::Ephemeral => {
            if !option_is_supported(
                capabilities.delivery.ephemeral,
                "options.visibility",
                "ephemeral visibility",
                degradations,
            ) {
                remove_option_or_error(
                    policy,
                    "options.visibility",
                    "ephemeral visibility",
                    true,
                    degradations,
                )?;
                message.options.visibility = MessageVisibility::Public;
            }
        }
        MessageVisibility::PrivateTo(_) => {
            if !option_is_supported(
                capabilities.delivery.private_to_users,
                "options.visibility",
                "private message visibility",
                degradations,
            ) {
                remove_option_or_error(
                    policy,
                    "options.visibility",
                    "private message visibility",
                    true,
                    degradations,
                )?;
                message.options.visibility = MessageVisibility::Public;
            }
        }
    }

    match message.options.delivery_time.clone() {
        DeliveryTime::Immediate => {}
        DeliveryTime::Scheduled(_) => {
            if !option_is_supported(
                capabilities.delivery.scheduling,
                "options.delivery_time",
                "scheduled delivery",
                degradations,
            ) {
                remove_option_or_error(
                    policy,
                    "options.delivery_time",
                    "scheduled delivery",
                    true,
                    degradations,
                )?;
                message.options.delivery_time = DeliveryTime::Immediate;
            }
        }
        DeliveryTime::Draft => {
            if !option_is_supported(
                capabilities.delivery.drafts,
                "options.delivery_time",
                "draft delivery",
                degradations,
            ) {
                remove_option_or_error(
                    policy,
                    "options.delivery_time",
                    "draft delivery",
                    true,
                    degradations,
                )?;
                message.options.delivery_time = DeliveryTime::Immediate;
            }
        }
    }

    let optional_features = [
        (
            message.options.notification == NotificationPolicy::Silent,
            capabilities.delivery.silent,
            "silent notification",
            false,
        ),
        (
            message.options.notification == NotificationPolicy::Force,
            capabilities.delivery.forced_notification,
            "forced notification",
            false,
        ),
        (
            message.options.protect_content,
            capabilities.delivery.protected_content,
            "protected content",
            true,
        ),
        (
            message.options.link_preview.is_some(),
            capabilities.delivery.link_preview_control,
            "link preview control",
            false,
        ),
        (
            message.options.mentions.is_some(),
            capabilities.delivery.mention_control,
            "mention policy",
            false,
        ),
        (
            message.options.idempotency_key.is_some(),
            capabilities.delivery.idempotency_keys,
            "idempotency key",
            false,
        ),
        (
            message.options.client_message_id.is_some(),
            capabilities.delivery.client_message_ids,
            "client message id",
            false,
        ),
        (
            !message.options.metadata.is_empty(),
            capabilities.delivery.metadata,
            "message metadata",
            false,
        ),
        (
            message.options.platform_data.is_some(),
            capabilities.content.platform_native,
            "platform-native message options",
            true,
        ),
    ];
    for (present, support, feature, semantic_loss) in optional_features {
        if present && !option_is_supported(support, "options", feature, degradations) {
            remove_option_or_error(policy, "options", feature, semantic_loss, degradations)?;
        }
    }
    if (!capabilities.delivery.silent.is_supported()
        && message.options.notification == NotificationPolicy::Silent)
        || (!capabilities.delivery.forced_notification.is_supported()
            && message.options.notification == NotificationPolicy::Force)
    {
        message.options.notification = NotificationPolicy::Default;
    }
    if !capabilities.delivery.protected_content.is_supported() {
        message.options.protect_content = false;
    }
    if !capabilities.delivery.link_preview_control.is_supported() {
        message.options.link_preview = None;
    }
    if !capabilities.delivery.mention_control.is_supported() {
        message.options.mentions = None;
    }
    if !capabilities.delivery.idempotency_keys.is_supported() {
        message.options.idempotency_key = None;
    }
    if !capabilities.delivery.client_message_ids.is_supported() {
        message.options.client_message_id = None;
    }
    if !capabilities.delivery.metadata.is_supported() {
        message.options.metadata.clear();
    }
    if !capabilities.content.platform_native.is_supported() {
        message.options.platform_data = None;
    }
    Ok(())
}

fn remove_option_or_error(
    policy: FallbackPolicy,
    path: &str,
    feature: &str,
    semantic_loss: bool,
    degradations: &mut Vec<DeliveryDegradation>,
) -> Result<(), DeliveryPlanningError> {
    if matches!(policy, FallbackPolicy::Strict)
        || (semantic_loss && matches!(policy, FallbackPolicy::Auto))
    {
        return Err(DeliveryPlanningError {
            path: path.to_owned(),
            feature: feature.to_owned(),
        });
    }
    degradations.push(DeliveryDegradation {
        path: path.to_owned(),
        feature: feature.to_owned(),
        kind: DegradationKind::OptionRemoved,
        detail: "unsupported delivery option was reset to the platform default".to_owned(),
    });
    Ok(())
}

fn segment_limit_violation(
    segment: &MessageSegment,
    capabilities: &BotCapabilities,
) -> Option<String> {
    let limits = &capabilities.limits;
    if let Some(max) = limits.max_rich_text_length {
        if matches!(segment, MessageSegment::RichText(text) if text.text.chars().count() > max) {
            return Some(format!(
                "rich text exceeds the platform limit of {max} characters"
            ));
        }
    }
    if let Some(max) = limits.max_caption_length {
        if matches!(segment, MessageSegment::Media { media, .. } if media.caption.as_ref().is_some_and(|caption| caption.text.chars().count() > max))
        {
            return Some(format!(
                "media caption exceeds the platform limit of {max} characters"
            ));
        }
    }
    if let Some(max) = limits.max_poll_options {
        if matches!(segment, MessageSegment::Poll(poll) if poll.options.len() > max) {
            return Some(format!("poll exceeds the platform limit of {max} options"));
        }
    }
    for file in segment_files(segment) {
        if let Some(max) = limits.max_file_bytes {
            if file.size.is_some_and(|size| size > max) {
                return Some(format!(
                    "file `{}` exceeds the platform limit of {max} bytes",
                    file.name
                ));
            }
        }
        if !limits.supported_mime_types.is_empty() {
            if let Some(mime) = file.mime.as_deref() {
                if !limits
                    .supported_mime_types
                    .iter()
                    .any(|allowed| mime_matches(allowed, mime))
                {
                    return Some(format!(
                        "MIME type `{mime}` is not accepted by the platform"
                    ));
                }
            }
        }
    }
    None
}

fn segment_files(segment: &MessageSegment) -> Vec<&File> {
    match segment {
        MessageSegment::Share { image, .. } => image.iter().collect(),
        MessageSegment::Media { media, .. } => vec![&media.file],
        MessageSegment::MediaGallery(items) => items.iter().map(|item| &item.media.file).collect(),
        MessageSegment::Emoji(emoji) => emoji.file.iter().collect(),
        MessageSegment::Sticker(sticker) => sticker.file.iter().collect(),
        _ => Vec::new(),
    }
}

fn mime_matches(allowed: &str, actual: &str) -> bool {
    allowed == actual
        || allowed == "*/*"
        || allowed.strip_suffix("/*").is_some_and(|prefix| {
            actual.starts_with(prefix) && actual.as_bytes().get(prefix.len()) == Some(&b'/')
        })
}

fn segment_support(segment: &MessageSegment, capabilities: &BotCapabilities) -> SupportLevel {
    let content = &capabilities.content;
    match segment {
        MessageSegment::Text { .. } => content.plain_text,
        MessageSegment::RichText(_) => content.rich_text,
        MessageSegment::Media { kind, media } => {
            let support = match kind {
                MediaType::Image => content.images,
                MediaType::Video => content.video,
                MediaType::Audio => content.audio,
                MediaType::Document => content.files,
                MediaType::Animation => content.animation,
                MediaType::VoiceNote => content.voice_notes,
                MediaType::VideoNote => content.video_notes,
                MediaType::PlatformNative(_) => content.platform_native,
            };
            if support == SupportLevel::Native && media_requires_emulation(media) {
                SupportLevel::Emulated
            } else {
                support
            }
        }
        MessageSegment::MediaGallery(_) => content.media_galleries,
        MessageSegment::Location(location) => {
            if content.location == SupportLevel::Native
                && (location.horizontal_accuracy.is_some()
                    || location.live_period.is_some()
                    || location.heading.is_some()
                    || location.proximity_alert_radius.is_some()
                    || location.platform_data.is_some())
            {
                SupportLevel::Emulated
            } else {
                content.location
            }
        }
        MessageSegment::Contact(_) => content.contacts,
        MessageSegment::Emoji(_) => content.custom_emoji,
        MessageSegment::Sticker(_) => content.stickers,
        MessageSegment::Poll(poll) => {
            if matches!(poll.kind, crate::content::PollType::Quiz) {
                content.quizzes
            } else {
                content.polls
            }
        }
        MessageSegment::Checklist(_) => content.checklists,
        MessageSegment::Layout(_) => content.rich_layout,
        MessageSegment::PlatformNative(_) => content.platform_native,
        MessageSegment::Reference { .. }
        | MessageSegment::ForwardNode { .. }
        | MessageSegment::ForwardCustomNode { .. } => capabilities.collaboration.forwarding,
        MessageSegment::At { .. } => content.user_mentions,
        MessageSegment::AtRole { .. } => content.role_mentions,
        MessageSegment::AtChannel { .. } => content.channel_mentions,
        MessageSegment::AtAll => content.everyone_mentions,
        MessageSegment::Share { .. } => content.shares,
    }
}

fn media_requires_emulation(media: &Media) -> bool {
    media.caption.is_some()
        || media.thumbnail.is_some()
        || media.width.is_some()
        || media.height.is_some()
        || media.spoiler
        || media.alt_text.is_some()
        || media.waveform.is_some()
        || media.platform_data.is_some()
}

fn component_count(components: &MessageComponents) -> usize {
    match components {
        MessageComponents::InlineKeyboard(keyboard) => {
            keyboard.rows.iter().map(|row| row.components.len()).sum()
        }
        MessageComponents::ReplyKeyboard(keyboard) => {
            keyboard.rows.iter().map(|row| row.buttons.len()).sum()
        }
        MessageComponents::RemoveReplyKeyboard { .. }
        | MessageComponents::ForceReply { .. }
        | MessageComponents::PlatformNative(_) => 1,
    }
}

fn split_message_options(options: &MessageOptions, first: bool) -> MessageOptions {
    if first {
        return options.clone();
    }
    let mut continuation = options.clone();
    continuation.components = None;
    continuation.reply = None;
    continuation.idempotency_key = None;
    continuation.client_message_id = None;
    continuation
}

fn split_for_media_limit(message: Message, limit: Option<usize>) -> Vec<Message> {
    let Some(limit) = limit.filter(|limit| *limit > 0) else {
        return vec![message];
    };
    if message.segments.iter().map(media_units).sum::<usize>() <= limit {
        return vec![message];
    }

    let mut output = Vec::new();
    let mut current = Message::default();
    let mut units = 0usize;
    let mut first = true;
    let flush = |output: &mut Vec<Message>, current: &mut Message, first: &mut bool| {
        if current.segments.is_empty() {
            return;
        }
        current.options = split_message_options(&message.options, *first);
        if *first {
            current.id = message.id.clone();
            *first = false;
        }
        output.push(std::mem::take(current));
    };

    for segment in &message.segments {
        if let MessageSegment::MediaGallery(items) = segment {
            for chunk in items.chunks(limit) {
                if units > 0 && units.saturating_add(chunk.len()) > limit {
                    flush(&mut output, &mut current, &mut first);
                    units = 0;
                }
                current
                    .segments
                    .push(MessageSegment::MediaGallery(chunk.to_vec()));
                units = units.saturating_add(chunk.len());
                if units == limit {
                    flush(&mut output, &mut current, &mut first);
                    units = 0;
                }
            }
            continue;
        }
        let segment_units = media_units(segment);
        if segment_units > 0 && units > 0 && units.saturating_add(segment_units) > limit {
            flush(&mut output, &mut current, &mut first);
            units = 0;
        }
        current.segments.push(segment.clone());
        units = units.saturating_add(segment_units);
        if units == limit {
            flush(&mut output, &mut current, &mut first);
            units = 0;
        }
    }
    flush(&mut output, &mut current, &mut first);
    output
}

fn media_units(segment: &MessageSegment) -> usize {
    match segment {
        MessageSegment::Media { .. } | MessageSegment::Sticker(_) => 1,
        MessageSegment::Share { image, .. } => {
            if image.is_some() {
                1
            } else {
                0
            }
        }
        MessageSegment::MediaGallery(items) => items.len(),
        _ => 0,
    }
}

fn split_for_text_limit(message: Message, limit: Option<usize>) -> Vec<Message> {
    let Some(limit) = limit.filter(|limit| *limit > 0) else {
        return vec![message];
    };
    let total_text = message
        .segments
        .iter()
        .map(text_character_len)
        .sum::<usize>();
    if total_text <= limit {
        return vec![message];
    }

    let mut messages = Vec::new();
    let mut current = Message::default();
    let mut current_text = 0usize;
    let mut first = true;

    let flush = |messages: &mut Vec<Message>, current: &mut Message, first: &mut bool| {
        if current.segments.is_empty() {
            return;
        }
        current.options = split_message_options(&message.options, *first);
        if *first {
            current.id = message.id.clone();
            *first = false;
        }
        messages.push(std::mem::take(current));
    };

    for segment in &message.segments {
        let text_len = text_character_len(segment);
        if text_len == 0 {
            current.segments.push(segment.clone());
            continue;
        }

        let mut offset = 0usize;
        while offset < text_len {
            if current_text == limit {
                flush(&mut messages, &mut current, &mut first);
                current_text = 0;
            }
            let available = limit - current_text;
            let take = available.min(text_len - offset);
            current
                .segments
                .push(slice_text_segment(segment, offset, offset + take));
            current_text += take;
            offset += take;
            if current_text == limit {
                flush(&mut messages, &mut current, &mut first);
                current_text = 0;
            }
        }
    }
    flush(&mut messages, &mut current, &mut first);

    messages
}

fn text_character_len(segment: &MessageSegment) -> usize {
    match segment {
        MessageSegment::Text { content } => content.chars().count(),
        MessageSegment::RichText(content) => content.text.chars().count(),
        _ => 0,
    }
}

fn slice_text_segment(segment: &MessageSegment, start: usize, end: usize) -> MessageSegment {
    match segment {
        MessageSegment::Text { content } => MessageSegment::Text {
            content: slice_chars(content, start, end).to_owned(),
        },
        MessageSegment::RichText(content) => {
            let (byte_start, byte_end) = char_range_to_bytes(&content.text, start, end);
            let spans = content
                .spans
                .iter()
                .filter_map(|span| {
                    let intersection_start = span.range.start.max(byte_start);
                    let intersection_end = span.range.end.min(byte_end);
                    (intersection_start < intersection_end).then(|| crate::content::TextSpan {
                        range: (intersection_start - byte_start)..(intersection_end - byte_start),
                        styles: span.styles.clone(),
                    })
                })
                .collect();
            MessageSegment::RichText(RichText {
                text: content.text[byte_start..byte_end].to_owned(),
                spans,
            })
        }
        _ => segment.clone(),
    }
}

fn slice_chars(text: &str, start: usize, end: usize) -> &str {
    let (byte_start, byte_end) = char_range_to_bytes(text, start, end);
    &text[byte_start..byte_end]
}

fn char_range_to_bytes(text: &str, start: usize, end: usize) -> (usize, usize) {
    let byte_start = if start == 0 {
        0
    } else {
        text.char_indices()
            .nth(start)
            .map_or(text.len(), |(offset, _)| offset)
    };
    let byte_end = if end == 0 {
        0
    } else {
        text.char_indices()
            .nth(end)
            .map_or(text.len(), |(offset, _)| offset)
    };
    (byte_start, byte_end)
}

fn render_poll(poll: &Poll) -> String {
    let mut output = poll.question.text.clone();
    for (index, option) in poll.options.iter().enumerate() {
        output.push_str(&format!("\n{}. {}", index + 1, option.text.text));
    }
    output
}

fn render_checklist(checklist: &Checklist) -> String {
    let mut output = checklist.title.text.clone();
    for task in &checklist.tasks {
        output.push_str(&format!(
            "\n{} {}",
            if task.completed { "[x]" } else { "[ ]" },
            task.text.text
        ));
    }
    output
}

fn render_components(components: &MessageComponents) -> String {
    use crate::interaction::{InteractionComponent, MessageComponents};
    match components {
        MessageComponents::InlineKeyboard(keyboard) => keyboard
            .rows
            .iter()
            .flat_map(|row| row.components.iter())
            .filter_map(|component| match component {
                InteractionComponent::Button(button) => Some(format!("[{}]", button.label)),
                InteractionComponent::Select(select) => select
                    .placeholder
                    .as_ref()
                    .map(|placeholder| format!("[{placeholder}]")),
                InteractionComponent::Input(_) | InteractionComponent::PlatformNative(_) => None,
            })
            .collect::<Vec<_>>()
            .join(" "),
        MessageComponents::ReplyKeyboard(keyboard) => keyboard
            .rows
            .iter()
            .flat_map(|row| row.buttons.iter())
            .map(|button| format!("[{}]", button.label))
            .collect::<Vec<_>>()
            .join(" "),
        MessageComponents::ForceReply { .. } => "[reply requested]".to_owned(),
        MessageComponents::RemoveReplyKeyboard { .. } | MessageComponents::PlatformNative(_) => {
            String::new()
        }
    }
}

/// Pure attachment descriptor. Construction never performs I/O; adapters own
/// path inspection, URL probing, streaming, and upload policy.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct File {
    pub id: Option<String>,
    pub name: String,
    pub uri: Option<String>,
    pub path: Option<PathBuf>,
    pub base64: Option<String>,
    pub mime: Option<String>,
    pub size: Option<u64>,
}

impl File {
    pub async fn try_from_path<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let metadata = tokio::fs::metadata(path).await?;
        anyhow::ensure!(metadata.is_file(), "path does not point to a file");

        let mut file = Self::from_path(path).size(metadata.len());
        file.mime = mime_guess::from_path(path)
            .first()
            .map(|mime| mime.to_string());
        Ok(file)
    }

    pub fn try_from_url(url: &str) -> anyhow::Result<Self> {
        anyhow::ensure!(url.contains("://"), "invalid URL");
        Ok(Self::from_url(url))
    }

    #[must_use]
    pub fn platform(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: Some(id.into()),
            name: name.into(),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn from_path(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        Self {
            name: path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_owned(),
            path: Some(path.to_owned()),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn from_url(url: impl Into<String>) -> Self {
        let url = url.into();
        let name = url
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .split('?')
            .next()
            .unwrap_or_default()
            .to_owned();
        Self {
            name,
            uri: Some(url),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn mime(mut self, mime: impl Into<String>) -> Self {
        self.mime = Some(mime.into());
        self
    }

    #[must_use]
    pub fn size(mut self, size: u64) -> Self {
        self.size = Some(size);
        self
    }

    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        self.id.as_ref().map_or(0, String::len)
            + self.name.len()
            + self.uri.as_ref().map_or(0, String::len)
            + self
                .path
                .as_ref()
                .map_or(0, |path| path.as_os_str().as_encoded_bytes().len())
            + self.base64.as_ref().map_or(0, String::len)
            + self.mime.as_ref().map_or(0, String::len)
            + 64
    }
}
