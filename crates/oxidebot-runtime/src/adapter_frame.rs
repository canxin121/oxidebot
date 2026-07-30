use crate::DecodeError;
use oxidebot_core::event::kernel::{DispatchBatch, DispatchDraft, DispatchIndex};
use oxidebot_core::event::{EventType, MessageEvent};
use oxidebot_core::{
    source::{message::Message, user::User},
    BotSlot, CompactId, ConversationKey, ConversationRef, Event, EventId, PlatformId, UserKey,
};
use std::{sync::Arc, time::SystemTime};

/// Routing metadata extracted before a platform frame is fully decoded.
#[derive(Clone, Debug, Default)]
pub struct FrameIndex {
    /// Routing indexes for the events expected from this frame.
    pub events: Vec<DispatchIndex>,
    /// Conservative retained-byte upper bound for the fully decoded batch.
    pub estimated_bytes: usize,
}

impl FrameIndex {
    /// Creates an index for a frame that contains exactly one event.
    #[must_use]
    pub fn one(event: DispatchIndex, estimated_bytes: usize) -> Self {
        Self {
            events: vec![event],
            estimated_bytes,
        }
    }
}

/// Two-stage inbound platform frame.
pub trait InboundFrame: Send + 'static {
    /// Extracts only fields needed by interest gating and routing.
    fn index(
        &self,
        bot: BotSlot,
        platform: &PlatformId,
    ) -> std::result::Result<FrameIndex, DecodeError>;

    /// Fully decodes the frame exactly once after bounded admission.
    fn decode(
        self,
        bot: BotSlot,
        platform: &PlatformId,
    ) -> std::result::Result<DispatchBatch, DecodeError>;

    /// Decodes while receiving the already validated routing index.
    ///
    /// The default preserves the simple adapter API. High-throughput adapters
    /// can override this method to reuse command IDs, conversation keys, JSON
    /// offsets, or other work produced during [`Self::index`] instead of
    /// parsing the raw frame twice.
    fn decode_indexed(
        self,
        bot: BotSlot,
        platform: &PlatformId,
        _index: &FrameIndex,
    ) -> std::result::Result<DispatchBatch, DecodeError>
    where
        Self: Sized,
    {
        self.decode(bot, platform)
    }
}

/// A single already-decoded canonical OxideBot event.
///
/// This is the normal inbound boundary for adapters, importers, webhook
/// handlers, and tests which have already mapped their wire payload into the
/// public [`Event`] hierarchy.  The runtime derives the routing index,
/// conversation partition, command key, interaction key, and native event key
/// from that event; adapter authors do not need to use the hidden dispatch
/// kernel types.
///
/// [`InboundFrame`] remains the escape hatch for high-throughput transports
/// that can cheaply index a wire payload before decoding it.  Use
/// [`EventFrame::retained_bytes`] only when an event can retain more than the
/// conservative default, such as an importer attaching a large native value.
pub struct EventFrame {
    id: EventId,
    event: Event,
    occurred_at: Option<SystemTime>,
    retained_bytes: usize,
}

impl EventFrame {
    /// Conservative per-event default that keeps ordinary adapter code free
    /// from manual allocation accounting.  A frame whose actual retained size
    /// exceeds this bound is rejected with a precise error and can be retried
    /// with [`Self::retained_bytes`].
    pub const DEFAULT_RETAINED_BYTES: usize = 64 * 1024;

    /// Creates a canonical frame with receive-time as its occurrence time.
    #[must_use]
    pub fn new(id: EventId, event: Event) -> Self {
        Self {
            id,
            event,
            occurred_at: Some(SystemTime::now()),
            retained_bytes: Self::DEFAULT_RETAINED_BYTES,
        }
    }

    /// Replaces the adapter-reported occurrence time.
    #[must_use]
    pub fn occurred_at(mut self, occurred_at: SystemTime) -> Self {
        self.occurred_at = Some(occurred_at);
        self
    }

    /// Omits an occurrence time when the transport does not provide one.
    #[must_use]
    pub fn without_occurrence_time(mut self) -> Self {
        self.occurred_at = None;
        self
    }

    /// Sets a conservative retained-byte charge for this event.
    ///
    /// The value includes all event-owned payload data but not the runtime's
    /// small dispatch-envelope overhead. It must be non-zero.
    #[must_use]
    pub fn retained_bytes(mut self, retained_bytes: usize) -> Self {
        self.retained_bytes = retained_bytes.max(std::mem::size_of::<Event>());
        self
    }

    fn dispatch_index(&self, bot: BotSlot, platform: &PlatformId) -> DispatchIndex {
        let mut index = DispatchIndex::event(bot, platform.clone(), self.event.event_type());
        match &self.event {
            Event::Message(event) => {
                index.conversation = Some(conversation_key(bot, &event.conversation));
                index.actor = Some(UserKey::new(bot, CompactId::from(event.sender.id.clone())));
                index.command = command_key(&event.message);
            }
            Event::Interaction(event) => {
                index.conversation = event
                    .conversation
                    .as_ref()
                    .map(|conversation| conversation_key(bot, conversation));
                index.actor = Some(UserKey::new(bot, CompactId::from(event.user.id.clone())));
                index.interaction = event.action_id.as_deref().map(Arc::from);
            }
            Event::Request(event) => {
                let (user, conversation) = match event {
                    oxidebot_core::event::RequestEvent::Friend(event) => (&event.user, None),
                    oxidebot_core::event::RequestEvent::GroupJoin(event) => {
                        (&event.user, Some(&event.conversation))
                    }
                    oxidebot_core::event::RequestEvent::GroupInvite(event) => {
                        (&event.user, Some(&event.conversation))
                    }
                };
                index.actor = Some(UserKey::new(bot, CompactId::from(user.id.clone())));
                index.conversation = conversation.map(|value| conversation_key(bot, value));
            }
            Event::Native(event) => index.native_type = Some(Arc::from(event.kind.as_str())),
            Event::Lifecycle(event) => {
                use oxidebot_core::event::LifecycleEvent;
                let (conversation, user) = match event {
                    LifecycleEvent::GroupMemberJoined(event) => {
                        (Some(&event.conversation), Some(&event.user))
                    }
                    LifecycleEvent::GroupMemberLeft(event) => {
                        (Some(&event.conversation), Some(&event.user))
                    }
                    LifecycleEvent::GroupAdminChanged(event) => {
                        (Some(&event.conversation), Some(&event.user))
                    }
                    LifecycleEvent::GroupMuteChanged(event) => {
                        (Some(&event.conversation), event.operator.as_ref())
                    }
                    LifecycleEvent::GroupMemberMuteChanged(event) => {
                        (Some(&event.conversation), Some(&event.user))
                    }
                    LifecycleEvent::GroupHighlightChanged(event) => (
                        Some(&event.conversation),
                        event.sender.as_ref().or(event.operator.as_ref()),
                    ),
                    LifecycleEvent::GroupMemberAliasChanged(event) => {
                        (Some(&event.conversation), Some(&event.user))
                    }
                    LifecycleEvent::MessageReactionsChanged(event) => {
                        (event.conversation.as_ref(), Some(&event.user))
                    }
                    LifecycleEvent::MessageDeleted(event) => (
                        event.conversation.as_ref(),
                        event.user.as_ref().or(event.operator.as_ref()),
                    ),
                    LifecycleEvent::MessageEdited(event) => {
                        (event.conversation.as_ref(), Some(&event.user))
                    }
                    _ => (None, None),
                };
                index.conversation = conversation.map(|value| conversation_key(bot, value));
                index.actor =
                    user.map(|value| UserKey::new(bot, CompactId::from(value.id.clone())));
            }
            Event::Meta(_) => {}
        }
        index
    }
}

fn conversation_key(bot: BotSlot, conversation: &ConversationRef) -> ConversationKey {
    if let Some(parent) = &conversation.parent {
        ConversationKey::new(bot, CompactId::from(parent.id.clone()))
            .in_subspace(CompactId::from(conversation.id.clone()))
    } else {
        ConversationKey::new(bot, CompactId::from(conversation.id.clone()))
    }
}

fn command_key(message: &Message) -> Option<Arc<str>> {
    message
        .get_raw_text()
        .strip_prefix('/')
        .and_then(|value| value.split_whitespace().next())
        .and_then(|value| value.split('@').next())
        .filter(|value| !value.is_empty())
        .map(Arc::from)
}

impl InboundFrame for EventFrame {
    fn index(
        &self,
        bot: BotSlot,
        platform: &PlatformId,
    ) -> std::result::Result<FrameIndex, DecodeError> {
        self.id
            .validate()
            .map_err(|error| DecodeError::new(format!("invalid event id: {error}")))?;
        Ok(FrameIndex::one(
            self.dispatch_index(bot, platform),
            self.retained_bytes.saturating_add(2_048),
        ))
    }

    fn decode(
        self,
        bot: BotSlot,
        platform: &PlatformId,
    ) -> std::result::Result<DispatchBatch, DecodeError> {
        let indexed = FrameIndex::one(self.dispatch_index(bot, platform), 0);
        self.decode_indexed(bot, platform, &indexed)
    }

    fn decode_indexed(
        self,
        _bot: BotSlot,
        _platform: &PlatformId,
        indexed: &FrameIndex,
    ) -> std::result::Result<DispatchBatch, DecodeError> {
        let index = indexed
            .events
            .first()
            .cloned()
            .ok_or_else(|| DecodeError::new("event frame index is empty"))?;
        let mut draft = DispatchDraft::new(self.id, index, self.event, self.retained_bytes);
        draft.occurred_at = self.occurred_at;
        Ok(DispatchBatch::new([draft]))
    }
}

/// A bounded batch of already-decoded canonical events from one transport
/// delivery. This preserves one admission decision while still letting the
/// runtime discard uninterested individual events after validation.
#[derive(Default)]
pub struct EventBatchFrame {
    events: Vec<EventFrame>,
}

impl EventBatchFrame {
    /// Creates a batch from canonical events in one transport delivery.
    #[must_use]
    pub fn new(events: impl IntoIterator<Item = EventFrame>) -> Self {
        Self {
            events: events.into_iter().collect(),
        }
    }

    /// Appends one canonical event to this batch.
    #[must_use]
    pub fn push(mut self, event: EventFrame) -> Self {
        self.events.push(event);
        self
    }

    /// Returns the number of canonical events in this batch.
    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Returns whether this batch contains no canonical events.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

impl InboundFrame for EventBatchFrame {
    fn index(
        &self,
        bot: BotSlot,
        platform: &PlatformId,
    ) -> std::result::Result<FrameIndex, DecodeError> {
        let mut events = Vec::with_capacity(self.events.len());
        let mut estimated_bytes = 0_usize;
        for frame in &self.events {
            let index = frame.index(bot, platform)?;
            events.extend(index.events);
            estimated_bytes = estimated_bytes.saturating_add(index.estimated_bytes);
        }
        Ok(FrameIndex {
            events,
            estimated_bytes,
        })
    }

    fn decode(
        self,
        bot: BotSlot,
        platform: &PlatformId,
    ) -> std::result::Result<DispatchBatch, DecodeError> {
        let indexed = self.index(bot, platform)?;
        self.decode_indexed(bot, platform, &indexed)
    }

    fn decode_indexed(
        self,
        _bot: BotSlot,
        _platform: &PlatformId,
        indexed: &FrameIndex,
    ) -> std::result::Result<DispatchBatch, DecodeError> {
        if indexed.events.len() != self.events.len() {
            return Err(DecodeError::new(
                "canonical event batch index does not match its event count",
            ));
        }
        let events = self
            .events
            .into_iter()
            .zip(indexed.events.iter().cloned())
            .map(|(frame, index)| {
                let mut draft =
                    DispatchDraft::new(frame.id, index, frame.event, frame.retained_bytes);
                draft.occurred_at = frame.occurred_at;
                draft
            });
        Ok(DispatchBatch::new(events))
    }
}

/// Canonical message frame for simple adapters, importers, console transports,
/// and tests. High-throughput adapters may still implement [`InboundFrame`]
/// directly to reuse offsets from their wire format.
pub struct MessageFrame {
    id: EventId,
    conversation: ConversationRef,
    actor: CompactId,
    sender: User,
    message: Message,
    occurred_at: Option<SystemTime>,
}

impl MessageFrame {
    /// Creates a message frame with a default sender carrying the supplied actor ID.
    #[must_use]
    pub fn new(
        id: EventId,
        conversation: ConversationRef,
        actor: impl Into<CompactId>,
        message: Message,
    ) -> Self {
        Self {
            id,
            conversation,
            actor: actor.into(),
            sender: User::new(CompactId::from("pending-actor")),
            message,
            occurred_at: Some(SystemTime::now()),
        }
        .with_default_sender()
    }

    /// Creates a message frame containing a text message with the given message ID.
    #[must_use]
    pub fn text(
        id: EventId,
        conversation: ConversationRef,
        actor: impl Into<CompactId>,
        message_id: impl Into<oxidebot_core::MessageId>,
        text: impl Into<String>,
    ) -> Self {
        let mut message = Message::text(text);
        message.id = Some(message_id.into());
        Self::new(id, conversation, actor, message)
    }

    /// Preserves adapter-provided sender profile data while keeping routing
    /// identity consistent with `sender.id`.
    #[must_use]
    pub fn sender(mut self, sender: User) -> Self {
        self.actor = CompactId::from(sender.id.clone());
        self.sender = sender;
        self
    }

    /// Replaces the occurrence time reported by the adapter.
    #[must_use]
    pub fn occurred_at(mut self, occurred_at: SystemTime) -> Self {
        self.occurred_at = Some(occurred_at);
        self
    }

    fn with_default_sender(mut self) -> Self {
        self.sender.id = self.actor.clone().into();
        self
    }

    fn dispatch_index(&self, bot: BotSlot, platform: &PlatformId) -> DispatchIndex {
        let mut index = DispatchIndex::event(bot, platform.clone(), EventType::Message);
        let conversation = if let Some(parent) = &self.conversation.parent {
            ConversationKey::new(bot, CompactId::from(parent.id.clone()))
                .in_subspace(CompactId::from(self.conversation.id.clone()))
        } else {
            ConversationKey::new(bot, CompactId::from(self.conversation.id.clone()))
        };
        index.conversation = Some(conversation);
        index.actor = Some(UserKey::new(bot, self.actor.clone()));
        index.command = self
            .message
            .get_raw_text()
            .strip_prefix('/')
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.split('@').next())
            .filter(|value| !value.is_empty())
            .map(Arc::from);
        index
    }

    pub(super) fn retained_bytes(&self) -> usize {
        self.id
            .estimated_bytes()
            .saturating_add(conversation_retained_bytes(&self.conversation))
            .saturating_add(self.actor.estimated_bytes())
            .saturating_add(user_retained_bytes(&self.sender))
            .saturating_add(self.message.estimated_bytes())
            .saturating_add(2_048)
    }
}

fn optional_string_bytes(value: &Option<String>) -> usize {
    value
        .as_ref()
        .map_or(0, |value| value.capacity().saturating_add(24))
}

fn user_retained_bytes(user: &User) -> usize {
    let mut bytes = user.id.estimated_bytes().saturating_add(64);
    if let Some(profile) = &user.profile {
        bytes = bytes
            .saturating_add(optional_string_bytes(&profile.display_name))
            .saturating_add(optional_string_bytes(&profile.avatar))
            .saturating_add(optional_string_bytes(&profile.email))
            .saturating_add(optional_string_bytes(&profile.phone))
            .saturating_add(optional_string_bytes(&profile.signature))
            .saturating_add(optional_string_bytes(&profile.level))
            .saturating_add(128);
    }
    bytes
}

fn conversation_retained_bytes(conversation: &ConversationRef) -> usize {
    conversation
        .id
        .estimated_bytes()
        .saturating_add(
            conversation
                .parent
                .as_deref()
                .map_or(0, conversation_retained_bytes),
        )
        .saturating_add(conversation.platform_data.as_ref().map_or(0, |data| {
            data.platform.capacity() + data.data.to_string().len()
        }))
        .saturating_add(128)
}

impl InboundFrame for MessageFrame {
    fn index(
        &self,
        bot: BotSlot,
        platform: &PlatformId,
    ) -> std::result::Result<FrameIndex, DecodeError> {
        self.id
            .validate()
            .map_err(|error| DecodeError::new(format!("invalid message event id: {error}")))?;
        self.conversation
            .id
            .validate()
            .map_err(|error| DecodeError::new(format!("invalid conversation id: {error}")))?;
        self.actor
            .validate()
            .map_err(|error| DecodeError::new(format!("invalid actor id: {error}")))?;
        if let Some(parent) = &self.conversation.parent {
            parent.id.validate().map_err(|error| {
                DecodeError::new(format!("invalid parent conversation id: {error}"))
            })?;
        }
        if self.message.id.is_none() {
            return Err(DecodeError::new("message id must not be empty"));
        }
        Ok(FrameIndex::one(
            self.dispatch_index(bot, platform),
            self.retained_bytes().saturating_add(2_048),
        ))
    }

    fn decode(
        self,
        bot: BotSlot,
        platform: &PlatformId,
    ) -> std::result::Result<DispatchBatch, DecodeError> {
        let indexed = FrameIndex::one(self.dispatch_index(bot, platform), 0);
        self.decode_indexed(bot, platform, &indexed)
    }

    fn decode_indexed(
        self,
        _bot: BotSlot,
        _platform: &PlatformId,
        indexed: &FrameIndex,
    ) -> std::result::Result<DispatchBatch, DecodeError> {
        let index = indexed
            .events
            .first()
            .cloned()
            .ok_or_else(|| DecodeError::new("message frame index is empty"))?;
        index
            .conversation
            .as_ref()
            .ok_or_else(|| DecodeError::new("message frame has no conversation"))?;
        let retained = self.retained_bytes();
        let event = Event::Message(MessageEvent {
            id: self.id.as_str().to_owned(),
            time: None,
            sender: self.sender,
            conversation: self.conversation,
            message: self.message,
        });
        let mut draft = DispatchDraft::new(self.id, index, event, retained);
        draft.occurred_at = self.occurred_at;
        Ok(DispatchBatch::new([draft]))
    }
}

/// Builder for adapters that receive a canonical message but want named setup
/// methods rather than implementing the two-stage frame contract manually.
pub struct MessageFrameBuilder {
    frame: MessageFrame,
}

impl MessageFrameBuilder {
    /// Starts a named builder for a canonical message frame.
    #[must_use]
    pub fn new(
        id: EventId,
        conversation: ConversationRef,
        actor: impl Into<CompactId>,
        message: Message,
    ) -> Self {
        Self {
            frame: MessageFrame::new(id, conversation, actor, message),
        }
    }

    /// Supplies the full sender profile for the frame.
    #[must_use]
    pub fn sender(mut self, sender: User) -> Self {
        self.frame = self.frame.sender(sender);
        self
    }

    /// Supplies the occurrence time reported by the adapter.
    #[must_use]
    pub fn occurred_at(mut self, occurred_at: SystemTime) -> Self {
        self.frame = self.frame.occurred_at(occurred_at);
        self
    }

    /// Finishes the builder and returns its canonical message frame.
    #[must_use]
    pub fn build(self) -> MessageFrame {
        self.frame
    }
}
