use super::{EventBatchFrame, EventFrame, InboundFrame, InterestPlan, MessageFrame};

use crate::{
    budget::{HierarchicalLease, QueueAcquireError, QueueLease, QueueLimiter},
    AdapterError, MetricsHandle, QueueBudget, ShutdownSignal,
};
use oxidebot_core::event::kernel::{DispatchBatch, DispatchIndex};
use oxidebot_core::{
    source::message::Message, BotSlot, CompactId, ConversationRef, Event, EventId, PlatformId,
    RetainedSize,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Result of submitting one platform frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Submission {
    /// The frame could not reach any route or active session and was not decoded.
    Ignored,
    /// The runtime admitted and decoded the frame.
    Accepted {
        /// Number of canonical events admitted from the frame.
        events: usize,
    },
}

pub(crate) struct IngressBatch {
    pub(crate) batch: DispatchBatch,
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

    /// Returns the runtime-assigned slot for this adapter's bot.
    #[must_use]
    pub const fn bot_slot(&self) -> BotSlot {
        self.slot
    }

    /// Returns the routes and active sessions whose traffic this adapter should decode.
    #[must_use]
    pub fn interest(&self) -> &InterestPlan {
        &self.interest
    }

    /// Returns the shared shutdown signal for this adapter run.
    #[must_use]
    pub fn shutdown(&self) -> &ShutdownSignal {
        &self.shutdown
    }

    /// Returns whether runtime shutdown has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    /// Submits one already-normalized standard event.
    ///
    /// This is the preferred adapter API once a wire payload has been mapped
    /// to [`Event`]. The runtime derives routing metadata and admission
    /// accounting; use [`Self::submit`] only when a transport can perform a
    /// cheaper two-stage index/decode operation on its raw frame.
    pub async fn submit_event(
        &self,
        id: EventId,
        event: Event,
    ) -> std::result::Result<Submission, AdapterError> {
        self.submit(EventFrame::new(id, event)).await
    }

    /// Submits a canonical event with adapter-provided occurrence time and
    /// retained-byte accounting.
    pub async fn submit_event_frame(
        &self,
        frame: EventFrame,
    ) -> std::result::Result<Submission, AdapterError> {
        self.submit(frame).await
    }

    /// Submits several canonical events produced by the same transport
    /// delivery. An empty batch is rejected, matching every other inbound
    /// frame contract.
    pub async fn submit_events(
        &self,
        events: impl IntoIterator<Item = EventFrame>,
    ) -> std::result::Result<Submission, AdapterError> {
        self.submit(EventBatchFrame::new(events)).await
    }

    /// Submits one already-canonical portable message without requiring a
    /// simple adapter to implement the two-stage [`InboundFrame`] contract.
    pub async fn submit_message(
        &self,
        id: EventId,
        conversation: ConversationRef,
        actor: impl Into<CompactId>,
        message: Message,
    ) -> std::result::Result<Submission, AdapterError> {
        self.submit(MessageFrame::new(id, conversation, actor, message))
            .await
    }

    /// Convenience form of [`AdapterContext::submit_message`] for transports
    /// whose input unit is a text line.
    pub async fn submit_text(
        &self,
        id: EventId,
        conversation: ConversationRef,
        actor: impl Into<CompactId>,
        message_id: impl Into<oxidebot_core::MessageId>,
        text: impl Into<String>,
    ) -> std::result::Result<Submission, AdapterError> {
        self.submit(MessageFrame::text(
            id,
            conversation,
            actor,
            message_id,
            text,
        ))
        .await
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
    /// event/index validation before publishing the batch.
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
                "decoded frame exceeds the configured event-count limit",
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

        // A frame may contain several events while only a subset is
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
    indexes: &[DispatchIndex],
    slot: BotSlot,
    platform: &PlatformId,
) -> std::result::Result<(), AdapterError> {
    for index in indexes {
        index.validate_for(slot, platform)?;
    }
    Ok(())
}

fn validate_batch(
    batch: &DispatchBatch,
    indexed_events: &[DispatchIndex],
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
