//! Deterministic platform fixtures for runtime integration tests.

use async_trait::async_trait;
use oxidebot_core::{
    BotId, BotSlot, CompactId, ConversationKey, EventBatch, EventBody, EventDraft, EventId,
    EventIndex, EventKind, MessageContent, MessageCreated, MessageReceipt, MessageRef,
    MessageTarget, OutgoingMessage, PlatformId, UserKey,
};
use oxidebot_runtime::{
    Adapter, AdapterContext, AdapterError, BotDescriptor, BotServices, DecodeError, FrameIndex,
    InboundFrame, MessageService, PlatformError, PlatformErrorKind,
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
    /// Returns how often full decoding ran.
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
    /// Creates a canonical message frame.
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

    /// Returns a counter that can verify interest-gated decoding.
    #[must_use]
    pub fn decode_counter(&self) -> DecodeCounter {
        self.decodes.clone()
    }

    fn event_index(&self, bot: BotSlot, platform: &PlatformId) -> EventIndex {
        let mut index = EventIndex::new(bot, platform.clone(), EventKind::MessageCreated);
        index.conversation = Some(ConversationKey::new(bot, self.conversation.clone()));
        index.actor = Some(UserKey::new(bot, self.actor.clone()));
        index.command = self
            .text
            .strip_prefix('/')
            .and_then(|value| value.split_whitespace().next())
            .filter(|value| !value.is_empty())
            .map(Arc::from);
        index
    }
}

impl InboundFrame for TestFrame {
    fn index(&self, bot: BotSlot, platform: &PlatformId) -> Result<FrameIndex, DecodeError> {
        Ok(FrameIndex::one(
            self.event_index(bot, platform),
            self.text.len().saturating_add(1024),
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
            raw: None,
        }]))
    }
}

/// One scripted transport action.
pub enum ScriptStep {
    /// Submit a frame.
    Frame(TestFrame),
    /// Pause the transport.
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
    /// Returns successful sends in acceptance order.
    #[must_use]
    pub fn sent(&self) -> Vec<SentMessage> {
        self.0
            .sent
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
    /// Makes the next `count` attempts fail with a retryable error.
    pub fn fail_temporarily(&self, count: usize) {
        self.0.temporary_failures.store(count, Ordering::Release);
    }
    /// Returns total send attempts, including failures.
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
    /// Creates an adapter and a handle to its observable outbound service.
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
    }
    async fn run(self: Box<Self>, context: AdapterContext) -> Result<(), AdapterError> {
        for step in self.steps {
            match step {
                ScriptStep::Frame(frame) => {
                    context.submit(frame).await?;
                }
                ScriptStep::Pause(duration) => {
                    let cancellation = context.cancellation_token();
                    tokio::select! {
                        () = tokio::time::sleep(duration) => {}
                        () = cancellation.cancelled() => break,
                    }
                }
            }
        }
        Ok(())
    }
}
