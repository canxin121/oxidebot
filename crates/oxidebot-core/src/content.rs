//! Portable message content, rich text, rich layout, media, poll, checklist,
//! contact, and delivery models.

#[path = "content_history.rs"]
mod history;

pub use history::{
    BatchItemResult, BatchMessage, BatchSendResult, ForwardContext, ForwardOptions,
    MessageEnvelope, MessageOrigin, MessageQuery, ReplyContext,
};

#[path = "content_layout.rs"]
mod layout;

pub use layout::{LayoutColumn, LayoutNode, LayoutStyle, RichLayout, TableCell};

#[path = "content_media.rs"]
mod media;

pub use media::{
    ContactCard, CustomEmoji, LocationContent, Media, MediaGalleryItem, MediaType, PhoneNumber,
    Sticker,
};

#[path = "content_poll.rs"]
mod poll;

pub use poll::{Checklist, ChecklistChange, ChecklistTask, Poll, PollOption, PollType};

#[path = "content_text.rs"]
mod text;

pub use text::{RichText, TextSpan, TextStyle};

#[path = "content_delivery.rs"]
mod delivery;

pub use delivery::{
    DeliveryTime, LinkPreviewOptions, MentionAllowance, MentionPolicy, MessageVisibility,
    NotificationPolicy, ReplyOptions,
};

#[path = "content_form.rs"]
mod form;

pub use form::FormValue;
