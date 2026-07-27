use crate::{
    budget::{HierarchicalLease, QueueAcquireError, QueueLease, QueueLimiter},
    handler::{RouteScope, RouteSpec},
    AdapterError, BotDescriptor, BotServices, DecodeError, MetricsHandle, QueueBudget,
    ShutdownSignal,
};
use async_trait::async_trait;
use oxidebot_core::{
    BotIdentity, BotSlot, EventBatch, EventIndex, EventKind, EventKindSet, PlatformId, RetainedSize,
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Routing metadata extracted before a platform frame is fully decoded.
#[derive(Clone, Debug, Default)]
pub struct FrameIndex {
    pub events: Vec<EventIndex>,
    /// Conservative retained-byte upper bound for the fully decoded batch.
    pub estimated_bytes: usize,
}

impl FrameIndex {
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
    ) -> std::result::Result<EventBatch, DecodeError>
    where
        Self: Sized,
    {
        self.decode(bot, platform)
    }
}

#[derive(Clone, Debug, Default)]
struct InterestSet {
    generic_kinds: EventKindSet,
    commands: HashSet<Arc<str>>,
    interactions: HashSet<Arc<str>>,
    native_types: HashSet<Arc<str>>,
}

impl InterestSet {
    fn accepts(&self, index: &EventIndex) -> bool {
        if self.generic_kinds.contains(index.kind) {
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

    fn add_route(&mut self, route: &RouteSpec) {
        match route {
            RouteSpec::Generic(kind) => self.generic_kinds.insert(*kind),
            RouteSpec::Command(value) => {
                self.commands.insert(value.clone());
            }
            RouteSpec::Interaction(value) => {
                self.interactions.insert(value.clone());
            }
            RouteSpec::Native(value) => {
                self.native_types.insert(value.clone());
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
struct StaticInterestPlan {
    global: InterestSet,
    platforms: HashMap<PlatformId, InterestSet>,
    bots: HashMap<BotIdentity, InterestSet>,
}

/// Immutable route interest compiled before adapters start.
///
/// Static route sets are shared by `Arc`. Each adapter binds only its identity,
/// so platform- or bot-scoped routes do not cause unrelated transports to fully
/// decode matching traffic.
#[derive(Clone, Debug, Default)]
pub struct InterestPlan {
    static_plan: Arc<StaticInterestPlan>,
    identity: Option<BotIdentity>,
    session_interest: Option<crate::session::SessionInterest>,
}

impl InterestPlan {
    /// Returns whether an index can reach a route or exact active session.
    #[must_use]
    pub fn accepts(&self, index: &EventIndex) -> bool {
        if self.static_plan.global.accepts(index) {
            return true;
        }
        if let Some(identity) = &self.identity {
            if self
                .static_plan
                .platforms
                .get(&identity.platform)
                .is_some_and(|interest| interest.accepts(index))
                || self
                    .static_plan
                    .bots
                    .get(identity)
                    .is_some_and(|interest| interest.accepts(index))
            {
                return true;
            }
        }
        self.session_interest
            .as_ref()
            .is_some_and(|interest| interest.accepts(index))
    }

    pub(crate) fn add_route(&mut self, route: &RouteSpec, scope: &RouteScope) {
        let static_plan = Arc::make_mut(&mut self.static_plan);
        if let Some(bot) = &scope.bot {
            static_plan
                .bots
                .entry(bot.clone())
                .or_default()
                .add_route(route);
        } else if let Some(platform) = &scope.platform {
            static_plan
                .platforms
                .entry(platform.clone())
                .or_default()
                .add_route(route);
        } else {
            static_plan.global.add_route(route);
        }
    }

    pub(crate) fn bind(&self, identity: BotIdentity) -> Self {
        Self {
            static_plan: Arc::clone(&self.static_plan),
            identity: Some(identity),
            session_interest: self.session_interest.clone(),
        }
    }

    pub(crate) fn set_session_interest(&mut self, value: crate::session::SessionInterest) {
        self.session_interest = Some(value);
    }
}

/// Result of submitting one platform frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Submission {
    Ignored,
    Accepted { events: usize },
}

pub(crate) struct IngressBatch {
    pub(crate) batch: EventBatch,
    pub(crate) lease: HierarchicalLease,
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
        let permit = match self.sender.clone().reserve_owned().await {
            Ok(permit) => permit,
            Err(_) => return Err(AdapterError::new("runtime ingress queue is closed")),
        };
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

#[derive(Clone, Copy)]
pub(crate) struct AdapterLimits {
    pub(crate) ingress: QueueBudget,
    pub(crate) max_frame_bytes: usize,
    pub(crate) max_frame_events: usize,
    pub(crate) max_event_bytes: usize,
}

/// Runtime facilities provided to one adapter runner.
#[derive(Clone)]
pub struct AdapterContext {
    slot: BotSlot,
    platform: PlatformId,
    sink: EventSink,
    ingress_limiter: QueueLimiter,
    interest: InterestPlan,
    cancellation: CancellationToken,
    shutdown: ShutdownSignal,
    metrics: MetricsHandle,
    max_frame_bytes: usize,
    max_frame_events: usize,
    max_event_bytes: usize,
}

impl AdapterContext {
    pub(crate) fn new(
        slot: BotSlot,
        platform: PlatformId,
        sink: EventSink,
        limits: AdapterLimits,
        interest: InterestPlan,
        cancellation: CancellationToken,
        metrics: MetricsHandle,
    ) -> Self {
        let shutdown = ShutdownSignal::new(cancellation.clone());
        Self {
            slot,
            platform,
            sink,
            ingress_limiter: QueueLimiter::new(limits.ingress),
            interest,
            cancellation,
            shutdown,
            metrics,
            max_frame_bytes: limits.max_frame_bytes,
            max_frame_events: limits.max_frame_events,
            max_event_bytes: limits.max_event_bytes,
        }
    }

    #[must_use]
    pub const fn bot_slot(&self) -> BotSlot {
        self.slot
    }

    #[must_use]
    pub fn interest(&self) -> &InterestPlan {
        &self.interest
    }

    #[must_use]
    pub fn shutdown(&self) -> &ShutdownSignal {
        &self.shutdown
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    async fn reserve_ingress(
        &self,
        estimated_bytes: usize,
    ) -> std::result::Result<(mpsc::OwnedPermit<IngressBatch>, HierarchicalLease), AdapterError>
    {
        // Local-first ordering prevents one bot from reserving global ingress
        // while it is already above its own item/byte share.
        let local = self
            .ingress_limiter
            .acquire(estimated_bytes)
            .await
            .map_err(map_ingress_error)?;
        let (permit, global) = self.sink.reserve(estimated_bytes).await?;
        Ok((permit, HierarchicalLease::new(local, global)))
    }

    /// Applies interest gating, bounded admission, one full decode, and strict
    /// body/index validation before publishing the batch.
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
        if index.events.len() > self.max_frame_events {
            return Err(AdapterError::new(
                "platform frame index exceeds the configured event-count limit",
            ));
        }
        if index.estimated_bytes > self.max_frame_bytes {
            return Err(AdapterError::new(
                "platform frame exceeds the configured per-frame byte limit",
            ));
        }
        if let Err(error) = validate_indexes(&index.events, self.slot, &self.platform) {
            self.metrics.validation_error();
            return Err(error);
        }
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
                return Err(AdapterError::cancelled("adapter submission was cancelled"));
            }
            admission = self.reserve_ingress(index.estimated_bytes) => {
                match admission {
                    Ok(value) => value,
                    Err(_) if self.cancellation.is_cancelled() => {
                        return Err(AdapterError::cancelled("adapter submission was cancelled"));
                    }
                    Err(error) => return Err(error),
                }
            },
        };
        let mut batch = frame.decode_indexed(self.slot, &self.platform, &index)?;
        if batch.events.len() > self.max_frame_events {
            self.metrics.validation_error();
            return Err(AdapterError::new(
                "decoded frame exceeds the configured canonical-event count limit",
            ));
        }
        if let Err(error) = validate_batch(&batch, &index.events, self.slot, &self.platform) {
            self.metrics.validation_error();
            return Err(error);
        }
        let retained_bytes = batch.retained_bytes();
        if retained_bytes > index.estimated_bytes {
            return Err(AdapterError::new(
                "decoded frame exceeds its pre-decode retained-byte estimate",
            ));
        }
        if retained_bytes > self.max_frame_bytes {
            return Err(AdapterError::new(
                "decoded frame exceeds the configured per-frame byte limit",
            ));
        }
        let decoded_events = batch.events.len();
        self.metrics.decoded_events(decoded_events);

        // A frame may contain several canonical events while only a subset is
        // subscribed. Validate the complete decode, then retain only events that
        // can still reach a route or an exact active session.
        batch
            .events
            .retain(|event| self.interest.accepts(&event.index));
        if batch.events.is_empty() {
            self.metrics.ignored_frame();
            return Ok(Submission::Ignored);
        }
        if batch
            .events
            .iter()
            .any(|event| event.estimated_bytes() > self.max_event_bytes)
        {
            self.metrics.validation_error();
            return Err(AdapterError::new(
                "interested event exceeds the configured per-event byte limit",
            ));
        }
        if self.cancellation.is_cancelled() {
            return Err(AdapterError::cancelled("adapter submission was cancelled"));
        }
        let events = batch.events.len();
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
        index.validate_for(slot, platform)?;
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
        event.index.validate_for(slot, platform)?;
        if &event.index != indexed {
            return Err(AdapterError::new(
                "decoded event routing index differs from the pre-decode frame index",
            ));
        }
        event.validate_for(slot, platform)?;
    }
    Ok(())
}

/// Whether normal completion is meaningful for an adapter.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AdapterMode {
    /// Long-lived network transport. Returning before cancellation is fatal.
    #[default]
    Persistent,
    /// Finite replay/import adapter that may finish successfully.
    Finite,
}

/// One platform transport and its outbound services.
#[async_trait]
pub trait Adapter: Send + 'static {
    fn descriptor(&self) -> BotDescriptor;
    fn services(&self) -> BotServices;
    fn mode(&self) -> AdapterMode {
        AdapterMode::Persistent
    }
    async fn run(self: Box<Self>, context: AdapterContext)
        -> std::result::Result<(), AdapterError>;
}
