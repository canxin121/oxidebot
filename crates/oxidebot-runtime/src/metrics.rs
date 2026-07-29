use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

pub type MetricsHandle = Arc<RuntimeMetrics>;

/// Cache-line isolated counter. Independent hot counters no longer invalidate
/// one another's cache lines when ingress, executor, and command workers run on
/// different cores.
#[repr(align(64))]
#[derive(Default)]
struct Counter(AtomicU64);

impl Counter {
    #[inline]
    fn add(&self, amount: u64) {
        self.0.fetch_add(amount, Ordering::Relaxed);
    }

    #[inline]
    fn load(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

#[repr(align(64))]
#[derive(Default)]
struct Gauge(AtomicU64);

impl Gauge {
    #[inline]
    fn increment(&self) -> u64 {
        self.0.fetch_add(1, Ordering::Relaxed).saturating_add(1)
    }

    #[inline]
    fn decrement(&self) {
        let mut current = self.0.load(Ordering::Relaxed);
        loop {
            match self.0.compare_exchange_weak(
                current,
                current.saturating_sub(1),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(observed) => current = observed,
            }
        }
    }

    #[inline]
    fn update_max(&self, candidate: u64) {
        self.0.fetch_max(candidate, Ordering::Relaxed);
    }

    #[inline]
    fn load(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

#[derive(Default)]
pub struct RuntimeMetrics {
    ingress_frames: Counter,
    ignored_frames: Counter,
    decoded_events: Counter,
    validation_errors: Counter,
    duplicate_events: Counter,
    dedupe_uncacheable: Counter,
    dispatched_events: Counter,
    dropped_events: Counter,
    rejected_events: Counter,
    session_fast_misses: Counter,
    session_consumed: Counter,
    route_candidates: Counter,
    filter_panics: Counter,
    handler_calls: Counter,
    handler_panics: Counter,
    handler_timeouts: Counter,
    handler_effect_rejections: Counter,
    commands: Counter,
    cancelled_commands: Counter,
    command_errors: Counter,
    command_retries: Counter,
    rate_limits: Counter,
    delivery_errors: Counter,
    delivery_panics: Counter,
    delivery_degradations: Counter,
    command_queue_wait_nanos: Counter,
    command_queue_wait_samples: Counter,
    command_duration_nanos: Counter,
    command_duration_samples: Counter,
    outbound_queue_depth: Gauge,
    outbound_queue_high_water: Gauge,
    outbound_in_flight: Gauge,
    active_sessions: Gauge,
    active_sessions_high_water: Gauge,
    active_handlers: Gauge,
    handler_duration_nanos: Counter,
    handler_duration_samples: Counter,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RuntimeMetricsSnapshot {
    pub ingress_frames: u64,
    pub ignored_frames: u64,
    pub decoded_events: u64,
    pub validation_errors: u64,
    pub duplicate_events: u64,
    pub dedupe_uncacheable: u64,
    pub dispatched_events: u64,
    pub dropped_events: u64,
    pub rejected_events: u64,
    pub session_fast_misses: u64,
    pub session_consumed: u64,
    pub route_candidates: u64,
    pub filter_panics: u64,
    pub handler_calls: u64,
    pub handler_panics: u64,
    pub handler_timeouts: u64,
    pub handler_effect_rejections: u64,
    pub commands: u64,
    pub cancelled_commands: u64,
    pub command_errors: u64,
    pub command_retries: u64,
    pub rate_limits: u64,
    pub delivery_errors: u64,
    pub delivery_panics: u64,
    pub delivery_degradations: u64,
    pub command_queue_wait_nanos: u64,
    pub command_queue_wait_samples: u64,
    pub command_duration_nanos: u64,
    pub command_duration_samples: u64,
    pub outbound_queue_depth: u64,
    pub outbound_queue_high_water: u64,
    pub outbound_in_flight: u64,
    pub active_sessions: u64,
    pub active_sessions_high_water: u64,
    pub active_handlers: u64,
    pub handler_duration_nanos: u64,
    pub handler_duration_samples: u64,
}

impl RuntimeMetrics {
    pub(crate) fn ingress_frame(&self) {
        self.ingress_frames.add(1);
    }

    pub(crate) fn ignored_frame(&self) {
        self.ignored_frames.add(1);
    }

    pub(crate) fn decoded_events(&self, count: usize) {
        self.decoded_events.add(count as u64);
    }

    pub(crate) fn validation_error(&self) {
        self.validation_errors.add(1);
    }

    pub(crate) fn duplicate_event(&self) {
        self.duplicate_events.add(1);
    }

    pub(crate) fn dedupe_uncacheable(&self) {
        self.dedupe_uncacheable.add(1);
    }

    pub(crate) fn dispatched_event(&self) {
        self.dispatched_events.add(1);
    }

    pub(crate) fn dropped_event(&self) {
        self.dropped_events.add(1);
    }

    pub(crate) fn rejected_event(&self) {
        self.rejected_events.add(1);
    }

    pub(crate) fn session_fast_miss(&self) {
        self.session_fast_misses.add(1);
    }

    pub(crate) fn session_consumed(&self) {
        self.session_consumed.add(1);
    }

    pub(crate) fn route_candidates(&self, count: usize) {
        self.route_candidates.add(count as u64);
    }

    pub(crate) fn filter_panic(&self) {
        self.filter_panics.add(1);
    }

    pub(crate) fn handler_call(&self) {
        self.handler_calls.add(1);
    }

    pub(crate) fn handler_started(&self) {
        self.active_handlers.increment();
    }

    pub(crate) fn handler_finished(&self, duration: std::time::Duration) {
        self.active_handlers.decrement();
        self.handler_duration_nanos
            .add(duration.as_nanos().min(u128::from(u64::MAX)) as u64);
        self.handler_duration_samples.add(1);
    }

    pub(crate) fn handler_panic(&self) {
        self.handler_panics.add(1);
    }

    pub(crate) fn handler_timeout(&self) {
        self.handler_timeouts.add(1);
    }

    pub(crate) fn handler_effect_rejection(&self) {
        self.handler_effect_rejections.add(1);
    }

    pub(crate) fn command(&self) {
        self.commands.add(1);
    }

    pub(crate) fn cancelled_command(&self) {
        self.cancelled_commands.add(1);
    }

    pub(crate) fn command_error(&self) {
        self.command_errors.add(1);
    }

    pub(crate) fn command_retry(&self) {
        self.command_retries.add(1);
    }

    pub(crate) fn rate_limit(&self) {
        self.rate_limits.add(1);
    }

    pub(crate) fn delivery_error(&self) {
        self.delivery_errors.add(1);
    }

    pub(crate) fn delivery_panic(&self) {
        self.delivery_panics.add(1);
    }

    pub(crate) fn delivery_degradation(&self) {
        self.delivery_degradations.add(1);
    }

    pub(crate) fn outbound_queued(&self) {
        let depth = self.outbound_queue_depth.increment();
        self.outbound_queue_high_water.update_max(depth);
    }

    pub(crate) fn outbound_dequeued(&self, wait: std::time::Duration) {
        self.outbound_queue_depth.decrement();
        self.command_queue_wait_nanos
            .add(wait.as_nanos().min(u128::from(u64::MAX)) as u64);
        self.command_queue_wait_samples.add(1);
    }

    pub(crate) fn command_started(&self) {
        self.outbound_in_flight.increment();
    }

    pub(crate) fn command_finished(&self, duration: std::time::Duration) {
        self.outbound_in_flight.decrement();
        self.command_duration_nanos
            .add(duration.as_nanos().min(u128::from(u64::MAX)) as u64);
        self.command_duration_samples.add(1);
    }

    pub(crate) fn session_opened(&self) {
        let active = self.active_sessions.increment();
        self.active_sessions_high_water.update_max(active);
    }

    pub(crate) fn session_closed(&self) {
        self.active_sessions.decrement();
    }

    #[must_use]
    pub fn snapshot(&self) -> RuntimeMetricsSnapshot {
        RuntimeMetricsSnapshot {
            ingress_frames: self.ingress_frames.load(),
            ignored_frames: self.ignored_frames.load(),
            decoded_events: self.decoded_events.load(),
            validation_errors: self.validation_errors.load(),
            duplicate_events: self.duplicate_events.load(),
            dedupe_uncacheable: self.dedupe_uncacheable.load(),
            dispatched_events: self.dispatched_events.load(),
            dropped_events: self.dropped_events.load(),
            rejected_events: self.rejected_events.load(),
            session_fast_misses: self.session_fast_misses.load(),
            session_consumed: self.session_consumed.load(),
            route_candidates: self.route_candidates.load(),
            filter_panics: self.filter_panics.load(),
            handler_calls: self.handler_calls.load(),
            handler_panics: self.handler_panics.load(),
            handler_timeouts: self.handler_timeouts.load(),
            handler_effect_rejections: self.handler_effect_rejections.load(),
            commands: self.commands.load(),
            cancelled_commands: self.cancelled_commands.load(),
            command_errors: self.command_errors.load(),
            command_retries: self.command_retries.load(),
            rate_limits: self.rate_limits.load(),
            delivery_errors: self.delivery_errors.load(),
            delivery_panics: self.delivery_panics.load(),
            delivery_degradations: self.delivery_degradations.load(),
            command_queue_wait_nanos: self.command_queue_wait_nanos.load(),
            command_queue_wait_samples: self.command_queue_wait_samples.load(),
            command_duration_nanos: self.command_duration_nanos.load(),
            command_duration_samples: self.command_duration_samples.load(),
            outbound_queue_depth: self.outbound_queue_depth.load(),
            outbound_queue_high_water: self.outbound_queue_high_water.load(),
            outbound_in_flight: self.outbound_in_flight.load(),
            active_sessions: self.active_sessions.load(),
            active_sessions_high_water: self.active_sessions_high_water.load(),
            active_handlers: self.active_handlers.load(),
            handler_duration_nanos: self.handler_duration_nanos.load(),
            handler_duration_samples: self.handler_duration_samples.load(),
        }
    }
}
