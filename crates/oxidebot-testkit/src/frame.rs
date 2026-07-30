//! Canonical deterministic inbound frames used by adapter and runtime tests.

use oxidebot_core::{
    conversation::ConversationRef,
    event::kernel::{DispatchBatch, DispatchDraft, DispatchIndex},
    event::{Event, EventType, MessageEvent},
    interaction::{InteractionEvent, InteractionKind, InteractionResponseHandle},
    source::{
        message::{Message, MessageSegment},
        user::User,
    },
    BotSlot, CompactId, ConversationKey, EventId, PlatformId, UserKey,
};
use oxidebot_runtime::{DecodeError, FrameIndex, InboundFrame};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, SystemTime},
};

/// Observable decode counter shared with a [`TestFrame`].
#[derive(Clone, Debug, Default)]
pub struct DecodeCounter(Arc<AtomicUsize>);

impl DecodeCounter {
    /// Returns how many times the associated frame has been fully decoded.
    #[must_use]
    pub fn get(&self) -> usize {
        self.0.load(Ordering::Relaxed)
    }
}

/// One deterministic incoming message frame.
pub struct TestFrame {
    id: EventId,
    conversation: CompactId,
    actor: CompactId,
    message_id: CompactId,
    text: Arc<str>,
    decodes: DecodeCounter,
}
impl TestFrame {
    /// Builds one deterministic incoming text-message frame.
    #[must_use]
    pub fn message(
        id: EventId,
        conversation: impl Into<CompactId>,
        actor: impl Into<CompactId>,
        message_id: impl Into<CompactId>,
        text: impl Into<Arc<str>>,
    ) -> Self {
        Self {
            id,
            conversation: conversation.into(),
            actor: actor.into(),
            message_id: message_id.into(),
            text: text.into(),
            decodes: DecodeCounter::default(),
        }
    }
    /// Returns the shared full-decode counter for this frame.
    #[must_use]
    pub fn decode_counter(&self) -> DecodeCounter {
        self.decodes.clone()
    }
    fn event_index(&self, bot: BotSlot, platform: &PlatformId) -> DispatchIndex {
        let mut index = DispatchIndex::event(bot, platform.clone(), EventType::Message);
        index.conversation = Some(ConversationKey::new(bot, self.conversation.clone()));
        index.actor = Some(UserKey::new(bot, self.actor.clone()));
        index.command = self
            .text
            .strip_prefix('/')
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.split('@').next())
            .filter(|value| !value.is_empty())
            .map(Arc::from);
        index
    }
    fn retained_event_bytes(&self) -> usize {
        self.id
            .estimated_bytes()
            .saturating_add(self.conversation.estimated_bytes())
            .saturating_add(self.actor.estimated_bytes())
            .saturating_add(self.message_id.estimated_bytes())
            .saturating_add(self.text.len())
            .saturating_add(2_048)
    }
}
impl InboundFrame for TestFrame {
    fn index(&self, bot: BotSlot, platform: &PlatformId) -> Result<FrameIndex, DecodeError> {
        Ok(FrameIndex::one(
            self.event_index(bot, platform),
            self.retained_event_bytes().saturating_add(2_048),
        ))
    }
    fn decode(self, bot: BotSlot, platform: &PlatformId) -> Result<DispatchBatch, DecodeError> {
        let index = FrameIndex::one(self.event_index(bot, platform), 0);
        self.decode_indexed(bot, platform, &index)
    }
    fn decode_indexed(
        self,
        _bot: BotSlot,
        _platform: &PlatformId,
        indexed: &FrameIndex,
    ) -> Result<DispatchBatch, DecodeError> {
        self.decodes.0.fetch_add(1, Ordering::Relaxed);
        let index = indexed
            .events
            .first()
            .cloned()
            .ok_or_else(|| DecodeError::new("test frame index is empty"))?;
        index
            .conversation
            .as_ref()
            .ok_or_else(|| DecodeError::new("test message has no conversation"))?;
        let retained_event_bytes = self.retained_event_bytes();
        let event = Event::Message(MessageEvent {
            id: self.id.as_str().to_owned(),
            time: None,
            sender: User::new(self.actor.to_string()),
            conversation: ConversationRef::direct(self.conversation.to_string()),
            message: Message {
                id: Some(self.message_id.to_string().into()),
                segments: vec![MessageSegment::text(self.text.to_string())],
                options: Default::default(),
            },
        });
        let mut draft = DispatchDraft::new(self.id, index, event, retained_event_bytes);
        draft.occurred_at = Some(SystemTime::now());
        Ok(DispatchBatch::new([draft]))
    }
}

/// Deterministic answerable interaction frame used by high-level scenarios.
pub struct InteractionFrame {
    id: EventId,
    conversation: CompactId,
    actor: CompactId,
    action_id: Arc<str>,
    response_id: Arc<str>,
    deadline_after: Duration,
}
impl InteractionFrame {
    /// Builds an answerable button-click event with the default five-second deadline.
    #[must_use]
    pub fn click(
        id: EventId,
        conversation: impl Into<CompactId>,
        actor: impl Into<CompactId>,
        action_id: impl Into<Arc<str>>,
    ) -> Self {
        let response_id: Arc<str> = Arc::from(format!("interaction-response-{}", id.as_str()));
        Self {
            id,
            conversation: conversation.into(),
            actor: actor.into(),
            action_id: action_id.into(),
            response_id,
            deadline_after: Duration::from_secs(5),
        }
    }
    /// Replaces the platform acknowledgement deadline for this interaction.
    #[must_use]
    pub const fn deadline_after(mut self, deadline_after: Duration) -> Self {
        self.deadline_after = deadline_after;
        self
    }
    fn event_index(&self, bot: BotSlot, platform: &PlatformId) -> DispatchIndex {
        let mut index = DispatchIndex::event(bot, platform.clone(), EventType::Interaction);
        index.conversation = Some(ConversationKey::new(bot, self.conversation.clone()));
        index.actor = Some(UserKey::new(bot, self.actor.clone()));
        index.interaction = Some(Arc::clone(&self.action_id));
        index
    }
    fn retained_event_bytes(&self) -> usize {
        self.id
            .estimated_bytes()
            .saturating_add(self.conversation.estimated_bytes())
            .saturating_add(self.actor.estimated_bytes())
            .saturating_add(self.action_id.len())
            .saturating_add(self.response_id.len())
            .saturating_add(2_048)
    }
}
impl InboundFrame for InteractionFrame {
    fn index(&self, bot: BotSlot, platform: &PlatformId) -> Result<FrameIndex, DecodeError> {
        Ok(FrameIndex::one(
            self.event_index(bot, platform),
            self.retained_event_bytes().saturating_add(2_048),
        ))
    }
    fn decode(self, bot: BotSlot, platform: &PlatformId) -> Result<DispatchBatch, DecodeError> {
        let index = self.event_index(bot, platform);
        let retained_event_bytes = self.retained_event_bytes();
        let event = Event::Interaction(InteractionEvent {
            id: self.id.as_str().to_owned(),
            kind: InteractionKind::Button,
            action_id: Some(self.action_id.to_string()),
            values: Vec::new(),
            user: User::new(self.actor.to_string()),
            conversation: Some(ConversationRef::direct(self.conversation.to_string())),
            message: None,
            context_id: Some(self.conversation.to_string()),
            response: Some(InteractionResponseHandle {
                id: self.response_id.to_string(),
                deadline: Some(
                    chrono::Utc::now()
                        + chrono::Duration::from_std(self.deadline_after)
                            .unwrap_or(chrono::Duration::MAX),
                ),
                ack_required: true,
                followups_supported: true,
                platform_data: None,
            }),
            fields: Default::default(),
            command: None,
            locale: None,
            permissions: Default::default(),
            data: serde_json::Value::Null,
        });
        let mut draft = DispatchDraft::new(self.id, index, event, retained_event_bytes);
        draft.occurred_at = Some(SystemTime::now());
        Ok(DispatchBatch::new([draft]))
    }
}
