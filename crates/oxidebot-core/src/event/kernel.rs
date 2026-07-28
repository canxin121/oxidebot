//! Internal bounded-dispatch records for the runtime.
//!
//! The payload is always the public OxideBot [`Event`]. This module only
//! adds compact routing metadata, queue accounting, and receive-time metadata.

use super::{Event, EventTag, EventType};
use crate::{
    BotSlot, ConversationKey, EventId, ExecutionKey, PlatformId, RetainedBytes, RetainedSize,
    UserKey,
};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::{
    sync::Arc,
    time::{Instant, SystemTime},
};
use thiserror::Error;

/// Maximum bytes allowed in command, interaction, and native route keys.
pub const MAX_ROUTE_KEY_BYTES: usize = 512;

/// Internal dispatch category used only to select optimized runtime paths.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[repr(u8)]
pub enum DispatchKind {
    Message = 0,
    MessageUpdate = 1,
    MessageDelete = 2,
    Interaction = 3,
    Reaction = 4,
    Member = 5,
    Conversation = 6,
    File = 7,
    Payment = 8,
    Native = 63,
}

/// How message work is partitioned by the virtual-actor executor.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MessageExecutionPartition {
    /// Preserve strict ordering for an entire conversation or thread.
    #[default]
    Conversation,
    /// Allow different actors in one conversation to execute concurrently.
    ConversationActor,
}

/// Lightweight routing metadata extracted before full event decoding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispatchIndex {
    pub bot: BotSlot,
    pub platform: PlatformId,
    pub kind: DispatchKind,
    pub event_type: EventType,
    pub conversation: Option<ConversationKey>,
    pub actor: Option<UserKey>,
    pub command: Option<Arc<str>>,
    pub interaction: Option<Arc<str>>,
    pub native_type: Option<Arc<str>>,
}

impl DispatchIndex {
    /// Creates the pre-decode index for one public event type.
    #[must_use]
    pub fn new(bot: BotSlot, platform: PlatformId, event_type: EventType) -> Self {
        Self {
            bot,
            platform,
            kind: event_type.dispatch_kind(),
            event_type,
            conversation: None,
            actor: None,
            command: None,
            interaction: None,
            native_type: None,
        }
    }

    /// Alias emphasizing that the discriminator is the public event type.
    #[must_use]
    pub fn event(bot: BotSlot, platform: PlatformId, event_type: EventType) -> Self {
        Self::new(bot, platform, event_type)
    }

    /// Replaces the event type and synchronizes the optimized dispatch kind.
    #[must_use]
    pub fn with_event_type(mut self, event_type: EventType) -> Self {
        self.kind = event_type.dispatch_kind();
        self.event_type = event_type;
        self
    }

    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        self.platform
            .estimated_bytes()
            .saturating_add(
                self.conversation
                    .as_ref()
                    .map_or(0, ConversationKey::estimated_bytes),
            )
            .saturating_add(self.actor.as_ref().map_or(0, UserKey::estimated_bytes))
            .saturating_add(self.command.as_ref().map_or(0, |value| value.len()))
            .saturating_add(self.interaction.as_ref().map_or(0, |value| value.len()))
            .saturating_add(self.native_type.as_ref().map_or(0, |value| value.len()))
            .saturating_add(104)
    }

    #[must_use]
    pub fn execution_key(
        &self,
        event_id: &EventId,
        message_partition: MessageExecutionPartition,
    ) -> ExecutionKey {
        if self.kind == DispatchKind::Message
            && message_partition == MessageExecutionPartition::ConversationActor
        {
            if let (Some(conversation), Some(actor)) = (&self.conversation, &self.actor) {
                return ExecutionKey::ConversationActor {
                    conversation: conversation.clone(),
                    actor: actor.clone(),
                };
            }
        }
        if let Some(conversation) = &self.conversation {
            ExecutionKey::Conversation(conversation.clone())
        } else if let Some(actor) = &self.actor {
            ExecutionKey::User(actor.clone())
        } else {
            ExecutionKey::Event(event_id.clone())
        }
    }

    pub fn validate_for(
        &self,
        bot: BotSlot,
        platform: &PlatformId,
    ) -> Result<(), DispatchValidationError> {
        if self.bot != bot {
            return Err(DispatchValidationError::WrongBot);
        }
        self.platform.validate()?;
        if &self.platform != platform {
            return Err(DispatchValidationError::WrongPlatform);
        }
        if self.kind != self.event_type.dispatch_kind() {
            return Err(DispatchValidationError::KindMismatch);
        }
        if let Some(conversation) = &self.conversation {
            if conversation.bot != bot {
                return Err(DispatchValidationError::WrongBot);
            }
            conversation.validate()?;
        }
        if let Some(actor) = &self.actor {
            if actor.bot != bot {
                return Err(DispatchValidationError::WrongBot);
            }
            actor.validate()?;
        }
        for value in [
            self.command.as_deref(),
            self.interaction.as_deref(),
            self.native_type.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if value.is_empty() || value.len() > MAX_ROUTE_KEY_BYTES {
                return Err(DispatchValidationError::InvalidRouteKey);
            }
        }
        Ok(())
    }
}

/// Shared original platform payload.
#[derive(Clone, Debug)]
pub enum RawPayload {
    Bytes(RetainedBytes),
    Json(Arc<RawValue>),
}

impl RawPayload {
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        match self {
            Self::Bytes(bytes) => bytes.retained_bytes(),
            Self::Json(json) => json.get().len(),
        }
    }
}

/// One shared public event plus the adapter's conservative retained-size charge.
#[derive(Clone, Debug)]
pub struct EventPayload {
    event: Event,
    retained_bytes: usize,
}

impl EventPayload {
    #[must_use]
    pub fn new(event: Event, retained_bytes: usize) -> Self {
        Self {
            event,
            retained_bytes: retained_bytes.max(std::mem::size_of::<Event>()),
        }
    }

    #[must_use]
    pub fn event(&self) -> &Event {
        &self.event
    }

    #[must_use]
    pub fn event_type(&self) -> EventType {
        self.event.event_type()
    }

    #[must_use]
    pub const fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }
}

/// Adapter-produced event before runtime sequence and receive time are assigned.
#[derive(Debug)]
pub struct DispatchDraft {
    pub id: EventId,
    pub index: DispatchIndex,
    pub occurred_at: Option<SystemTime>,
    pub delivery_attempt: u16,
    event: Arc<EventPayload>,
}

impl DispatchDraft {
    /// Creates a draft from the single public event representation.
    #[must_use]
    pub fn new(id: EventId, mut index: DispatchIndex, event: Event, retained_bytes: usize) -> Self {
        let payload = Arc::new(EventPayload::new(event, retained_bytes));
        index = index.with_event_type(payload.event_type());
        Self {
            id,
            index,
            occurred_at: None,
            delivery_attempt: 0,
            event: payload,
        }
    }

    #[must_use]
    pub fn event(&self) -> &Event {
        self.event.event()
    }

    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        self.id
            .estimated_bytes()
            .saturating_add(self.index.estimated_bytes())
            .saturating_add(self.event.retained_bytes())
            .saturating_add(176)
    }

    pub fn validate_for(
        &self,
        bot: BotSlot,
        platform: &PlatformId,
    ) -> Result<(), DispatchValidationError> {
        self.id.validate()?;
        self.index.validate_for(bot, platform)?;
        if self.event.event_type() != self.index.event_type {
            return Err(DispatchValidationError::EventTypeMismatch);
        }
        Ok(())
    }

    #[must_use]
    pub fn finalize(
        self,
        sequence: u64,
        received_at: Instant,
        raw: Option<Arc<RawPayload>>,
    ) -> DispatchEnvelope {
        let incremental_bytes = self.estimated_bytes();
        DispatchEnvelope {
            id: self.id,
            sequence,
            index: self.index,
            occurred_at: self.occurred_at,
            received_at,
            delivery_attempt: self.delivery_attempt,
            event: self.event,
            raw,
            incremental_bytes,
        }
    }
}

/// Batch decoded once from one platform frame.
#[derive(Debug, Default)]
pub struct DispatchBatch {
    pub raw: Option<Arc<RawPayload>>,
    pub events: Vec<DispatchDraft>,
}

impl DispatchBatch {
    #[must_use]
    pub fn new(events: impl IntoIterator<Item = DispatchDraft>) -> Self {
        Self {
            raw: None,
            events: events.into_iter().collect(),
        }
    }

    #[must_use]
    pub fn with_raw(mut self, raw: Arc<RawPayload>) -> Self {
        self.raw = Some(raw);
        self
    }

    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        self.raw
            .as_ref()
            .map_or(0, |raw| raw.estimated_bytes())
            .saturating_add(
                self.events
                    .iter()
                    .map(DispatchDraft::estimated_bytes)
                    .sum::<usize>(),
            )
            .saturating_add(self.events.capacity() * std::mem::size_of::<DispatchDraft>())
    }
}

impl RetainedSize for DispatchBatch {
    fn retained_bytes(&self) -> usize {
        self.estimated_bytes()
    }
}

/// Runtime-owned immutable event.
#[derive(Debug)]
pub struct DispatchEnvelope {
    pub id: EventId,
    pub sequence: u64,
    pub index: DispatchIndex,
    pub occurred_at: Option<SystemTime>,
    pub received_at: Instant,
    pub delivery_attempt: u16,
    event: Arc<EventPayload>,
    pub raw: Option<Arc<RawPayload>>,
    incremental_bytes: usize,
}

impl DispatchEnvelope {
    #[must_use]
    pub fn event(&self) -> &Event {
        self.event.event()
    }

    #[must_use]
    pub fn event_as<T: EventTag>(&self) -> Option<&T::Event> {
        T::get(self.event())
    }

    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        self.raw
            .as_ref()
            .map_or(0, |raw| raw.estimated_bytes())
            .saturating_add(self.incremental_estimated_bytes())
    }

    #[must_use]
    pub const fn incremental_estimated_bytes(&self) -> usize {
        self.incremental_bytes
    }

    #[must_use]
    pub const fn incremental_retained_bytes(&self) -> usize {
        self.incremental_bytes
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum DispatchValidationError {
    #[error("event references a different bot")]
    WrongBot,
    #[error("event references a different platform")]
    WrongPlatform,
    #[error("event route key is empty or too large")]
    InvalidRouteKey,
    #[error("event dispatch kind differs from its public event type")]
    KindMismatch,
    #[error("decoded event type differs from its pre-decode index")]
    EventTypeMismatch,
    #[error(transparent)]
    InvalidId(#[from] crate::InvalidId),
}
