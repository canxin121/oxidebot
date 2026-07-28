//! Deterministic platform fixtures for runtime integration tests.

use async_trait::async_trait;
use oxidebot_core::event::kernel::{DispatchBatch, DispatchDraft, DispatchIndex};
use oxidebot_core::{
    api::{payload::SendMessageTarget, response::SendMessageResponse},
    event::{Event, EventType, MessageEvent},
    source::{
        message::{Message, MessageSegment},
        user::User,
    },
    BotId, BotSlot, CallApiTrait, CompactId, ConversationKey, EventId, PlatformId, UserKey,
};
use oxidebot_runtime::{
    Adapter, AdapterContext, AdapterError, AdapterMode, BotDescriptor, BotServices, DecodeError,
    FrameIndex, IdempotencyGuarantee, InboundFrame, PlatformError, PlatformErrorKind,
};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime},
};

/// Observable decode counter shared with a [`TestFrame`].
#[derive(Clone, Debug, Default)]
pub struct DecodeCounter(Arc<AtomicUsize>);

impl DecodeCounter {
    #[must_use]
    pub fn get(&self) -> usize {
        self.0.load(Ordering::Relaxed)
    }
}

/// One deterministic incoming message frame.
pub struct TestFrame {
    id: EventId,
    conversation: CompactId,
    subspace: Option<CompactId>,
    actor: CompactId,
    message_id: CompactId,
    text: Arc<str>,
    decodes: DecodeCounter,
}

impl TestFrame {
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
            subspace: None,
            actor: actor.into(),
            message_id: message_id.into(),
            text: text.into(),
            decodes: DecodeCounter::default(),
        }
    }

    #[must_use]
    pub fn in_subspace(mut self, subspace: impl Into<CompactId>) -> Self {
        self.subspace = Some(subspace.into());
        self
    }

    #[must_use]
    pub fn decode_counter(&self) -> DecodeCounter {
        self.decodes.clone()
    }

    fn event_index(&self, bot: BotSlot, platform: &PlatformId) -> DispatchIndex {
        let mut index = DispatchIndex::event(bot, platform.clone(), EventType::Message);
        let mut conversation = ConversationKey::new(bot, self.conversation.clone());
        if let Some(subspace) = &self.subspace {
            conversation = conversation.in_subspace(subspace.clone());
        }
        index.conversation = Some(conversation);
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

    /// Conservative charge for the decoded public event and its routing keys.
    fn retained_event_bytes(&self) -> usize {
        self.id
            .estimated_bytes()
            .saturating_add(self.conversation.estimated_bytes())
            .saturating_add(self.subspace.as_ref().map_or(0, CompactId::estimated_bytes))
            .saturating_add(self.actor.estimated_bytes())
            .saturating_add(self.message_id.estimated_bytes())
            .saturating_add(self.text.len())
            // Covers owned strings, the message-segment vector, enum storage,
            // and the adapter's receive-time metadata.
            .saturating_add(2_048)
    }

    /// Includes the dispatch envelope and one-element batch-vector overhead.
    fn frame_estimate_bytes(&self) -> usize {
        self.retained_event_bytes().saturating_add(2_048)
    }
}

impl InboundFrame for TestFrame {
    fn index(&self, bot: BotSlot, platform: &PlatformId) -> Result<FrameIndex, DecodeError> {
        Ok(FrameIndex::one(
            self.event_index(bot, platform),
            self.frame_estimate_bytes(),
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
        let message_id = self.message_id.to_string();
        let actor_id = self.actor.to_string();
        let text = self.text.to_string();
        let event = Event::MessageEvent(MessageEvent {
            id: self.id.as_str().to_owned(),
            time: None,
            sender: User {
                id: actor_id,
                ..User::default()
            },
            group: None,
            message: Message {
                id: message_id,
                segments: vec![MessageSegment::text(text)],
                options: Default::default(),
            },
        });
        let mut draft = DispatchDraft::new(self.id, index, event, retained_event_bytes);
        draft.occurred_at = Some(SystemTime::now());
        Ok(DispatchBatch::new([draft]))
    }
}

/// One scripted transport action.
pub enum ScriptStep {
    Frame(TestFrame),
    Pause(Duration),
}

/// Successfully sent message recorded by the scripted API.
#[derive(Clone, Debug)]
pub struct SentMessage {
    pub target: SendMessageTarget,
    pub message: Vec<MessageSegment>,
}

#[derive(Default)]
struct ServiceState {
    sent: Mutex<Vec<SentMessage>>,
    attempts: AtomicUsize,
    temporary_failures: AtomicUsize,
}

/// Clone-cheap bot API fixture with deterministic temporary failures.
#[derive(Clone, Default)]
pub struct ScriptedApi(Arc<ServiceState>);

impl ScriptedApi {
    #[must_use]
    pub fn sent(&self) -> Vec<SentMessage> {
        self.0
            .sent
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub fn fail_temporarily(&self, count: usize) {
        self.0.temporary_failures.store(count, Ordering::Release);
    }

    #[must_use]
    pub fn attempts(&self) -> usize {
        self.0.attempts.load(Ordering::Acquire)
    }
}

impl ScriptedApi {
    fn record(
        &self,
        target: SendMessageTarget,
        message: Vec<MessageSegment>,
    ) -> Result<usize, PlatformError> {
        let attempt = self.0.attempts.fetch_add(1, Ordering::AcqRel) + 1;
        if self
            .0
            .temporary_failures
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok()
        {
            return Err(PlatformError::new(
                PlatformErrorKind::Temporary,
                "scripted temporary failure",
            ));
        }
        self.0
            .sent
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(SentMessage { target, message });
        Ok(attempt)
    }
}

#[async_trait]
impl CallApiTrait for ScriptedApi {
    async fn send_message(
        &self,
        message: Vec<MessageSegment>,
        target: SendMessageTarget,
    ) -> anyhow::Result<Vec<SendMessageResponse>> {
        let attempt = self.record(target, message).map_err(anyhow::Error::new)?;
        Ok(vec![SendMessageResponse {
            sent_message_id: attempt.to_string(),
        }])
    }
}

/// Finite adapter that replays a sequence of frames and pauses.
pub struct ScriptedAdapter {
    platform: PlatformId,
    bot: BotId,
    steps: Vec<ScriptStep>,
    service: ScriptedApi,
}

impl ScriptedAdapter {
    #[must_use]
    pub fn new(
        platform: PlatformId,
        bot: BotId,
        steps: impl IntoIterator<Item = ScriptStep>,
    ) -> (Self, ScriptedApi) {
        let service = ScriptedApi::default();
        (
            Self {
                platform,
                bot,
                steps: steps.into_iter().collect(),
                service: service.clone(),
            },
            service,
        )
    }
}

#[async_trait]
impl Adapter for ScriptedAdapter {
    fn descriptor(&self) -> BotDescriptor {
        BotDescriptor::new(self.platform.clone(), self.bot.clone())
    }

    fn services(&self) -> BotServices {
        BotServices::new(Arc::new(self.service.clone()))
            .send_idempotency(IdempotencyGuarantee::AdapterEmulated)
            .idempotent_delete(true)
    }

    fn mode(&self) -> AdapterMode {
        AdapterMode::Finite
    }

    async fn run(self: Box<Self>, context: AdapterContext) -> Result<(), AdapterError> {
        for step in self.steps {
            match step {
                ScriptStep::Frame(frame) => {
                    context.submit(frame).await?;
                }
                ScriptStep::Pause(duration) => {
                    tokio::select! {
                        () = tokio::time::sleep(duration) => {}
                        () = context.shutdown().cancelled() => break,
                    }
                }
            }
        }
        Ok(())
    }
}
