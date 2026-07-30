use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

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

#[path = "message_core.rs"]
mod message_core;

pub use message_core::Message;

#[path = "message_options.rs"]
mod message_options;

pub use message_options::MessageOptions;

#[path = "message_delivery.rs"]
mod message_delivery;

#[path = "message_delivery_report.rs"]
mod message_delivery_report;

use message_delivery::{
    adapt_options, adapt_segment, render_checklist, render_poll, split_for_media_limit,
    split_for_text_limit,
};

pub use message_delivery_report::{
    DegradationKind, DeliveryDegradation, DeliveryItemResult, DeliveryPlan, DeliveryPlanningError,
    DeliveryReport, FallbackPolicy, PartialDeliveryError,
};

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
