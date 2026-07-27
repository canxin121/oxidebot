use crate::{
    BotSlot, CompactId, ConversationKey, EventId, ExecutionKey, Media, MessageContent, MessageRef,
    NativeData, PlatformId, UserKey,
};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::{
    collections::HashSet,
    sync::Arc,
    time::{Instant, SystemTime},
};

/// Canonical event category. One semantic action has exactly one category.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[repr(u8)]
pub enum EventKind {
    /// A message was created.
    MessageCreated = 0,
    /// A message was edited.
    MessageUpdated = 1,
    /// One or more messages were deleted.
    MessagesDeleted = 2,
    /// A button, command, select, or form interaction.
    Interaction = 3,
    /// Reactions on a message changed.
    ReactionChanged = 4,
    /// Conversation membership changed.
    MemberChanged = 5,
    /// Conversation profile or state changed.
    ConversationChanged = 6,
    /// A file was shared, updated, or deleted.
    FileChanged = 7,
    /// A payment or subscription changed.
    PaymentChanged = 8,
    /// Platform-native event.
    Native = 63,
}

impl EventKind {
    /// Returns the bit used by [`EventKindSet`].
    #[must_use]
    pub const fn bit(self) -> u64 {
        1_u64 << (self as u8)
    }
}

/// Compact set of event kinds used by adapter interest plans.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EventKindSet(u64);

impl EventKindSet {
    /// Creates an empty set.
    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Creates a set containing one kind.
    #[must_use]
    pub const fn one(kind: EventKind) -> Self {
        Self(kind.bit())
    }

    /// Inserts a kind.
    pub fn insert(&mut self, kind: EventKind) {
        self.0 |= kind.bit();
    }

    /// Returns whether the set contains a kind.
    #[must_use]
    pub const fn contains(self, kind: EventKind) -> bool {
        self.0 & kind.bit() != 0
    }

    /// Returns the union of two sets.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// Lightweight routing metadata extracted before full event decoding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventIndex {
    /// Runtime bot slot.
    pub bot: BotSlot,
    /// Platform identifier.
    pub platform: PlatformId,
    /// Canonical event kind.
    pub kind: EventKind,
    /// Optional conversation.
    pub conversation: Option<ConversationKey>,
    /// Optional actor.
    pub actor: Option<UserKey>,
    /// Normalized command name without a leading slash.
    pub command: Option<Arc<str>>,
    /// Interaction custom or action identifier.
    pub interaction: Option<Arc<str>>,
    /// Native event type used by native route indexing.
    pub native_type: Option<Arc<str>>,
}

impl EventIndex {
    /// Creates a minimal event index.
    #[must_use]
    pub fn new(bot: BotSlot, platform: PlatformId, kind: EventKind) -> Self {
        Self {
            bot,
            platform,
            kind,
            conversation: None,
            actor: None,
            command: None,
            interaction: None,
            native_type: None,
        }
    }

    /// Approximate retained bytes.
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
            .saturating_add(96)
    }

    /// Determines the default ordered-execution key.
    #[must_use]
    pub fn execution_key(&self, event_id: &EventId) -> ExecutionKey {
        if let Some(conversation) = &self.conversation {
            ExecutionKey::Conversation(conversation.clone())
        } else if let Some(actor) = &self.actor {
            ExecutionKey::User(actor.clone())
        } else {
            ExecutionKey::Event(event_id.clone())
        }
    }
}

/// Shared raw platform payload.
#[derive(Clone, Debug)]
pub enum RawPayload {
    /// Original bytes.
    Bytes(Bytes),
    /// Original JSON retained without conversion to a `Value` tree.
    Json(Arc<RawValue>),
}

impl RawPayload {
    /// Approximate bytes retained by the payload.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        match self {
            Self::Bytes(bytes) => bytes.len(),
            Self::Json(json) => json.get().len(),
        }
    }
}

/// Canonical message-created event.
#[derive(Clone, Debug)]
pub struct MessageCreated {
    /// Message reference.
    pub reference: MessageRef,
    /// Optional sender.
    pub sender: Option<UserKey>,
    /// Ordered portable content.
    pub content: Vec<MessageContent>,
    /// Precomputed plain-text view, when available.
    pub text: Option<Arc<str>>,
    /// Whether the incoming message explicitly mentions the bot.
    pub mentioned_bot: bool,
}

/// Canonical message-updated event.
#[derive(Clone, Debug)]
pub struct MessageUpdated {
    /// Updated message.
    pub message: MessageCreated,
    /// Optional previous portable content.
    pub previous_content: Option<Vec<MessageContent>>,
}

/// Canonical messages-deleted event.
#[derive(Clone, Debug)]
pub struct MessagesDeleted {
    /// Deleted message references.
    pub messages: Vec<MessageRef>,
    /// Optional actor that deleted the messages.
    pub actor: Option<UserKey>,
}

/// Canonical interaction event.
#[derive(Clone, Debug)]
pub struct Interaction {
    /// Platform interaction identifier.
    pub id: Arc<str>,
    /// Callback, action, or custom identifier.
    pub custom_id: Option<Arc<str>>,
    /// Actor.
    pub actor: UserKey,
    /// Conversation, if any.
    pub conversation: Option<ConversationKey>,
    /// Submitted string values.
    pub values: Vec<Arc<str>>,
    /// Platform-native fields.
    pub native: Option<NativeData>,
}

/// Portable reaction identifier.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Reaction {
    /// Unicode emoji.
    Unicode(Arc<str>),
    /// Platform custom emoji or reaction identifier.
    Custom(CompactId),
}

impl Reaction {
    /// Approximate retained bytes.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        match self {
            Self::Unicode(value) => value.len(),
            Self::Custom(value) => value.estimated_bytes(),
        }
    }
}

/// Canonical reaction event.
#[derive(Clone, Debug)]
pub struct ReactionChanged {
    /// Message whose reactions changed.
    pub message: MessageRef,
    /// Actor, when exposed by the platform.
    pub actor: Option<UserKey>,
    /// Reactions added by this update.
    pub added: Vec<Reaction>,
    /// Reactions removed by this update.
    pub removed: Vec<Reaction>,
}

/// Canonical membership event.
#[derive(Clone, Debug)]
pub struct MemberChanged {
    /// Conversation whose membership changed.
    pub conversation: ConversationKey,
    /// Affected member.
    pub member: UserKey,
    /// Whether the member is currently present.
    pub present: bool,
    /// Optional actor that caused the change.
    pub actor: Option<UserKey>,
    /// Optional platform-native fields.
    pub native: Option<NativeData>,
}

/// Canonical conversation state event.
#[derive(Clone, Debug)]
pub struct ConversationChanged {
    /// Affected conversation.
    pub conversation: ConversationKey,
    /// Optional human-readable change label.
    pub change: Option<Arc<str>>,
    /// Optional platform-native fields.
    pub native: Option<NativeData>,
}

/// Kind of file lifecycle change.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileChangeKind {
    /// A file was shared or uploaded.
    Shared,
    /// File metadata or contents changed.
    Updated,
    /// A file was deleted.
    Deleted,
}

/// Canonical file lifecycle event.
#[derive(Clone, Debug)]
pub struct FileChanged {
    /// Conversation associated with the file, when available.
    pub conversation: Option<ConversationKey>,
    /// Actor, when available.
    pub actor: Option<UserKey>,
    /// File lifecycle operation.
    pub kind: FileChangeKind,
    /// File description, absent for deletion events that expose only an ID.
    pub file: Option<Media>,
    /// Optional platform file identifier.
    pub file_id: Option<CompactId>,
    /// Optional platform-native fields.
    pub native: Option<NativeData>,
}

/// Canonical payment or subscription event.
#[derive(Clone, Debug)]
pub struct PaymentChanged {
    /// Stable platform payment or subscription identifier.
    pub id: CompactId,
    /// Optional actor.
    pub actor: Option<UserKey>,
    /// Portable status label.
    pub status: Arc<str>,
    /// Optional platform-native fields.
    pub native: Option<NativeData>,
}

/// Platform-native event retained losslessly.
#[derive(Clone, Debug)]
pub struct NativeEvent {
    /// Stable platform event type.
    pub kind: Arc<str>,
    /// Native fields.
    pub data: NativeData,
}

/// One canonical event body.
#[derive(Clone, Debug)]
pub enum EventBody {
    /// Message created.
    MessageCreated(Box<MessageCreated>),
    /// Message updated.
    MessageUpdated(Box<MessageUpdated>),
    /// Messages deleted.
    MessagesDeleted(Box<MessagesDeleted>),
    /// Interaction.
    Interaction(Box<Interaction>),
    /// Reaction changed.
    ReactionChanged(Box<ReactionChanged>),
    /// Membership changed.
    MemberChanged(Box<MemberChanged>),
    /// Conversation changed.
    ConversationChanged(Box<ConversationChanged>),
    /// File changed.
    FileChanged(Box<FileChanged>),
    /// Payment changed.
    PaymentChanged(Box<PaymentChanged>),
    /// Native event.
    Native(Box<NativeEvent>),
}

impl EventBody {
    /// Returns the canonical kind for this body.
    #[must_use]
    pub const fn kind(&self) -> EventKind {
        match self {
            Self::MessageCreated(_) => EventKind::MessageCreated,
            Self::MessageUpdated(_) => EventKind::MessageUpdated,
            Self::MessagesDeleted(_) => EventKind::MessagesDeleted,
            Self::Interaction(_) => EventKind::Interaction,
            Self::ReactionChanged(_) => EventKind::ReactionChanged,
            Self::MemberChanged(_) => EventKind::MemberChanged,
            Self::ConversationChanged(_) => EventKind::ConversationChanged,
            Self::FileChanged(_) => EventKind::FileChanged,
            Self::PaymentChanged(_) => EventKind::PaymentChanged,
            Self::Native(_) => EventKind::Native,
        }
    }

    /// Approximate retained bytes used by bounded queues.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        match self {
            Self::MessageCreated(message) => message
                .content
                .iter()
                .map(MessageContent::estimated_bytes)
                .sum::<usize>()
                .saturating_add(message.content.capacity() * std::mem::size_of::<MessageContent>())
                .saturating_add(message.reference.estimated_bytes())
                .saturating_add(message.sender.as_ref().map_or(0, UserKey::estimated_bytes))
                .saturating_add(message.text.as_ref().map_or(0, |text| text.len()))
                .saturating_add(128),
            Self::MessageUpdated(event) => event
                .message
                .content
                .iter()
                .map(MessageContent::estimated_bytes)
                .sum::<usize>()
                .saturating_add(
                    event.message.content.capacity() * std::mem::size_of::<MessageContent>(),
                )
                .saturating_add(event.message.reference.estimated_bytes())
                .saturating_add(
                    event
                        .message
                        .sender
                        .as_ref()
                        .map_or(0, UserKey::estimated_bytes),
                )
                .saturating_add(event.message.text.as_ref().map_or(0, |text| text.len()))
                .saturating_add(event.previous_content.as_ref().map_or(0, |content| {
                    content
                        .iter()
                        .map(MessageContent::estimated_bytes)
                        .sum::<usize>()
                        .saturating_add(content.capacity() * std::mem::size_of::<MessageContent>())
                }))
                .saturating_add(160),
            Self::MessagesDeleted(event) => event
                .messages
                .iter()
                .map(MessageRef::estimated_bytes)
                .sum::<usize>()
                .saturating_add(event.messages.capacity() * std::mem::size_of::<MessageRef>())
                .saturating_add(event.actor.as_ref().map_or(0, UserKey::estimated_bytes))
                .saturating_add(64),
            Self::Interaction(event) => event
                .values
                .iter()
                .map(|value| value.len())
                .sum::<usize>()
                .saturating_add(event.values.capacity() * std::mem::size_of::<Arc<str>>())
                .saturating_add(event.id.len())
                .saturating_add(event.custom_id.as_ref().map_or(0, |value| value.len()))
                .saturating_add(event.actor.estimated_bytes())
                .saturating_add(
                    event
                        .conversation
                        .as_ref()
                        .map_or(0, ConversationKey::estimated_bytes),
                )
                .saturating_add(event.native.as_ref().map_or(0, NativeData::estimated_bytes))
                .saturating_add(128),
            Self::ReactionChanged(event) => event
                .added
                .iter()
                .chain(event.removed.iter())
                .map(Reaction::estimated_bytes)
                .sum::<usize>()
                .saturating_add(event.added.capacity() * std::mem::size_of::<Reaction>())
                .saturating_add(event.removed.capacity() * std::mem::size_of::<Reaction>())
                .saturating_add(event.message.estimated_bytes())
                .saturating_add(event.actor.as_ref().map_or(0, UserKey::estimated_bytes))
                .saturating_add(128),
            Self::MemberChanged(event) => event
                .conversation
                .estimated_bytes()
                .saturating_add(event.member.estimated_bytes())
                .saturating_add(event.actor.as_ref().map_or(0, UserKey::estimated_bytes))
                .saturating_add(event.native.as_ref().map_or(0, NativeData::estimated_bytes))
                .saturating_add(128),
            Self::ConversationChanged(event) => event
                .change
                .as_ref()
                .map_or(0, |change| change.len())
                .saturating_add(event.conversation.estimated_bytes())
                .saturating_add(event.native.as_ref().map_or(0, NativeData::estimated_bytes))
                .saturating_add(128),
            Self::FileChanged(event) => event
                .file
                .as_ref()
                .map_or(0, Media::estimated_bytes)
                .saturating_add(
                    event
                        .conversation
                        .as_ref()
                        .map_or(0, ConversationKey::estimated_bytes),
                )
                .saturating_add(event.actor.as_ref().map_or(0, UserKey::estimated_bytes))
                .saturating_add(event.file_id.as_ref().map_or(0, CompactId::estimated_bytes))
                .saturating_add(128)
                .saturating_add(event.native.as_ref().map_or(0, NativeData::estimated_bytes)),
            Self::PaymentChanged(event) => event
                .status
                .len()
                .saturating_add(event.id.estimated_bytes())
                .saturating_add(event.actor.as_ref().map_or(0, UserKey::estimated_bytes))
                .saturating_add(event.native.as_ref().map_or(0, NativeData::estimated_bytes))
                .saturating_add(96),
            Self::Native(event) => event
                .data
                .estimated_bytes()
                .saturating_add(event.kind.len())
                .saturating_add(64),
        }
    }
}

/// Adapter-produced event before runtime sequence and receive time are assigned.
#[derive(Clone, Debug)]
pub struct EventDraft {
    /// Stable event identifier.
    pub id: EventId,
    /// Lightweight routing index.
    pub index: EventIndex,
    /// Platform occurrence time.
    pub occurred_at: Option<SystemTime>,
    /// Delivery attempt reported by the platform.
    pub delivery_attempt: u16,
    /// Canonical body.
    pub body: EventBody,
    /// Optional shared raw payload.
    pub raw: Option<Arc<RawPayload>>,
}

impl EventDraft {
    /// Finalizes the event for runtime dispatch.
    #[must_use]
    pub fn finalize(self, sequence: u64, received_at: Instant) -> EventEnvelope {
        EventEnvelope {
            id: self.id,
            sequence,
            index: self.index,
            occurred_at: self.occurred_at,
            received_at,
            delivery_attempt: self.delivery_attempt,
            body: self.body,
            raw: self.raw,
        }
    }

    /// Approximate retained payload bytes.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        self.raw
            .as_ref()
            .map_or(0, |raw| raw.estimated_bytes())
            .saturating_add(self.id.estimated_bytes())
            .saturating_add(self.index.estimated_bytes())
            .saturating_add(self.body.estimated_bytes())
            .saturating_add(192)
    }
}

/// Batch decoded once from one platform frame.
#[derive(Clone, Debug, Default)]
pub struct EventBatch {
    /// Canonical event drafts.
    pub events: Vec<EventDraft>,
}

impl EventBatch {
    /// Creates a batch.
    #[must_use]
    pub fn new(events: impl IntoIterator<Item = EventDraft>) -> Self {
        Self {
            events: events.into_iter().collect(),
        }
    }

    /// Approximate retained bytes. Shared raw payloads are counted once.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        let mut seen = HashSet::new();
        let raw_bytes = self
            .events
            .iter()
            .filter_map(|event| event.raw.as_ref())
            .filter(|raw| seen.insert(Arc::as_ptr(raw) as usize))
            .map(|raw| raw.estimated_bytes())
            .sum::<usize>();
        raw_bytes.saturating_add(
            self.events
                .iter()
                .map(|event| {
                    event
                        .id
                        .estimated_bytes()
                        .saturating_add(event.index.estimated_bytes())
                        .saturating_add(event.body.estimated_bytes())
                        .saturating_add(192)
                })
                .sum::<usize>(),
        )
    }
}

/// Runtime-owned immutable event.
#[derive(Clone, Debug)]
pub struct EventEnvelope {
    /// Stable event identifier.
    pub id: EventId,
    /// Monotonic process-local sequence.
    pub sequence: u64,
    /// Lightweight routing index.
    pub index: EventIndex,
    /// Platform occurrence time.
    pub occurred_at: Option<SystemTime>,
    /// Monotonic receive time.
    pub received_at: Instant,
    /// Platform delivery attempt.
    pub delivery_attempt: u16,
    /// Canonical body.
    pub body: EventBody,
    /// Optional shared raw payload.
    pub raw: Option<Arc<RawPayload>>,
}

impl EventEnvelope {
    /// Returns the message body when this is a message-created event.
    #[must_use]
    pub fn message_created(&self) -> Option<&MessageCreated> {
        match &self.body {
            EventBody::MessageCreated(message) => Some(message),
            _ => None,
        }
    }

    /// Returns the interaction body when this is an interaction event.
    #[must_use]
    pub fn interaction(&self) -> Option<&Interaction> {
        match &self.body {
            EventBody::Interaction(interaction) => Some(interaction),
            _ => None,
        }
    }

    /// Approximate retained bytes used by bounded executor queues.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        self.raw
            .as_ref()
            .map_or(0, |raw| raw.estimated_bytes())
            .saturating_add(self.id.estimated_bytes())
            .saturating_add(self.index.estimated_bytes())
            .saturating_add(self.body.estimated_bytes())
            .saturating_add(192)
    }
}
