use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

pub type MetricsHandle = Arc<RuntimeMetrics>;

#[derive(Default)]
pub struct RuntimeMetrics {
    ingress_frames: AtomicU64,
    ignored_frames: AtomicU64,
    decoded_events: AtomicU64,
    duplicate_events: AtomicU64,
    dispatched_events: AtomicU64,
    dropped_events: AtomicU64,
    session_consumed: AtomicU64,
    handler_panics: AtomicU64,
    commands: AtomicU64,
    command_errors: AtomicU64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RuntimeMetricsSnapshot {
    pub ingress_frames: u64,
    pub ignored_frames: u64,
    pub decoded_events: u64,
    pub duplicate_events: u64,
    pub dispatched_events: u64,
    pub dropped_events: u64,
    pub session_consumed: u64,
    pub handler_panics: u64,
    pub commands: u64,
    pub command_errors: u64,
}

impl RuntimeMetrics {
    fn increment(value: &AtomicU64, amount: u64) {
        value.fetch_add(amount, Ordering::Relaxed);
    }
    pub(crate) fn ingress_frame(&self) {
        Self::increment(&self.ingress_frames, 1);
    }
    pub(crate) fn ignored_frame(&self) {
        Self::increment(&self.ignored_frames, 1);
    }
    pub(crate) fn decoded_events(&self, count: usize) {
        Self::increment(&self.decoded_events, count as u64);
    }
    pub(crate) fn duplicate_event(&self) {
        Self::increment(&self.duplicate_events, 1);
    }
    pub(crate) fn dispatched_event(&self) {
        Self::increment(&self.dispatched_events, 1);
    }
    pub(crate) fn dropped_event(&self) {
        Self::increment(&self.dropped_events, 1);
    }
    pub(crate) fn session_consumed(&self) {
        Self::increment(&self.session_consumed, 1);
    }
    pub(crate) fn handler_panic(&self) {
        Self::increment(&self.handler_panics, 1);
    }
    pub(crate) fn command(&self) {
        Self::increment(&self.commands, 1);
    }
    pub(crate) fn command_error(&self) {
        Self::increment(&self.command_errors, 1);
    }
    #[must_use]
    pub fn snapshot(&self) -> RuntimeMetricsSnapshot {
        let load = |value: &AtomicU64| value.load(Ordering::Relaxed);
        RuntimeMetricsSnapshot {
            ingress_frames: load(&self.ingress_frames),
            ignored_frames: load(&self.ignored_frames),
            decoded_events: load(&self.decoded_events),
            duplicate_events: load(&self.duplicate_events),
            dispatched_events: load(&self.dispatched_events),
            dropped_events: load(&self.dropped_events),
            session_consumed: load(&self.session_consumed),
            handler_panics: load(&self.handler_panics),
            commands: load(&self.commands),
            command_errors: load(&self.command_errors),
        }
    }
}
