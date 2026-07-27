use crate::{BuildError, OverloadPolicy, QueueBudget};
use std::time::Duration;

/// Predefined resource envelopes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RuntimeProfile {
    /// Small queues and conservative concurrency.
    Eco,
    /// Balanced defaults for general bots.
    #[default]
    Balanced,
    /// Larger queues and more parallel work.
    Throughput,
}

/// Complete runtime resource and resilience configuration.
#[derive(Clone, Copy, Debug)]
pub struct RuntimeConfig {
    pub ingress: QueueBudget,
    pub executor: QueueBudget,
    pub command: QueueBudget,
    pub executor_overload: OverloadPolicy,
    pub command_overload: OverloadPolicy,
    pub executor_shards: usize,
    pub executor_in_flight_per_shard: usize,
    pub command_in_flight_per_bot: usize,
    pub session_shards: usize,
    pub session_commands_per_shard: usize,
    pub max_sessions: usize,
    pub dedupe_capacity: usize,
    pub handler_timeout: Option<Duration>,
    pub command_timeout: Option<Duration>,
    pub command_max_retries: u8,
    pub command_retry_base: Duration,
    pub shutdown_grace: Duration,
}

impl RuntimeConfig {
    /// Builds a complete configuration from a predefined profile.
    #[must_use]
    pub fn for_profile(profile: RuntimeProfile) -> Self {
        let (items, bytes, shards, in_flight) = match profile {
            RuntimeProfile::Eco => (64, 2 * 1024 * 1024, 1, 8),
            RuntimeProfile::Balanced => (512, 16 * 1024 * 1024, 4, 32),
            RuntimeProfile::Throughput => (4096, 128 * 1024 * 1024, 16, 128),
        };
        Self {
            ingress: QueueBudget::new(items, bytes),
            executor: QueueBudget::new(items, bytes),
            command: QueueBudget::new(items, bytes),
            executor_overload: OverloadPolicy::Block,
            command_overload: OverloadPolicy::Block,
            executor_shards: shards,
            executor_in_flight_per_shard: in_flight,
            command_in_flight_per_bot: in_flight,
            session_shards: shards,
            session_commands_per_shard: items,
            max_sessions: items,
            dedupe_capacity: items.saturating_mul(16),
            handler_timeout: None,
            command_timeout: Some(Duration::from_secs(30)),
            command_max_retries: 3,
            command_retry_base: Duration::from_millis(25),
            shutdown_grace: Duration::from_secs(5),
        }
    }

    pub(crate) fn validate(&self) -> Result<(), BuildError> {
        let nonzero = self.ingress.max_items > 0
            && self.ingress.max_bytes > 0
            && self.executor.max_items > 0
            && self.executor.max_bytes > 0
            && self.command.max_items > 0
            && self.command.max_bytes > 0
            && self.executor_shards > 0
            && self.executor_in_flight_per_shard > 0
            && self.command_in_flight_per_bot > 0
            && self.session_shards > 0
            && self.session_commands_per_shard > 0
            && self.max_sessions > 0
            && self.dedupe_capacity > 0;
        if nonzero {
            Ok(())
        } else {
            Err(BuildError::InvalidConfig(
                "runtime capacities must be non-zero",
            ))
        }
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self::for_profile(RuntimeProfile::Balanced)
    }
}
