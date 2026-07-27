use crate::{
    budget::{QueueAcquireError, QueueLease, QueueLimiter},
    AdapterError, BotDescriptor, BotServices, DecodeError, MetricsHandle, QueueBudget,
};
use async_trait::async_trait;
use oxidebot_core::{BotSlot, EventBatch, EventIndex, EventKind, EventKindSet, PlatformId};
use std::{collections::HashSet, sync::Arc};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Routing metadata extracted before a platform frame is fully decoded.
#[derive(Clone, Debug, Default)]
pub struct FrameIndex {
    /// Canonical events the frame may produce.
    pub events: Vec<EventIndex>,
    /// Conservative retained-byte estimate used for admission before decode.
    pub estimated_bytes: usize,
}

impl FrameIndex {
    /// Creates an index for a single canonical event.
    #[must_use]
    pub fn one(event: EventIndex, estimated_bytes: usize) -> Self {
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
    ) -> std::result::Result<EventBatch, DecodeError>;
}

/// Immutable route interest compiled before adapters start.
#[derive(Clone, Debug, Default)]
pub struct InterestPlan {
    generic_kinds: EventKindSet,
    commands: HashSet<Arc<str>>,
    interactions: HashSet<Arc<str>>,
    native_types: HashSet<Arc<str>>,
    session_interest: Option<crate::session::SessionInterest>,
}

impl InterestPlan {
    /// Returns whether an index can reach a route or exact active session.
    #[must_use]
    pub fn accepts(&self, index: &EventIndex) -> bool {
        if self.generic_kinds.contains(index.kind) {
            return true;
        }
        if self
            .session_interest
            .as_ref()
            .is_some_and(|interest| interest.accepts(index))
        {
            return true;
        }
        match index.kind {
            EventKind::MessageCreated => index
                .command
                .as_ref()
                .is_some_and(|value| self.commands.contains(value)),
            EventKind::Interaction => index
                .interaction
                .as_ref()
                .is_some_and(|value| self.interactions.contains(value)),
            EventKind::Native => index
                .native_type
                .as_ref()
                .is_some_and(|value| self.native_types.contains(value)),
            EventKind::MessageUpdated
            | EventKind::MessagesDeleted
            | EventKind::ReactionChanged
            | EventKind::MemberChanged
            | EventKind::ConversationChanged
            | EventKind::FileChanged
            | EventKind::PaymentChanged => false,
        }
    }

    pub(crate) fn add_generic(&mut self, kind: EventKind) {
        self.generic_kinds.insert(kind);
    }

    pub(crate) fn add_command(&mut self, value: Arc<str>) {
        self.commands.insert(value);
    }

    pub(crate) fn add_interaction(&mut self, value: Arc<str>) {
        self.interactions.insert(value);
    }

    pub(crate) fn add_native_type(&mut self, value: Arc<str>) {
        self.native_types.insert(value);
    }

    pub(crate) fn set_session_interest(&mut self, value: crate::session::SessionInterest) {
        self.session_interest = Some(value);
    }
}

/// Result of submitting one platform frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Submission {
    /// Full decoding was skipped because nothing was interested.
    Ignored,
    /// The decoded canonical events were accepted.
    Accepted {
        /// Number of canonical events accepted.
        events: usize,
    },
}

pub(crate) struct IngressBatch {
    pub(crate) batch: EventBatch,
    pub(crate) lease: QueueLease,
}

#[derive(Clone)]
pub(crate) struct EventSink {
    sender: mpsc::Sender<IngressBatch>,
    limiter: QueueLimiter,
}

impl EventSink {
    pub(crate) fn channel(budget: QueueBudget) -> (Self, mpsc::Receiver<IngressBatch>) {
        let (sender, receiver) = mpsc::channel(budget.max_items);
        (
            Self {
                sender,
                limiter: QueueLimiter::new(budget),
            },
            receiver,
        )
    }

    async fn reserve(
        &self,
        estimated_bytes: usize,
    ) -> std::result::Result<(mpsc::OwnedPermit<IngressBatch>, QueueLease), AdapterError> {
        let lease = self
            .limiter
            .acquire(estimated_bytes)
            .await
            .map_err(map_ingress_error)?;
        let permit = self
            .sender
            .clone()
            .reserve_owned()
            .await
            .map_err(|_| AdapterError::new("runtime ingress queue is closed"))?;
        Ok((permit, lease))
    }
}

fn map_ingress_error(error: QueueAcquireError) -> AdapterError {
    match error {
        QueueAcquireError::Closed => AdapterError::new("runtime ingress budget is closed"),
        QueueAcquireError::Full => AdapterError::new("runtime ingress budget is full"),
        QueueAcquireError::TooLarge => {
            AdapterError::new("platform frame exceeds the ingress byte budget")
        }
    }
}

/// Runtime facilities provided to one adapter runner.
#[derive(Clone)]
pub struct AdapterContext {
    slot: BotSlot,
    platform: PlatformId,
    sink: EventSink,
    interest: InterestPlan,
    cancellation: CancellationToken,
    metrics: MetricsHandle,
}

impl AdapterContext {
    pub(crate) fn new(
        slot: BotSlot,
        platform: PlatformId,
        sink: EventSink,
        interest: InterestPlan,
        cancellation: CancellationToken,
        metrics: MetricsHandle,
    ) -> Self {
        Self {
            slot,
            platform,
            sink,
            interest,
            cancellation,
            metrics,
        }
    }

    /// Returns the adapter's dense bot slot.
    #[must_use]
    pub const fn bot_slot(&self) -> BotSlot {
        self.slot
    }

    /// Returns the compiled interest plan.
    #[must_use]
    pub fn interest(&self) -> &InterestPlan {
        &self.interest
    }

    /// Returns a cancellation token for transport loops.
    #[must_use]
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    /// Returns whether structured shutdown has started.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    /// Applies interest gating, bounded admission, and one full decode.
    pub async fn submit<F>(&self, frame: F) -> std::result::Result<Submission, AdapterError>
    where
        F: InboundFrame,
    {
        self.metrics.ingress_frame();
        let index = frame.index(self.slot, &self.platform)?;
        if index.events.is_empty() {
            return Err(AdapterError::new(
                "platform frame produced an empty pre-decode index",
            ));
        }
        validate_indexes(&index.events, self.slot, &self.platform)?;
        if index
            .events
            .iter()
            .all(|event| !self.interest.accepts(event))
        {
            self.metrics.ignored_frame();
            return Ok(Submission::Ignored);
        }

        let (permit, lease) = tokio::select! {
            _ = self.cancellation.cancelled() => {
                return Err(AdapterError::new("adapter submission was cancelled"));
            }
            admission = self.sink.reserve(index.estimated_bytes) => admission?,
        };
        let batch = frame.decode(self.slot, &self.platform)?;
        validate_batch(&batch, &index.events, self.slot, &self.platform)?;
        if batch.estimated_bytes() > index.estimated_bytes {
            return Err(AdapterError::new(
                "decoded frame exceeds its pre-decode retained-byte estimate",
            ));
        }
        if self.cancellation.is_cancelled() {
            return Err(AdapterError::new("adapter submission was cancelled"));
        }
        let events = batch.events.len();
        self.metrics.decoded_events(events);
        permit.send(IngressBatch { batch, lease });
        Ok(Submission::Accepted { events })
    }
}

fn validate_indexes(
    indexes: &[EventIndex],
    slot: BotSlot,
    platform: &PlatformId,
) -> std::result::Result<(), AdapterError> {
    for index in indexes {
        validate_index(index, slot, platform)?;
    }
    Ok(())
}

fn validate_batch(
    batch: &EventBatch,
    indexed_events: &[EventIndex],
    slot: BotSlot,
    platform: &PlatformId,
) -> std::result::Result<(), AdapterError> {
    if batch.events.is_empty() {
        return Err(AdapterError::new(
            "an interested frame decoded to an empty event batch",
        ));
    }
    if batch.events.len() != indexed_events.len() {
        return Err(AdapterError::new(
            "decoded event count does not match the pre-decode frame index",
        ));
    }
    for (event, indexed) in batch.events.iter().zip(indexed_events) {
        validate_index(&event.index, slot, platform)?;
        if &event.index != indexed {
            return Err(AdapterError::new(
                "decoded event routing index differs from the pre-decode frame index",
            ));
        }
        if event.index.kind != event.body.kind() {
            return Err(AdapterError::new(
                "decoded event body does not match its routing index",
            ));
        }
    }
    Ok(())
}

fn validate_index(
    index: &EventIndex,
    slot: BotSlot,
    platform: &PlatformId,
) -> std::result::Result<(), AdapterError> {
    if index.bot != slot || &index.platform != platform {
        return Err(AdapterError::new(
            "adapter emitted an event for a different bot or platform",
        ));
    }
    if index
        .conversation
        .as_ref()
        .is_some_and(|conversation| conversation.bot != slot)
        || index.actor.as_ref().is_some_and(|actor| actor.bot != slot)
    {
        return Err(AdapterError::new(
            "event conversation or actor uses a different bot slot",
        ));
    }
    Ok(())
}

/// One platform transport and its outbound services.
#[async_trait]
pub trait Adapter: Send + 'static {
    /// Returns immutable identity before the adapter starts.
    fn descriptor(&self) -> BotDescriptor;

    /// Returns split outbound services before the adapter starts.
    fn services(&self) -> BotServices;

    /// Runs the inbound transport until cancellation, completion, or failure.
    async fn run(self: Box<Self>, context: AdapterContext)
        -> std::result::Result<(), AdapterError>;
}
