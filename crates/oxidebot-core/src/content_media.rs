//! Portable file-backed media, location, contact, emoji, and sticker models.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::RichText;
use crate::{interaction::PlatformNativeData, source::message::File};

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
