//! Deterministic platform fixtures for runtime integration tests.

use async_trait::async_trait;
use oxidebot_core::event::kernel::{DispatchBatch, DispatchDraft, DispatchIndex};
use oxidebot_core::{
    conversation::{ConversationRef, MessageRef, MessageTarget},
    event::{Event, EventType, MessageEvent},
    interaction::{
        InteractionEvent, InteractionKind, InteractionResponse, InteractionResponseHandle,
    },
    source::{
        message::{
            DeliveryItemResult, DeliveryPlan, DeliveryReport, Message, MessageSegment,
            PartialDeliveryError,
        },
        user::User,
    },
    BotCapabilities, BotId, BotSlot, CallApiTrait, CallError, CallResult, CompactId,
    ConversationKey, EventId, InteractionVisibility, PlatformId, SupportLevel, UserKey,
};
use oxidebot_runtime::{
    Adapter, AdapterContext, AdapterError, AdapterMode, BotDescriptor, BotServices, Command,
    CommandArgs, CommandParseError, CommandTree, DecodeError, FrameIndex, FromCommandMatch,
    IdempotencyGuarantee, InboundFrame, PlatformError, PlatformErrorKind,
};
use std::{
    marker::PhantomData,
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

    /// Conservative charge for the decoded public event and its routing keys.
    fn retained_event_bytes(&self) -> usize {
        self.id
            .estimated_bytes()
            .saturating_add(self.conversation.estimated_bytes())
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
        let event = Event::Message(MessageEvent {
            id: self.id.as_str().to_owned(),
            time: None,
            sender: User {
                id: actor_id,
                ..User::default()
            },
            conversation: ConversationRef::direct(self.conversation.to_string()),
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
    /// Submits one deterministic message frame.
    Frame(TestFrame),
    /// Submits one deterministic interaction frame.
    Interaction(InteractionFrame),
    /// Waits for the given virtual transport duration.
    Pause(Duration),
    /// Waits until the scripted API has observed at least `count` successful
    /// sends. This is a deterministic synchronization point for multi-turn
    /// dialogue tests and does not rely on arbitrary sleeps.
    WaitForSent {
        /// Number of successful sends that must be observed.
        count: usize,
        /// Maximum wall-clock wait for the recorded send count.
        timeout: Duration,
    },
    /// Waits until the scripted API has observed interaction side effects.
    WaitForInteractions {
        /// Number of interaction effects that must be observed.
        count: usize,
        /// Maximum wall-clock wait for interaction effects.
        timeout: Duration,
    },
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
        let actor_id = self.actor.to_string();
        let event = Event::Interaction(InteractionEvent {
            id: self.id.as_str().to_owned(),
            kind: InteractionKind::Button,
            action_id: Some(self.action_id.to_string()),
            values: Vec::new(),
            user: User {
                id: actor_id,
                ..User::default()
            },
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

/// Successfully sent message recorded by the scripted API.
#[derive(Clone, Debug)]
pub struct SentMessage {
    /// Target selected by the application delivery pipeline.
    pub target: MessageTarget,
    /// Fully normalized message submitted to the scripted API.
    pub message: Message,
}

/// Scheduler-observable interaction API call recorded by [`ScriptedApi`].
#[derive(Clone, Debug, PartialEq)]
pub enum InteractionCall {
    /// Records an initial interaction acknowledgement or response.
    Answer {
        /// Platform response-handle identity.
        id: String,
        /// Response sent for the interaction.
        response: InteractionResponse,
    },
    /// Records a follow-up interaction message.
    Followup {
        /// Platform response-handle identity.
        id: String,
        /// Follow-up message.
        message: Message,
    },
    /// Records an edit to the original interaction response.
    Edit {
        /// Platform response-handle identity.
        id: String,
        /// Replacement message.
        message: Message,
    },
}

#[derive(Default)]
struct ServiceState {
    sent: Mutex<Vec<SentMessage>>,
    attempts: AtomicUsize,
    temporary_failures: AtomicUsize,
    sent_notify: tokio::sync::Notify,
    interactions: Mutex<Vec<InteractionCall>>,
    interaction_notify: tokio::sync::Notify,
}

/// Clone-cheap bot API fixture with deterministic temporary failures.
#[derive(Clone, Default)]
pub struct ScriptedApi(Arc<ServiceState>);

impl ScriptedApi {
    /// Returns all successfully recorded deliveries in send order.
    #[must_use]
    pub fn sent(&self) -> Vec<SentMessage> {
        self.0
            .sent
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// Returns all recorded interaction side effects in call order.
    #[must_use]
    pub fn interactions(&self) -> Vec<InteractionCall> {
        self.0
            .interactions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// Makes the next `count` delivery attempts return a temporary failure.
    pub fn fail_temporarily(&self, count: usize) {
        self.0.temporary_failures.store(count, Ordering::Release);
    }

    /// Returns total outbound delivery attempts, including injected failures.
    #[must_use]
    pub fn attempts(&self) -> usize {
        self.0.attempts.load(Ordering::Acquire)
    }

    /// Returns the number of successfully recorded sends.
    #[must_use]
    pub fn sent_count(&self) -> usize {
        self.0
            .sent
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .len()
    }

    async fn wait_for_sent(&self, count: usize, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self.sent_count() >= count {
                return true;
            }
            let notified = self.0.sent_notify.notified();
            if self.sent_count() >= count {
                return true;
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return self.sent_count() >= count;
            }
        }
    }

    async fn wait_for_interactions(&self, count: usize, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self.interactions().len() >= count {
                return true;
            }
            let notified = self.0.interaction_notify.notified();
            if self.interactions().len() >= count {
                return true;
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return self.interactions().len() >= count;
            }
        }
    }

    fn record_interaction(&self, call: InteractionCall) {
        self.0
            .interactions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(call);
        self.0.interaction_notify.notify_waiters();
    }
}

impl ScriptedApi {
    fn record(&self, target: MessageTarget, message: Message) -> Result<usize, PlatformError> {
        let attempt = self.0.attempts.fetch_add(1, Ordering::AcqRel) + 1;
        let mut failures = self.0.temporary_failures.load(Ordering::Acquire);
        let should_fail = loop {
            let Some(next) = failures.checked_sub(1) else {
                break false;
            };
            match self.0.temporary_failures.compare_exchange_weak(
                failures,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break true,
                Err(observed) => failures = observed,
            }
        };
        if should_fail {
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
        self.0.sent_notify.notify_one();
        Ok(attempt)
    }
}

#[async_trait]
impl CallApiTrait for ScriptedApi {
    fn bot_capabilities(&self) -> BotCapabilities {
        let mut capabilities = BotCapabilities::portable();
        capabilities.delivery.idempotency_keys = SupportLevel::Native;
        capabilities.interaction_lifecycle.acknowledge = SupportLevel::Native;
        capabilities.interaction_lifecycle.defer = SupportLevel::Native;
        capabilities.interaction_lifecycle.initial_message = SupportLevel::Native;
        capabilities.interaction_lifecycle.followups = SupportLevel::Native;
        capabilities.interaction_lifecycle.edit_original = SupportLevel::Native;
        capabilities
    }

    async fn send_delivery_plan(
        &self,
        target: MessageTarget,
        plan: DeliveryPlan,
    ) -> CallResult<DeliveryReport> {
        let mut messages = Vec::new();
        let mut items = Vec::with_capacity(plan.messages.len());
        for (index, message) in plan.messages.into_iter().enumerate() {
            match self
                .record(target.clone(), message)
                .map_err(|error| match error.kind {
                    PlatformErrorKind::Temporary => CallError::temporary(error.message),
                    PlatformErrorKind::RateLimited => {
                        CallError::rate_limited(error.message, error.retry_after)
                    }
                    PlatformErrorKind::Timeout => CallError::timeout(error.message),
                    PlatformErrorKind::Permanent => CallError::permanent(error.message),
                    PlatformErrorKind::InvalidRequest => CallError::invalid_request(error.message),
                    PlatformErrorKind::Unsupported => CallError::unsupported(error.message),
                    PlatformErrorKind::NotFound => CallError::not_found(error.message),
                }) {
                Ok(attempt) => {
                    let sent = MessageRef::new(attempt.to_string())
                        .in_conversation(target.conversation.clone());
                    messages.push(sent.clone());
                    items.push(DeliveryItemResult {
                        index,
                        messages: vec![sent],
                        error: None,
                    });
                }
                Err(error) if messages.is_empty() => return Err(error),
                Err(error) => {
                    items.push(DeliveryItemResult {
                        index,
                        messages: Vec::new(),
                        error: Some(error.to_string()),
                    });
                    return Err(PartialDeliveryError {
                        report: DeliveryReport {
                            messages,
                            degradations: plan.degradations,
                            items,
                        },
                    }
                    .into());
                }
            }
        }
        Ok(DeliveryReport {
            messages,
            degradations: plan.degradations,
            items,
        })
    }

    async fn answer_interaction(
        &self,
        interaction_id: String,
        response: InteractionResponse,
    ) -> CallResult<()> {
        self.record_interaction(InteractionCall::Answer {
            id: interaction_id,
            response,
        });
        Ok(())
    }

    async fn defer_interaction(
        &self,
        handle: InteractionResponseHandle,
        visibility: InteractionVisibility,
    ) -> CallResult<()> {
        self.answer_interaction(handle.id, InteractionResponse::Defer { visibility })
            .await
    }

    async fn send_interaction_followup(
        &self,
        handle: InteractionResponseHandle,
        message: Message,
    ) -> CallResult<Vec<MessageRef>> {
        let index = self.interactions().len();
        self.record_interaction(InteractionCall::Followup {
            id: handle.id,
            message,
        });
        Ok(vec![MessageRef::new(format!("followup-{index}"))
            .in_conversation(ConversationRef::group("test-room"))])
    }

    async fn edit_interaction_response(
        &self,
        handle: InteractionResponseHandle,
        message: Message,
    ) -> CallResult<()> {
        self.record_interaction(InteractionCall::Edit {
            id: handle.id,
            message,
        });
        Ok(())
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
    /// Creates a finite scripted adapter and the corresponding observable API fixture.
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
                ScriptStep::Interaction(frame) => {
                    context.submit(frame).await?;
                }
                ScriptStep::Pause(duration) => {
                    tokio::select! {
                        () = tokio::time::sleep(duration) => {}
                        () = context.shutdown().cancelled() => break,
                    }
                }
                ScriptStep::WaitForSent { count, timeout } => {
                    tokio::select! {
                        reached = self.service.wait_for_sent(count, timeout) => {
                            if !reached {
                                return Err(AdapterError::new(format!(
                                    "timed out waiting for {count} sent messages; observed {}",
                                    self.service.sent_count(),
                                )));
                            }
                        }
                        () = context.shutdown().cancelled() => break,
                    }
                }
                ScriptStep::WaitForInteractions { count, timeout } => {
                    tokio::select! {
                        reached = self.service.wait_for_interactions(count, timeout) => {
                            if !reached {
                                return Err(AdapterError::new(format!(
                                    "timed out waiting for {count} interaction calls; observed {}",
                                    self.service.interactions().len(),
                                )));
                            }
                        }
                        () = context.shutdown().cancelled() => break,
                    }
                }
            }
        }
        Ok(())
    }
}

/// One high-level expectation in a [`BotTest`] script.
#[derive(Clone, Debug)]
pub enum ReplyExpectation {
    /// Requires exact raw-text equality.
    Exact(String),
    /// Requires raw text to contain the supplied substring.
    Contains(String),
    /// Requires complete message-model equality.
    Message(Box<Message>),
}

/// One expected interaction-side effect in call order.
#[derive(Clone, Debug)]
pub enum InteractionExpectation {
    /// Requires an initial acknowledgement, message, or defer response.
    Acknowledged,
    /// Requires an original-response edit containing the supplied text.
    EditContains(String),
    /// Requires a follow-up response containing the supplied text.
    FollowupContains(String),
}

/// Result of a completed high-level Bot test.
#[derive(Clone, Debug)]
pub struct BotTestReport {
    /// Successful messages captured during the scenario.
    pub sent: Vec<SentMessage>,
    /// Interaction side effects captured during the scenario.
    pub interactions: Vec<InteractionCall>,
    /// Total attempted deliveries, including injected temporary failures.
    pub attempts: usize,
}

/// Concise deterministic test harness layered over [`ScriptedAdapter`].
///
/// It is intentionally a convenience API: the lower-level frame and adapter
/// fixtures remain available for admission, retry, and scheduling tests.
pub struct BotTest<S = ()>
where
    S: Send + Sync + 'static,
{
    state: S,
    module: oxidebot_runtime::Module<S>,
    steps: Vec<ScriptStep>,
    expectations: Vec<ReplyExpectation>,
    interaction_expectations: Vec<InteractionExpectation>,
    next_event: u64,
    pause: Duration,
    expectation_timeout: Duration,
    conversation: Arc<str>,
    actor: Arc<str>,
}

impl BotTest<()> {
    /// Creates an empty test application. Add generated commands or configured
    /// features with [`BotTest::add`].
    #[must_use]
    pub fn empty() -> Self {
        Self::with_state((), oxidebot_runtime::Module::new())
    }

    /// Creates a test harness around one complete stateless module.
    #[must_use]
    pub fn new(module: oxidebot_runtime::Module<()>) -> Self {
        Self::with_state((), module)
    }

    /// Creates a stateless test harness from one feature.
    #[must_use]
    pub fn feature<F>(feature: F) -> Self
    where
        F: oxidebot_runtime::IntoFeature<()>,
    {
        Self::empty().add(feature)
    }
}

impl<S> BotTest<S>
where
    S: Send + Sync + 'static,
{
    /// Creates a scenario with explicit root state and a complete module.
    #[must_use]
    pub fn with_state(state: S, module: oxidebot_runtime::Module<S>) -> Self {
        Self {
            state,
            module,
            steps: Vec::new(),
            expectations: Vec::new(),
            interaction_expectations: Vec::new(),
            next_event: 1,
            pause: Duration::ZERO,
            expectation_timeout: Duration::from_secs(2),
            conversation: Arc::from("test-room"),
            actor: Arc::from("test-user"),
        }
    }

    /// Adds one generated command, locally configured feature, or module.
    #[must_use]
    #[allow(
        clippy::should_implement_trait,
        reason = "`add` installs a feature into this fluent test builder; it is not arithmetic"
    )]
    pub fn add<F>(mut self, feature: F) -> Self
    where
        F: oxidebot_runtime::IntoFeature<S>,
    {
        self.module = self.module.add(feature);
        self
    }

    /// Includes another flat module in the test application.
    #[must_use]
    pub fn include(mut self, module: oxidebot_runtime::Module<S>) -> Self {
        self.module = self.module.include(module);
        self
    }

    /// Adds a delay after each generated message or interaction step.
    #[must_use]
    pub fn settle_for(mut self, duration: Duration) -> Self {
        self.pause = duration;
        self
    }

    /// Sets the timeout used by reply expectations as deterministic transport
    /// barriers. The default is two seconds.
    #[must_use]
    pub fn expect_within(mut self, duration: Duration) -> Self {
        self.expectation_timeout = duration;
        self
    }

    /// Replaces the default conversation identity used by generated events.
    #[must_use]
    pub fn in_conversation(mut self, conversation: impl Into<Arc<str>>) -> Self {
        self.conversation = conversation.into();
        self
    }

    /// Replaces the default sender identity used by generated events.
    #[must_use]
    #[allow(
        clippy::wrong_self_convention,
        reason = "`as_user` is an established fluent scenario-builder phrase"
    )]
    pub fn as_user(mut self, actor: impl Into<Arc<str>>) -> Self {
        self.actor = actor.into();
        self
    }

    /// Adds one incoming text message to the scenario.
    #[must_use]
    pub fn message(mut self, text: impl Into<Arc<str>>) -> Self {
        let sequence = self.next_event;
        self.next_event = self.next_event.saturating_add(1);
        let id = EventId::new(format!("test-event-{sequence}"))
            .expect("generated test event id is valid");
        self.steps.push(ScriptStep::Frame(TestFrame::message(
            id,
            self.conversation.as_ref(),
            self.actor.as_ref(),
            sequence,
            text,
        )));
        if !self.pause.is_zero() {
            self.steps.push(ScriptStep::Pause(self.pause));
        }
        self
    }

    /// Adds an answerable button-click interaction to the scenario.
    #[must_use]
    pub fn click(mut self, action_id: impl Into<Arc<str>>) -> Self {
        self.push_click(action_id.into(), Duration::from_secs(5));
        self
    }

    /// Adds a click with an explicit platform acknowledgement deadline.
    #[must_use]
    pub fn click_with_deadline(
        mut self,
        action_id: impl Into<Arc<str>>,
        deadline_after: Duration,
    ) -> Self {
        self.push_click(action_id.into(), deadline_after);
        self
    }

    fn push_click(&mut self, action_id: Arc<str>, deadline_after: Duration) {
        let sequence = self.next_event;
        self.next_event = self.next_event.saturating_add(1);
        let id = EventId::new(format!("test-interaction-{sequence}"))
            .expect("generated test interaction id is valid");
        self.steps.push(ScriptStep::Interaction(
            InteractionFrame::click(
                id,
                self.conversation.as_ref(),
                self.actor.as_ref(),
                action_id,
            )
            .deadline_after(deadline_after),
        ));
        if !self.pause.is_zero() {
            self.steps.push(ScriptStep::Pause(self.pause));
        }
    }

    /// Adds an explicit pause to a scenario without changing the default
    /// settling delay used after each message.
    #[must_use]
    pub fn pause(mut self, duration: Duration) -> Self {
        self.steps.push(ScriptStep::Pause(duration));
        self
    }

    fn push_expectation(mut self, expectation: ReplyExpectation) -> Self {
        self.expectations.push(expectation);
        self.steps.push(ScriptStep::WaitForSent {
            count: self.expectations.len(),
            timeout: self.expectation_timeout,
        });
        self
    }

    /// Expects one exact-text reply after the preceding scenario steps.
    #[must_use]
    pub fn expect_reply(self, text: impl Into<String>) -> Self {
        self.push_expectation(ReplyExpectation::Exact(text.into()))
    }

    /// Expects one reply containing the supplied text.
    #[must_use]
    pub fn expect_reply_contains(self, text: impl Into<String>) -> Self {
        self.push_expectation(ReplyExpectation::Contains(text.into()))
    }

    /// Expects one reply equal to the complete portable message model.
    #[must_use]
    pub fn expect_message(self, message: Message) -> Self {
        self.push_expectation(ReplyExpectation::Message(Box::new(message)))
    }

    fn push_interaction_expectation(mut self, expectation: InteractionExpectation) -> Self {
        self.interaction_expectations.push(expectation);
        self.steps.push(ScriptStep::WaitForInteractions {
            count: self.interaction_expectations.len(),
            timeout: self.expectation_timeout,
        });
        self
    }

    /// Expects any successful initial interaction acknowledgement, including
    /// an immediate message or defer response.
    #[must_use]
    pub fn expect_interaction_ack(self) -> Self {
        self.push_interaction_expectation(InteractionExpectation::Acknowledged)
    }

    /// Expects an original interaction-response edit containing the supplied text.
    #[must_use]
    pub fn expect_edit_contains(self, text: impl Into<String>) -> Self {
        self.push_interaction_expectation(InteractionExpectation::EditContains(text.into()))
    }

    /// Expects an interaction follow-up containing the supplied text.
    #[must_use]
    pub fn expect_followup_contains(self, text: impl Into<String>) -> Self {
        self.push_interaction_expectation(InteractionExpectation::FollowupContains(text.into()))
    }

    /// Explicitly documents that a scenario must not send any messages.
    #[must_use]
    pub fn expect_no_reply(self) -> Self {
        assert!(
            self.expectations.is_empty(),
            "expect_no_reply cannot follow reply expectations",
        );
        self
    }

    /// Runs the complete finite scenario and verifies all declared expectations.
    pub async fn run(self) -> Result<BotTestReport, String> {
        let platform = PlatformId::new("test").expect("static test platform id is valid");
        let bot = BotId::new("bot").expect("static test bot id is valid");
        let (adapter, api) = ScriptedAdapter::new(platform, bot, self.steps);
        oxidebot_runtime::OxideBot::with_state(self.state)
            .adapter(adapter)
            .include(self.module)
            .run_to_completion()
            .await
            .map_err(|error| error.to_string())?;
        let sent = api.sent();
        let interactions = api.interactions();
        if sent.len() != self.expectations.len() {
            return Err(format!(
                "expected {} replies, observed {}: {:?}",
                self.expectations.len(),
                sent.len(),
                sent,
            ));
        }
        for (index, (actual, expected)) in sent.iter().zip(&self.expectations).enumerate() {
            let actual_message = actual.message.clone();
            let actual_text = actual_message.get_raw_text();
            let matches = match expected {
                ReplyExpectation::Exact(expected) => actual_text == *expected,
                ReplyExpectation::Contains(expected) => actual_text.contains(expected),
                ReplyExpectation::Message(expected) => &actual_message == expected.as_ref(),
            };
            if !matches {
                return Err(format!(
                    "reply {} did not match {:?}; actual message was {:?}",
                    index + 1,
                    expected,
                    actual_message,
                ));
            }
        }
        if interactions.len() != self.interaction_expectations.len() {
            return Err(format!(
                "expected {} interaction calls, observed {}: {:?}",
                self.interaction_expectations.len(),
                interactions.len(),
                interactions,
            ));
        }
        for (index, (actual, expected)) in interactions
            .iter()
            .zip(&self.interaction_expectations)
            .enumerate()
        {
            let matches = match (actual, expected) {
                (InteractionCall::Answer { .. }, InteractionExpectation::Acknowledged) => true,
                (
                    InteractionCall::Edit { message, .. },
                    InteractionExpectation::EditContains(expected),
                ) => message.get_raw_text().contains(expected),
                (
                    InteractionCall::Followup { message, .. },
                    InteractionExpectation::FollowupContains(expected),
                ) => message.get_raw_text().contains(expected),
                _ => false,
            };
            if !matches {
                return Err(format!(
                    "interaction call {} did not match {:?}; actual call was {:?}",
                    index + 1,
                    expected,
                    actual,
                ));
            }
        }
        Ok(BotTestReport {
            sent,
            interactions,
            attempts: api.attempts(),
        })
    }
}

/// Focused parser harness that exercises the same immutable command IR without
/// starting adapters, queues, or the executor.
pub struct CommandTest<T> {
    command: Command,
    _value: PhantomData<fn() -> T>,
}

impl<T> CommandTest<T>
where
    T: FromCommandMatch,
{
    /// Builds a parser harness for one immutable command definition.
    #[must_use]
    pub fn new(command: Command) -> Self {
        Self {
            command,
            _value: PhantomData,
        }
    }

    /// Parses input with the same command IR used by the runtime.
    pub fn parse(&self, input: impl Into<String>) -> Result<T, CommandParseError> {
        let matched = self.command.parse_message(&Message::text(input.into()))?;
        T::from_match(&matched)
    }

    /// Returns the command definition exercised by this harness.
    #[must_use]
    pub fn command(&self) -> &Command {
        &self.command
    }
}

/// Builds a focused parser harness for one flat `CommandArgs` type.
#[must_use]
pub fn command_test<T>(name: impl Into<Arc<str>>) -> CommandTest<T>
where
    T: CommandArgs,
{
    CommandTest::new(T::command(name))
}

/// Builds a focused parser harness for one derived command tree.
#[must_use]
pub fn command_tree_test<T>() -> CommandTest<T>
where
    T: CommandTree,
{
    CommandTest::new(T::command())
}

/// Reusable assertions for adapter delivery implementations.
///
/// Adapter crates can run these checks against their own `DeliveryPlan` and
/// result without starting an OxideBot runtime. They enforce the portable
/// report contract: one ordered item per planned physical message, preserved
/// degradation details, and only successful message references in the summary.
pub mod adapter_contract {
    use oxidebot_core::{CallError, DeliveryPlan, DeliveryReport};

    /// Verifies a fully successful delivery report.
    pub fn assert_complete(plan: &DeliveryPlan, report: &DeliveryReport) -> Result<(), String> {
        if report.degradations != plan.degradations {
            return Err("delivery report did not preserve planner degradations".into());
        }
        if report.items.len() != plan.messages.len() {
            return Err(format!(
                "delivery report contains {} items for {} planned messages",
                report.items.len(),
                plan.messages.len()
            ));
        }
        for (index, item) in report.items.iter().enumerate() {
            if item.index != index {
                return Err(format!(
                    "delivery report item at position {index} claims index {}",
                    item.index
                ));
            }
            if item.error.is_some() {
                return Err(format!("delivery report item {index} unexpectedly failed"));
            }
        }
        let item_messages = report
            .items
            .iter()
            .flat_map(|item| item.messages.iter())
            .collect::<Vec<_>>();
        if report.messages.len() != item_messages.len()
            || !report
                .messages
                .iter()
                .zip(item_messages)
                .all(|(a, b)| a == b)
        {
            return Err("delivery report summary differs from successful item references".into());
        }
        Ok(())
    }

    /// Verifies the structured report carried by a partial-delivery error.
    pub fn assert_partial(plan: &DeliveryPlan, error: &CallError) -> Result<(), String> {
        let CallError::PartialDelivery(partial) = error else {
            return Err("expected CallError::PartialDelivery".into());
        };
        let report = &partial.report;
        if report.degradations != plan.degradations {
            return Err("partial report did not preserve planner degradations".into());
        }
        let Some(failed) = report.items.last() else {
            return Err("partial report has no failed item".into());
        };
        if failed.error.is_none() || !failed.messages.is_empty() {
            return Err("partial report terminal item must fail without message references".into());
        }
        if report.items.len() > plan.messages.len() {
            return Err("partial report has more items than the delivery plan".into());
        }
        for (index, item) in report.items.iter().enumerate() {
            if item.index != index {
                return Err(format!(
                    "partial report item at position {index} claims index {}",
                    item.index
                ));
            }
        }
        Ok(())
    }
}
