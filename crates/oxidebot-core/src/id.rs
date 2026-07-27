use crate::RetainedSize;
use serde::{Deserialize, Deserializer, Serialize};
use std::{fmt, sync::Arc};
use thiserror::Error;

/// Maximum byte length accepted for externally supplied semantic identifiers.
pub const MAX_SEMANTIC_ID_BYTES: usize = 1024;

/// Error returned when a semantic identifier is empty or unreasonably large.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("identifier must contain 1..={MAX_SEMANTIC_ID_BYTES} bytes")]
pub struct InvalidId;

macro_rules! string_id {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(Arc<str>);

        impl $name {
            /// Creates a validated identifier.
            pub fn new(value: impl Into<Arc<str>>) -> Result<Self, InvalidId> {
                let value = value.into();
                if value.is_empty() || value.len() > MAX_SEMANTIC_ID_BYTES {
                    Err(InvalidId)
                } else {
                    Ok(Self(value))
                }
            }

            /// Revalidates an identifier after crossing an untrusted boundary.
            pub fn validate(&self) -> Result<(), InvalidId> {
                if self.0.is_empty() || self.0.len() > MAX_SEMANTIC_ID_BYTES {
                    Err(InvalidId)
                } else {
                    Ok(())
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

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }

        impl TryFrom<Arc<str>> for $name {
            type Error = InvalidId;

            fn try_from(value: Arc<str>) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl From<$name> for Arc<str> {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl RetainedSize for $name {
            fn retained_bytes(&self) -> usize {
                self.estimated_bytes()
                    .saturating_add(std::mem::size_of::<Self>())
            }
        }
    };
}

string_id!(PlatformId, "Stable platform identifier.");
string_id!(BotId, "Stable bot identifier inside a platform.");
string_id!(EventId, "Stable event identifier supplied by a platform.");
string_id!(
    SessionNamespace,
    "Logical namespace used to identify and cancel a dialogue session."
);

/// Stable identity of one bot connection without lossy string concatenation.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct BotIdentity {
    pub platform: PlatformId,
    pub bot: BotId,
}

impl BotIdentity {
    #[must_use]
    pub const fn new(platform: PlatformId, bot: BotId) -> Self {
        Self { platform, bot }
    }
}

impl RetainedSize for BotIdentity {
    fn retained_bytes(&self) -> usize {
        self.platform
            .retained_bytes()
            .saturating_add(self.bot.retained_bytes())
            .saturating_add(std::mem::size_of::<Self>())
    }
}

/// Compact platform-owned identifier.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(untagged)]
pub enum CompactId {
    /// Unsigned numeric identifier without an allocation.
    Number(u64),
    /// Signed numeric identifier, required by platforms with negative chat IDs.
    Signed(i64),
    /// Opaque string identifier.
    Text(Arc<str>),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum CompactIdWire {
    Unsigned(u64),
    Signed(i64),
    Text(Arc<str>),
}

impl<'de> Deserialize<'de> for CompactId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = CompactIdWire::deserialize(deserializer)?;
        match wire {
            CompactIdWire::Unsigned(value) => Ok(Self::Number(value)),
            CompactIdWire::Signed(value) => Ok(Self::from(value)),
            CompactIdWire::Text(value) => Self::text(value).map_err(serde::de::Error::custom),
        }
    }
}

impl CompactId {
    /// Creates a validated opaque identifier.
    pub fn text(value: impl Into<Arc<str>>) -> Result<Self, InvalidId> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_SEMANTIC_ID_BYTES {
            Err(InvalidId)
        } else {
            Ok(Self::Text(value))
        }
    }

    /// Validates opaque identifiers created through infallible conversion.
    pub fn validate(&self) -> Result<(), InvalidId> {
        match self {
            Self::Text(value) if value.is_empty() || value.len() > MAX_SEMANTIC_ID_BYTES => {
                Err(InvalidId)
            }
            Self::Number(_) | Self::Signed(_) | Self::Text(_) => Ok(()),
        }
    }

    /// Approximate retained bytes.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        match self {
            Self::Number(_) | Self::Signed(_) => 0,
            Self::Text(value) => value.len(),
        }
    }
}

impl From<u64> for CompactId {
    fn from(value: u64) -> Self {
        Self::Number(value)
    }
}

impl From<u32> for CompactId {
    fn from(value: u32) -> Self {
        Self::Number(value.into())
    }
}

impl From<i64> for CompactId {
    fn from(value: i64) -> Self {
        if value >= 0 {
            Self::Number(value as u64)
        } else {
            Self::Signed(value)
        }
    }
}

impl From<i32> for CompactId {
    fn from(value: i32) -> Self {
        Self::from(i64::from(value))
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
            Self::Signed(value) => value.fmt(formatter),
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
    /// Optional thread or topic identifier within the conversation.
    pub thread: Option<CompactId>,
}

impl ConversationKey {
    /// Creates a root conversation key.
    #[must_use]
    pub fn new(bot: BotSlot, id: impl Into<CompactId>) -> Self {
        Self {
            bot,
            id: id.into(),
            thread: None,
        }
    }

    /// Addresses a thread or topic inside this conversation.
    #[must_use]
    pub fn thread(mut self, thread: impl Into<CompactId>) -> Self {
        self.thread = Some(thread.into());
        self
    }

    /// Alias for adapters that call a thread/topic a subspace.
    #[must_use]
    pub fn in_subspace(self, subspace: impl Into<CompactId>) -> Self {
        self.thread(subspace)
    }

    /// Validates platform-owned identifiers.
    pub fn validate(&self) -> Result<(), InvalidId> {
        self.id.validate()?;
        if let Some(thread) = &self.thread {
            thread.validate()?;
        }
        Ok(())
    }

    /// Approximate retained bytes.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        self.id
            .estimated_bytes()
            .saturating_add(self.thread.as_ref().map_or(0, CompactId::estimated_bytes))
            .saturating_add(16)
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

    /// Validates the platform-owned identifier.
    pub fn validate(&self) -> Result<(), InvalidId> {
        self.id.validate()
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
    /// Order by conversation or thread.
    Conversation(ConversationKey),
    /// Order independently by actor inside one conversation.
    ConversationActor {
        conversation: ConversationKey,
        actor: UserKey,
    },
    /// Order by user when no conversation exists.
    User(UserKey),
    /// Order only duplicate deliveries of an otherwise unscoped event.
    Event(EventId),
}

impl RetainedSize for CompactId {
    fn retained_bytes(&self) -> usize {
        self.estimated_bytes()
            .saturating_add(std::mem::size_of::<Self>())
    }
}

impl RetainedSize for ConversationKey {
    fn retained_bytes(&self) -> usize {
        self.estimated_bytes()
            .saturating_add(std::mem::size_of::<Self>())
    }
}

impl RetainedSize for UserKey {
    fn retained_bytes(&self) -> usize {
        self.estimated_bytes()
            .saturating_add(std::mem::size_of::<Self>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_ids_reject_empty_and_oversized_values() {
        assert_eq!(PlatformId::new(""), Err(InvalidId));
        assert_eq!(
            EventId::new("x".repeat(MAX_SEMANTIC_ID_BYTES + 1)),
            Err(InvalidId)
        );
        assert_eq!(BotId::new("bot").expect("valid id").as_str(), "bot");
    }

    #[test]
    fn compact_ids_preserve_numeric_and_text_identity() {
        assert_ne!(CompactId::from(42_u64), CompactId::from("42"));
        assert_eq!(CompactId::from(-42_i64).to_string(), "-42");
        assert_eq!(CompactId::from("room").estimated_bytes(), 4);
    }

    #[test]
    fn conversation_threads_are_distinct_execution_keys() {
        let root = ConversationKey::new(BotSlot(1), 7_u64);
        assert_ne!(root.clone(), root.thread(9_u64));
    }
}
