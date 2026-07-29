//! Runtime identity wrappers around the public message IR.
//!
//! Message content is defined only once in `source::message`. This module keeps
//! the dense runtime address types used by bounded bot command queues.

use crate::{BotSlot, CompactId, ConversationKey, InvalidId, PlatformId, RetainedSize, UserKey};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::sync::Arc;
use thiserror::Error;

pub const MAX_METADATA_ENTRIES: usize = 64;
pub const MAX_METADATA_KEY_BYTES: usize = 1_024;
pub const MAX_METADATA_BYTES: usize = 256 * 1024;
pub const MAX_IDEMPOTENCY_KEY_BYTES: usize = 256;
pub const MAX_MESSAGE_CONTENT_ITEMS: usize = 4_096;
pub const MAX_MESSAGE_RECIPIENTS: usize = 4_096;
pub const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_DESCRIPTOR_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ModelError {
    #[error(transparent)]
    InvalidId(#[from] InvalidId),
    #[error("value references a different bot slot")]
    WrongBot,
    #[error("native data belongs to a different platform")]
    WrongPlatform,
    #[error("message metadata exceeds its structural limit")]
    MetadataTooLarge,
    #[error("message metadata keys must contain 1..={MAX_METADATA_KEY_BYTES} bytes")]
    InvalidMetadataKey,
    #[error("message idempotency key must contain 1..={MAX_IDEMPOTENCY_KEY_BYTES} bytes")]
    InvalidIdempotencyKey,
    #[error("{0} exceeds its structural item limit")]
    CollectionTooLarge(&'static str),
    #[error("{0} exceeds its structural byte limit")]
    ValueTooLarge(&'static str),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct MessageRef {
    pub conversation: ConversationKey,
    pub id: CompactId,
}

impl MessageRef {
    #[must_use]
    pub fn new(conversation: ConversationKey, id: impl Into<CompactId>) -> Self {
        Self {
            conversation,
            id: id.into(),
        }
    }

    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        self.conversation
            .estimated_bytes()
            .saturating_add(self.id.estimated_bytes())
            .saturating_add(32)
    }

    pub fn validate_for(&self, bot: BotSlot) -> Result<(), ModelError> {
        if self.conversation.bot != bot {
            return Err(ModelError::WrongBot);
        }
        self.conversation.validate()?;
        self.id.validate()?;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct MessageReceipt {
    pub reference: MessageRef,
}

impl MessageReceipt {
    #[must_use]
    pub fn new(reference: MessageRef) -> Self {
        Self { reference }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct MessageTarget {
    pub conversation: ConversationKey,
    pub recipients: Vec<UserKey>,
}

impl MessageTarget {
    #[must_use]
    pub fn new(conversation: ConversationKey) -> Self {
        Self {
            conversation,
            recipients: Vec::new(),
        }
    }

    #[must_use]
    pub fn recipients(mut self, recipients: impl IntoIterator<Item = UserKey>) -> Self {
        self.recipients.extend(recipients);
        self
    }

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

#[derive(Clone, Debug)]
pub struct NativeData {
    pub platform: PlatformId,
    pub data: Arc<RawValue>,
}

impl PartialEq for NativeData {
    fn eq(&self, other: &Self) -> bool {
        self.platform == other.platform && self.data.get() == other.data.get()
    }
}

impl NativeData {
    #[must_use]
    pub fn new(platform: PlatformId, data: Arc<RawValue>) -> Self {
        Self { platform, data }
    }

    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        self.data
            .get()
            .len()
            .saturating_add(self.platform.as_str().len())
    }

    pub fn validate_for(&self, platform: &PlatformId) -> Result<(), ModelError> {
        self.platform.validate()?;
        if &self.platform == platform {
            Ok(())
        } else {
            Err(ModelError::WrongPlatform)
        }
    }
}

impl crate::source::message::Message {
    pub fn validate_for(&self, _bot: BotSlot, _platform: &PlatformId) -> Result<(), ModelError> {
        if self.segments.len() > MAX_MESSAGE_CONTENT_ITEMS {
            return Err(ModelError::CollectionTooLarge("message segments"));
        }
        if self
            .options
            .idempotency_key
            .as_ref()
            .is_some_and(|key| key.is_empty() || key.len() > MAX_IDEMPOTENCY_KEY_BYTES)
        {
            return Err(ModelError::InvalidIdempotencyKey);
        }
        if self.options.metadata.len() > MAX_METADATA_ENTRIES {
            return Err(ModelError::MetadataTooLarge);
        }
        if self
            .options
            .metadata
            .keys()
            .any(|key| key.is_empty() || key.len() > MAX_METADATA_KEY_BYTES)
        {
            return Err(ModelError::InvalidMetadataKey);
        }
        let metadata_bytes = self
            .options
            .metadata
            .iter()
            .map(|(key, value)| key.len().saturating_add(value.to_string().len()))
            .sum::<usize>();
        if metadata_bytes > MAX_METADATA_BYTES {
            return Err(ModelError::MetadataTooLarge);
        }
        if self.extract_plain_text().len() > MAX_TEXT_BYTES {
            return Err(ModelError::ValueTooLarge("message text"));
        }
        Ok(())
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

impl RetainedSize for crate::source::message::Message {
    fn retained_bytes(&self) -> usize {
        self.estimated_bytes()
    }
}
