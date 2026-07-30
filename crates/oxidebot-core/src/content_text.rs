//! Rich-text content and semantic style annotations.

use std::ops::Range;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{conversation::MessageRef, interaction::PlatformNativeData};

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
