use serde::{Deserialize, Serialize};
use std::{fmt, sync::Arc};
use thiserror::Error;

/// Error returned when a semantic identifier is empty.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("identifier must not be empty")]
pub struct InvalidId;

macro_rules! string_id {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Arc<str>);

        impl $name {
            /// Creates a validated identifier.
            pub fn new(value: impl Into<Arc<str>>) -> Result<Self, InvalidId> {
                let value = value.into();
                if value.is_empty() {
                    Err(InvalidId)
                } else {
                    Ok(Self(value))
                }
            }

            /// Returns the identifier as a string slice.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Approximate retained bytes.
            #[must_use]
            pub fn estimated_bytes(&self) -> usize {
                self.0.len()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

string_id!(PlatformId, "Stable platform identifier.");
string_id!(BotId, "Stable bot identifier inside a platform.");
string_id!(EventId, "Stable event identifier supplied by a platform.");
string_id!(
    SessionNamespace,
    "Logical namespace for a dialogue session."
);

/// Compact platform-owned identifier.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CompactId {
    /// Numeric identifier without an allocation.
    Number(u64),
    /// Opaque string identifier.
    Text(Arc<str>),
}

impl CompactId {
    /// Approximate retained bytes.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        match self {
            Self::Number(_) => 0,
            Self::Text(value) => value.len(),
        }
    }
}

impl From<u64> for CompactId {
    fn from(value: u64) -> Self {
        Self::Number(value)
    }
}

impl From<String> for CompactId {
    fn from(value: String) -> Self {
        Self::Text(value.into())
    }
}

impl From<&str> for CompactId {
    fn from(value: &str) -> Self {
        Self::Text(Arc::from(value))
    }
}

impl fmt::Display for CompactId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(value) => value.fmt(formatter),
            Self::Text(value) => formatter.write_str(value),
        }
    }
}

/// Dense process-local index assigned to a registered bot.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct BotSlot(pub u32);

/// Conversation identifier scoped to one registered bot.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ConversationKey {
    /// Owning bot.
    pub bot: BotSlot,
    /// Platform conversation identifier.
    pub id: CompactId,
}

impl ConversationKey {
    /// Creates a conversation key.
    #[must_use]
    pub fn new(bot: BotSlot, id: impl Into<CompactId>) -> Self {
        Self { bot, id: id.into() }
    }

    /// Approximate retained bytes.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        self.id.estimated_bytes().saturating_add(8)
    }
}

/// User identifier scoped to one registered bot.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct UserKey {
    /// Owning bot.
    pub bot: BotSlot,
    /// Platform user identifier.
    pub id: CompactId,
}

impl UserKey {
    /// Creates a user key.
    #[must_use]
    pub fn new(bot: BotSlot, id: impl Into<CompactId>) -> Self {
        Self { bot, id: id.into() }
    }

    /// Approximate retained bytes.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        self.id.estimated_bytes().saturating_add(8)
    }
}

/// Default key used to preserve event order without serializing unrelated work.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum ExecutionKey {
    /// Order by conversation.
    Conversation(ConversationKey),
    /// Order by user when no conversation exists.
    User(UserKey),
    /// Order only duplicate deliveries of an otherwise unscoped event.
    Event(EventId),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_ids_reject_empty_values() {
        assert_eq!(PlatformId::new(""), Err(InvalidId));
        assert_eq!(BotId::new("bot").expect("valid id").as_str(), "bot");
    }

    #[test]
    fn compact_ids_preserve_numeric_and_text_identity() {
        assert_ne!(CompactId::from(42_u64), CompactId::from("42"));
        assert_eq!(CompactId::from(42_u64).to_string(), "42");
        assert_eq!(CompactId::from("room").estimated_bytes(), 4);
    }
}
