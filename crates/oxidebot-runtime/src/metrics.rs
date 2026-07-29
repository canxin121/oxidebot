use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

/// A cheaply cloned, shared handle to the runtime's cumulative metrics.
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

/// Lock-free counters and gauges maintained while a runtime is operating.
///
/// Obtain a point-in-time, internally consistent-per-counter view with
/// [`RuntimeMetrics::snapshot`]. Counters are cumulative for the lifetime of
/// this value; gauges describe the current value and, where available, their
/// lifetime high-water mark.
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

/// A point-in-time view of [`RuntimeMetrics`].
///
/// Each field is loaded independently with relaxed atomics. The snapshot is
/// therefore suitable for telemetry and operational decisions, but is not a
/// transactionally consistent trace of a single instant across all fields.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RuntimeMetricsSnapshot {
    /// Frames admitted from adapters into runtime ingress.
    pub ingress_frames: u64,
    /// Frames skipped by adapter interest or admission checks before decoding.
    pub ignored_frames: u64,
    /// Canonical events successfully decoded from admitted frames.
    pub decoded_events: u64,
    /// Events rejected because they failed runtime validation.
    pub validation_errors: u64,
    /// Events suppressed because their deduplication identity was already seen.
    pub duplicate_events: u64,
    /// Events whose identity could not be retained by the deduplication cache.
    pub dedupe_uncacheable: u64,
    /// Events accepted and submitted to the executor.
    pub dispatched_events: u64,
    /// Events dropped by the configured overload policy.
    pub dropped_events: u64,
    /// Events rejected by ingress or execution budget admission.
    pub rejected_events: u64,
    /// Events that found no candidate active dialogue session on the fast path.
    pub session_fast_misses: u64,
    /// Events consumed by an active dialogue session.
    pub session_consumed: u64,
    /// Total route candidates considered by dispatch.
    pub route_candidates: u64,
    /// Filter panics isolated by the runtime.
    pub filter_panics: u64,
    /// Handler invocations that began execution.
    pub handler_calls: u64,
    /// Handler panics isolated by the runtime.
    pub handler_panics: u64,
    /// Handler invocations that exceeded their configured deadline.
    pub handler_timeouts: u64,
    /// Handler-produced effects rejected by validation or policy.
    pub handler_effect_rejections: u64,
    /// Outbound platform commands admitted to command processing.
    pub commands: u64,
    /// Outbound commands cancelled before completion.
    pub cancelled_commands: u64,
    /// Platform command attempts that completed with an error.
    pub command_errors: u64,
    /// Additional command attempts scheduled after a retryable failure.
    pub command_retries: u64,
    /// Command failures classified as platform rate limits.
    pub rate_limits: u64,
    /// Delivery attempts that completed with an error.
    pub delivery_errors: u64,
    /// Delivery middleware panics isolated by the runtime.
    pub delivery_panics: u64,
    /// Delivery plans degraded because an adapter could not support every requested feature.
    pub delivery_degradations: u64,
    /// Sum of nanoseconds commands spent waiting in the outbound queue.
    pub command_queue_wait_nanos: u64,
    /// Number of samples contributing to [`Self::command_queue_wait_nanos`].
    pub command_queue_wait_samples: u64,
    /// Sum of nanoseconds spent executing outbound commands.
    pub command_duration_nanos: u64,
    /// Number of samples contributing to [`Self::command_duration_nanos`].
    pub command_duration_samples: u64,
    /// Current number of commands waiting in the outbound queue.
    pub outbound_queue_depth: u64,
    /// Largest observed [`Self::outbound_queue_depth`] during this runtime's lifetime.
    pub outbound_queue_high_water: u64,
    /// Current number of outbound commands executing against adapters.
    pub outbound_in_flight: u64,
    /// Current number of active dialogue sessions.
    pub active_sessions: u64,
    /// Largest observed [`Self::active_sessions`] during this runtime's lifetime.
    pub active_sessions_high_water: u64,
    /// Current number of handler invocations executing.
    pub active_handlers: u64,
    /// Sum of nanoseconds spent in handler invocations.
    pub handler_duration_nanos: u64,
    /// Number of samples contributing to [`Self::handler_duration_nanos`].
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

    /// Captures the current counters and gauges without blocking runtime work.
    ///
    /// Fields are individually loaded with relaxed atomics; see
    /// [`RuntimeMetricsSnapshot`] for the resulting consistency guarantee.
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
