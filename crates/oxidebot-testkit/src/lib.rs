//! Deterministic platform fixtures for runtime integration tests.

use async_trait::async_trait;
use oxidebot_core::{
    BotId, BotSlot, CompactId, ConversationKey, EventBatch, EventBody, EventDraft, EventId,
    EventIndex, EventKind, MessageContent, MessageCreated, MessageReceipt, MessageRef,
    MessageTarget, OutgoingMessage, PlatformId, UserKey,
};
use oxidebot_runtime::{
    Adapter, AdapterContext, AdapterError, AdapterMode, BotDescriptor, BotServices, DecodeError,
    FrameIndex, IdempotencyGuarantee, InboundFrame, MessageService, PlatformError,
    PlatformErrorKind,
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

    fn event_index(&self, bot: BotSlot, platform: &PlatformId) -> EventIndex {
        let mut index = EventIndex::new(bot, platform.clone(), EventKind::MessageCreated);
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
}

impl InboundFrame for TestFrame {
    fn index(&self, bot: BotSlot, platform: &PlatformId) -> Result<FrameIndex, DecodeError> {
        Ok(FrameIndex::one(
            self.event_index(bot, platform),
            self.text.len().saturating_add(2_048),
        ))
    }

    fn decode(self, bot: BotSlot, platform: &PlatformId) -> Result<EventBatch, DecodeError> {
        self.decodes.0.fetch_add(1, Ordering::Relaxed);
        let index = self.event_index(bot, platform);
        let conversation = index
            .conversation
            .clone()
            .expect("test message has a conversation");
        let body = MessageCreated {
            reference: MessageRef::new(conversation, self.message_id),
            sender: index.actor.clone(),
            content: vec![MessageContent::Text(self.text.clone())],
            text: Some(self.text),
            mentioned_bot: false,
        };
        Ok(EventBatch::new([EventDraft {
            id: self.id,
            index,
            occurred_at: Some(SystemTime::now()),
            delivery_attempt: 0,
            body: EventBody::MessageCreated(Box::new(body)),
        }]))
    }
}

/// One scripted transport action.
pub enum ScriptStep {
    Frame(TestFrame),
    Pause(Duration),
}

/// Successfully sent message recorded by the scripted service.
#[derive(Clone, Debug)]
pub struct SentMessage {
    pub target: MessageTarget,
    pub message: OutgoingMessage,
}

#[derive(Default)]
struct ServiceState {
    sent: Mutex<Vec<SentMessage>>,
    attempts: AtomicUsize,
    temporary_failures: AtomicUsize,
}

/// Clone-cheap outbound service with deterministic temporary failures.
#[derive(Clone, Default)]
pub struct ScriptedMessageService(Arc<ServiceState>);

impl ScriptedMessageService {
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

#[async_trait]
impl MessageService for ScriptedMessageService {
    async fn send(
        &self,
        target: &MessageTarget,
        message: &OutgoingMessage,
    ) -> Result<MessageReceipt, PlatformError> {
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
            .push(SentMessage {
                target: target.clone(),
                message: message.clone(),
            });
        Ok(MessageReceipt::new(MessageRef::new(
            target.conversation.clone(),
            attempt as u64,
        )))
    }
}

/// Finite adapter that replays a sequence of frames and pauses.
pub struct ScriptedAdapter {
    platform: PlatformId,
    bot: BotId,
    steps: Vec<ScriptStep>,
    service: ScriptedMessageService,
}

impl ScriptedAdapter {
    #[must_use]
    pub fn new(
        platform: PlatformId,
        bot: BotId,
        steps: impl IntoIterator<Item = ScriptStep>,
    ) -> (Self, ScriptedMessageService) {
        let service = ScriptedMessageService::default();
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
        BotServices::messages(Arc::new(self.service.clone()))
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
