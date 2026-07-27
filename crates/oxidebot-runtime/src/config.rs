use crate::{BuildError, OverloadPolicy, QueueBudget};
use oxidebot_core::MessageExecutionPartition;
use std::time::Duration;

/// Predefined resource envelopes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RuntimeProfile {
    Eco,
    #[default]
    Balanced,
    Throughput,
}

/// Complete runtime resource and resilience configuration.
#[derive(Clone, Copy, Debug)]
pub struct RuntimeConfig {
    pub ingress: QueueBudget,
    /// Global executor budget shared across all bots.
    pub executor: QueueBudget,
    /// Per-bot executor budget protecting against noisy neighbours.
    pub executor_per_bot: QueueBudget,
    /// Per-bot command channel capacity (the byte bound is also validated).
    pub command: QueueBudget,
    /// Process-wide command admission budget shared by all bots.
    pub global_command: QueueBudget,
    pub executor_overload: OverloadPolicy,
    pub command_overload: OverloadPolicy,
    pub executor_shards: usize,
    pub executor_in_flight_per_shard: usize,
    pub executor_in_flight_per_bot: usize,
    /// Process-wide cap on platform commands executing or waiting inside services.
    pub command_in_flight_global: usize,
    pub command_in_flight_per_bot: usize,
    pub command_high_priority_burst: usize,
    pub session_shards: usize,
    pub session_commands_per_shard: usize,
    pub max_sessions: usize,
    pub dedupe_capacity: usize,
    pub dedupe_max_bytes: usize,
    pub dedupe_ttl: Duration,
    pub message_execution_partition: MessageExecutionPartition,
    pub handler_timeout: Option<Duration>,
    /// Timeout for one platform command attempt.
    pub command_attempt_timeout: Option<Duration>,
    /// Total command deadline including retry backoff.
    pub command_total_timeout: Option<Duration>,
    pub command_max_retries: u8,
    pub command_retry_base: Duration,
    pub command_retry_max: Duration,
    pub shutdown_grace: Duration,
}

impl RuntimeConfig {
    #[must_use]
    pub fn for_profile(profile: RuntimeProfile) -> Self {
        let (items, bytes, shards, in_flight, handler_timeout) = match profile {
            RuntimeProfile::Eco => (64, 2 * 1024 * 1024, 1, 8, Duration::from_secs(30)),
            RuntimeProfile::Balanced => (512, 16 * 1024 * 1024, 4, 32, Duration::from_secs(60)),
            RuntimeProfile::Throughput => {
                (4096, 128 * 1024 * 1024, 16, 128, Duration::from_secs(30))
            }
        };
        let per_bot_items = (items / 4).max(16);
        let per_bot_bytes = (bytes / 4).max(512 * 1024);
        Self {
            ingress: QueueBudget::new(items, bytes),
            executor: QueueBudget::new(items, bytes),
            executor_per_bot: QueueBudget::new(per_bot_items, per_bot_bytes),
            command: QueueBudget::new(items, bytes),
            global_command: QueueBudget::new(items.saturating_mul(4), bytes.saturating_mul(4)),
            executor_overload: OverloadPolicy::Block,
            command_overload: OverloadPolicy::Block,
            executor_shards: shards,
            executor_in_flight_per_shard: in_flight,
            executor_in_flight_per_bot: in_flight,
            command_in_flight_global: in_flight.saturating_mul(4),
            command_in_flight_per_bot: in_flight,
            command_high_priority_burst: 8,
            session_shards: shards,
            session_commands_per_shard: items,
            max_sessions: items,
            dedupe_capacity: items.saturating_mul(16),
            dedupe_max_bytes: bytes,
            dedupe_ttl: Duration::from_secs(15 * 60),
            message_execution_partition: MessageExecutionPartition::Conversation,
            handler_timeout: Some(handler_timeout),
            command_attempt_timeout: Some(Duration::from_secs(15)),
            command_total_timeout: Some(Duration::from_secs(30)),
            command_max_retries: 3,
            command_retry_base: Duration::from_millis(25),
            command_retry_max: Duration::from_secs(5),
            shutdown_grace: Duration::from_secs(5),
        }
    }

    pub(crate) fn validate(&self) -> Result<(), BuildError> {
        let budgets = [
            self.ingress,
            self.executor,
            self.executor_per_bot,
            self.command,
            self.global_command,
        ];
        if budgets
            .iter()
            .any(|budget| budget.max_items == 0 || budget.max_bytes == 0)
            || self.executor_shards == 0
            || self.executor_in_flight_per_shard == 0
            || self.executor_in_flight_per_bot == 0
            || self.command_in_flight_global == 0
            || self.command_in_flight_per_bot == 0
            || self.command_high_priority_burst == 0
            || self.session_shards == 0
            || self.session_commands_per_shard == 0
            || self.max_sessions == 0
            || self.dedupe_capacity == 0
            || self.dedupe_max_bytes == 0
            || self.dedupe_ttl.is_zero()
            || self.shutdown_grace.is_zero()
        {
            return Err(BuildError::InvalidConfig(
                "runtime capacities and durations must be non-zero",
            ));
        }
        if self.executor_per_bot.max_items > self.executor.max_items
            || self.executor_per_bot.max_bytes > self.executor.max_bytes
            || self.command.max_items > self.global_command.max_items
            || self.command.max_bytes > self.global_command.max_bytes
            || self.command_in_flight_per_bot > self.command_in_flight_global
        {
            return Err(BuildError::InvalidConfig(
                "per-bot budgets and concurrency cannot exceed global limits",
            ));
        }
        if self
            .handler_timeout
            .is_some_and(|duration| duration.is_zero())
            || self
                .command_attempt_timeout
                .is_some_and(|duration| duration.is_zero())
            || self
                .command_total_timeout
                .is_some_and(|duration| duration.is_zero())
            || (self.command_max_retries > 0
                && (self.command_retry_base.is_zero() || self.command_retry_max.is_zero()))
        {
            return Err(BuildError::InvalidConfig(
                "configured handler, command, and retry durations must be non-zero",
            ));
        }
        if self.command_retry_base > self.command_retry_max {
            return Err(BuildError::InvalidConfig(
                "command retry base cannot exceed the retry maximum",
            ));
        }
        let now = std::time::Instant::now();
        if [
            Some(self.dedupe_ttl),
            self.handler_timeout,
            self.command_attempt_timeout,
            self.command_total_timeout,
            Some(self.command_retry_base),
            Some(self.command_retry_max),
            Some(self.shutdown_grace),
        ]
        .into_iter()
        .flatten()
        .any(|duration| now.checked_add(duration).is_none())
        {
            return Err(BuildError::InvalidConfig(
                "configured duration exceeds the platform monotonic clock range",
            ));
        }
        if let (Some(attempt), Some(total)) =
            (self.command_attempt_timeout, self.command_total_timeout)
        {
            if attempt > total {
                return Err(BuildError::InvalidConfig(
                    "command attempt timeout cannot exceed the total timeout",
                ));
            }
        }
        Ok(())
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self::for_profile(RuntimeProfile::Balanced)
    }
}
