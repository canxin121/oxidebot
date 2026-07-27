use crate::{BuildError, OverloadPolicy, QueueBudget};
use oxidebot_core::MessageExecutionPartition;
use std::time::Duration;
use tokio::sync::Semaphore;

/// Upper bound on worker-shard fan-out created by one runtime.
const MAX_RUNTIME_SHARDS: usize = 1_024;
/// Upper bound on aggregate session-command channel slots.
const MAX_SESSION_COMMAND_SLOTS: usize = 1_048_576;

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
    /// Per-bot ingress share preventing one transport from occupying the
    /// entire process queue while other bots are ready.
    pub ingress_per_bot: QueueBudget,
    /// Maximum retained bytes for one decoded platform frame.
    pub max_frame_bytes: usize,
    /// Maximum canonical events one platform frame may expand into.
    pub max_frame_events: usize,
    /// Maximum incremental retained bytes for one canonical event.
    pub max_event_bytes: usize,
    /// Global executor budget shared across all bots.
    pub executor: QueueBudget,
    /// Per-bot executor budget protecting against noisy neighbours.
    pub executor_per_bot: QueueBudget,
    /// Per-bot command channel capacity and byte budget.
    pub command: QueueBudget,
    /// Maximum retained bytes for one outbound command operation.
    pub max_command_bytes: usize,
    /// Process-wide command admission budget shared by all bots.
    pub global_command: QueueBudget,
    pub executor_overload: OverloadPolicy,
    pub command_overload: OverloadPolicy,
    pub executor_shards: usize,
    pub executor_in_flight_per_shard: usize,
    pub executor_in_flight_per_bot: usize,
    /// Process-wide cap on platform calls actively inside adapter services.
    pub command_in_flight_global: usize,
    /// Global capacity unavailable to normal commands and reserved for high-priority interactions.
    pub command_in_flight_reserved_high_global: usize,
    /// Per-bot cap on commands admitted into the running set.
    pub command_in_flight_per_bot: usize,
    /// Per-bot running slots unavailable to normal commands and reserved for interactions.
    pub command_in_flight_reserved_high_per_bot: usize,
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
    /// Total command deadline including queueing and retry backoff.
    pub command_total_timeout: Option<Duration>,
    pub command_max_retries: u8,
    pub command_retry_base: Duration,
    pub command_retry_max: Duration,
    pub shutdown_grace: Duration,
}

impl RuntimeConfig {
    #[must_use]
    pub fn for_profile(profile: RuntimeProfile) -> Self {
        let (items, bytes, shards, in_flight, frame_events, handler_timeout): (
            usize,
            usize,
            usize,
            usize,
            usize,
            Duration,
        ) = match profile {
            RuntimeProfile::Eco => (64, 2 * 1024 * 1024, 1, 8, 32, Duration::from_secs(30)),
            RuntimeProfile::Balanced => {
                (512, 16 * 1024 * 1024, 4, 32, 128, Duration::from_secs(60))
            }
            RuntimeProfile::Throughput => (
                4096,
                128 * 1024 * 1024,
                16,
                128,
                512,
                Duration::from_secs(30),
            ),
        };
        let per_bot_items = (items / 4).max(16);
        let per_bot_bytes = (bytes / 4).max(512 * 1024);
        let command_global_in_flight = in_flight.saturating_mul(4);
        let reserved_high_global = (command_global_in_flight / 16).max(1);
        let reserved_high_per_bot = (in_flight / 16).max(1);
        Self {
            ingress: QueueBudget::new(items, bytes),
            ingress_per_bot: QueueBudget::new(per_bot_items, per_bot_bytes),
            max_frame_bytes: per_bot_bytes,
            max_frame_events: frame_events,
            max_event_bytes: per_bot_bytes,
            executor: QueueBudget::new(items, bytes),
            executor_per_bot: QueueBudget::new(per_bot_items, per_bot_bytes),
            command: QueueBudget::new(items, bytes),
            max_command_bytes: per_bot_bytes,
            global_command: QueueBudget::new(items.saturating_mul(4), bytes.saturating_mul(4)),
            executor_overload: OverloadPolicy::Block,
            command_overload: OverloadPolicy::Block,
            executor_shards: shards,
            executor_in_flight_per_shard: in_flight,
            executor_in_flight_per_bot: in_flight,
            command_in_flight_global: command_global_in_flight,
            command_in_flight_reserved_high_global: reserved_high_global,
            command_in_flight_per_bot: in_flight,
            command_in_flight_reserved_high_per_bot: reserved_high_per_bot,
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
            self.ingress_per_bot,
            self.executor,
            self.executor_per_bot,
            self.command,
            self.global_command,
        ];
        if budgets
            .iter()
            .any(|budget| budget.max_items == 0 || budget.max_bytes == 0)
            || self.max_frame_bytes == 0
            || self.max_frame_events == 0
            || self.max_event_bytes == 0
            || self.max_command_bytes == 0
            || self.command.max_items < 2
            || self.global_command.max_items < 2
            || self.executor_shards == 0
            || self.executor_in_flight_per_shard == 0
            || self.executor_in_flight_per_bot == 0
            || self.command_in_flight_global == 0
            || self.command_in_flight_reserved_high_global == 0
            || self.command_in_flight_per_bot == 0
            || self.command_in_flight_reserved_high_per_bot == 0
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
                "runtime capacities must be non-zero and priority command queues require at least two items",
            ));
        }

        if self.ingress_per_bot.max_items > self.ingress.max_items
            || self.ingress_per_bot.max_bytes > self.ingress.max_bytes
            || self.executor_per_bot.max_items > self.executor.max_items
            || self.executor_per_bot.max_bytes > self.executor.max_bytes
            || self.command.max_items > self.global_command.max_items
            || self.command.max_bytes > self.global_command.max_bytes
            || self.command_in_flight_per_bot > self.command_in_flight_global
            || self.max_frame_bytes > self.ingress.max_bytes
            || self.max_frame_bytes > self.ingress_per_bot.max_bytes
            || self.max_event_bytes > self.max_frame_bytes
            || self.max_event_bytes > self.ingress.max_bytes
            || self.max_event_bytes > self.executor.max_bytes
            || self.max_event_bytes > self.executor_per_bot.max_bytes
            || self.max_command_bytes > self.command.max_bytes
            || self.max_command_bytes > self.global_command.max_bytes
            || self.command.max_bytes < self.max_command_bytes.saturating_mul(2)
            || self.global_command.max_bytes < self.max_command_bytes.saturating_mul(2)
        {
            return Err(BuildError::InvalidConfig(
                "per-bot, per-item, and global resource limits are inconsistent",
            ));
        }

        if self.executor_shards > MAX_RUNTIME_SHARDS || self.session_shards > MAX_RUNTIME_SHARDS {
            return Err(BuildError::InvalidConfig(
                "runtime shard counts exceed the supported safety limit",
            ));
        }
        let session_command_slots = self
            .session_shards
            .checked_mul(self.session_commands_per_shard)
            .ok_or(BuildError::InvalidConfig(
                "aggregate session command capacity overflows usize",
            ))?;
        if session_command_slots > MAX_SESSION_COMMAND_SLOTS {
            return Err(BuildError::InvalidConfig(
                "aggregate session command capacity exceeds the safety limit",
            ));
        }

        if self.command_in_flight_reserved_high_global >= self.command_in_flight_global
            || self.command_in_flight_reserved_high_per_bot >= self.command_in_flight_per_bot
        {
            return Err(BuildError::InvalidConfig(
                "high-priority command reservations must be smaller than total concurrency",
            ));
        }

        for budget in budgets {
            validate_semaphore_permits(budget.max_items, "queue item capacity")?;
            validate_semaphore_permits(budget.max_bytes, "queue byte capacity")?;
            if budget.max_bytes > u32::MAX as usize {
                return Err(BuildError::InvalidConfig(
                    "queue byte capacities must fit Tokio acquire_many u32 permits",
                ));
            }
        }
        for (value, label) in [
            (
                self.executor_in_flight_per_shard,
                "per-shard executor concurrency",
            ),
            (
                self.executor_in_flight_per_bot,
                "per-bot executor concurrency",
            ),
            (self.command_in_flight_global, "global command concurrency"),
            (
                self.command_in_flight_per_bot,
                "per-bot command concurrency",
            ),
            (self.max_sessions, "session capacity"),
            (
                self.session_commands_per_shard,
                "session shard command capacity",
            ),
        ] {
            validate_semaphore_permits(value, label)?;
        }

        if self.max_frame_events > u32::MAX as usize {
            return Err(BuildError::InvalidConfig(
                "maximum events per frame must fit in u32",
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

fn validate_semaphore_permits(value: usize, _label: &'static str) -> Result<(), BuildError> {
    if value > Semaphore::MAX_PERMITS {
        Err(BuildError::InvalidConfig(
            "configured capacity exceeds Tokio Semaphore::MAX_PERMITS",
        ))
    } else {
        Ok(())
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self::for_profile(RuntimeProfile::Balanced)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semaphore_limits_are_rejected_during_build_validation() {
        let mut config = RuntimeConfig::default();
        config.ingress.max_items = Semaphore::MAX_PERMITS.saturating_add(1);
        assert!(matches!(
            config.validate(),
            Err(BuildError::InvalidConfig(_))
        ));
    }

    #[test]
    fn one_event_must_fit_every_executor_byte_envelope() {
        let mut config = RuntimeConfig::default();
        config.max_event_bytes = config.executor_per_bot.max_bytes.saturating_add(1);
        assert!(matches!(
            config.validate(),
            Err(BuildError::InvalidConfig(_))
        ));
    }

    #[test]
    fn aggregate_session_command_slots_are_bounded() {
        let config = RuntimeConfig {
            session_shards: MAX_RUNTIME_SHARDS,
            session_commands_per_shard: MAX_SESSION_COMMAND_SLOTS,
            ..RuntimeConfig::default()
        };
        assert!(matches!(
            config.validate(),
            Err(BuildError::InvalidConfig(_))
        ));
    }

    #[test]
    fn command_budget_reserves_space_for_one_maximum_priority_item() {
        let mut config = RuntimeConfig::default();
        config.max_command_bytes = config.command.max_bytes;
        assert!(matches!(
            config.validate(),
            Err(BuildError::InvalidConfig(_))
        ));
    }
}
