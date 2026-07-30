//! Scripted virtual transport for deterministic runtime tests.
//!
//! It implements the real adapter and API traits while recording every
//! delivery and interaction side effect for assertions by higher-level tests.

use super::frame::{InteractionFrame, TestFrame};
use async_trait::async_trait;
use oxidebot_core::{
    conversation::{ConversationRef, MessageRef, MessageTarget},
    interaction::{InteractionResponse, InteractionResponseHandle},
    source::message::{
        DeliveryItemResult, DeliveryPlan, DeliveryReport, Message, PartialDeliveryError,
    },
    BotCapabilities, BotId, CallApiTrait, CallError, CallResult, InteractionVisibility, PlatformId,
    SupportLevel,
};
use oxidebot_runtime::{
    Adapter, AdapterContext, AdapterError, AdapterMode, BotDescriptor, BotServices,
    IdempotencyGuarantee, PlatformError, PlatformErrorKind,
};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

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
