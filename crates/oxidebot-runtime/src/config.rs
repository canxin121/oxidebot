use crate::{dedupe::MIN_DEDUPE_BYTES_FOR_MAX_ID, BuildError, OverloadPolicy, QueueBudget};
use oxidebot_core::event::kernel::MessageExecutionPartition;
use std::time::Duration;
use tokio::sync::Semaphore;

/// Upper bound on worker-shard fan-out created by one runtime.
const MAX_RUNTIME_SHARDS: usize = 1_024;
/// Upper bound on aggregate session-command channel slots.
const MAX_SESSION_COMMAND_SLOTS: usize = 1_048_576;
/// A single handler outcome cannot fan out an unbounded number of commands.
const MAX_HANDLER_REPLIES_HARD_LIMIT: usize = 4_096;

/// Predefined resource envelopes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RuntimeProfile {
    /// Conservative queues and concurrency for small bots.
    Eco,
    /// General-purpose default resource envelope.
    #[default]
    Balanced,
    /// Larger resource envelope for high-volume bots.
    Throughput,
}

/// Complete runtime resource and resilience configuration.
#[derive(Clone, Copy, Debug)]
pub struct RuntimeConfig {
    /// Process-wide inbound frame admission budget.
    pub ingress: QueueBudget,
    /// Per-bot ingress share preventing one transport from occupying the
    /// entire process queue while other bots are ready.
    pub ingress_per_bot: QueueBudget,
    /// Maximum retained bytes for one decoded platform frame.
    pub max_frame_bytes: usize,
    /// Maximum events one platform frame may expand into.
    pub max_frame_events: usize,
    /// Maximum incremental retained bytes for one event.
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
    /// Executor response when its queue is overloaded.
    pub executor_overload: OverloadPolicy,
    /// Platform command response when its queue is overloaded.
    pub command_overload: OverloadPolicy,
    /// Number of independent executor worker shards.
    pub executor_shards: usize,
    /// Concurrent handler executions allowed in each shard.
    pub executor_in_flight_per_shard: usize,
    /// Concurrent handler executions allowed for each bot.
    pub executor_in_flight_per_bot: usize,
    /// Process-wide cap on platform calls actively inside adapter services.
    pub command_in_flight_global: usize,
    /// Global capacity unavailable to normal commands and reserved for high-priority interactions.
    pub command_in_flight_reserved_high_global: usize,
    /// Per-bot cap on commands admitted into the running set.
    pub command_in_flight_per_bot: usize,
    /// Per-bot running slots unavailable to normal commands and reserved for interactions.
    pub command_in_flight_reserved_high_per_bot: usize,
    /// Number of high-priority commands served before normal fairness resumes.
    pub command_high_priority_burst: usize,
    /// Number of dialogue/session registry shards.
    pub session_shards: usize,
    /// Session command channel capacity in each shard.
    pub session_commands_per_shard: usize,
    /// Maximum active dialogue sessions.
    pub max_sessions: usize,
    /// Maximum dedupe entries retained.
    pub dedupe_capacity: usize,
    /// Maximum bytes retained by the dedupe store.
    pub dedupe_max_bytes: usize,
    /// Lifetime of a dedupe record.
    pub dedupe_ttl: Duration,
    /// Key used to serialize message handler execution.
    pub message_execution_partition: MessageExecutionPartition,
    /// Maximum deferred replies accepted from one handler outcome.
    pub max_handler_replies: usize,
    /// Optional deadline for one handler invocation.
    pub handler_timeout: Option<Duration>,
    /// Timeout for one platform command attempt.
    pub command_attempt_timeout: Option<Duration>,
    /// Total command deadline including queueing and retry backoff.
    pub command_total_timeout: Option<Duration>,
    /// Maximum retry count after the initial platform command attempt.
    pub command_max_retries: u8,
    /// Initial retry backoff duration.
    pub command_retry_base: Duration,
    /// Maximum retry backoff duration.
    pub command_retry_max: Duration,
    /// Grace period used while draining runtime tasks during shutdown.
    pub shutdown_grace: Duration,
}

impl RuntimeConfig {
    /// Resource-conservative defaults suitable for small personal bots.
    #[must_use]
    pub fn eco() -> Self {
        Self::for_profile(RuntimeProfile::Eco)
    }

    /// General-purpose defaults suitable for most applications.
    #[must_use]
    pub fn balanced() -> Self {
        Self::for_profile(RuntimeProfile::Balanced)
    }

    /// Larger queues and concurrency for high-volume applications.
    #[must_use]
    pub fn throughput() -> Self {
        Self::for_profile(RuntimeProfile::Throughput)
    }

    /// Builds defaults for a named resource profile.
    #[must_use]
    pub fn for_profile(profile: RuntimeProfile) -> Self {
        let (items, bytes, shards, in_flight, frame_events, handler_replies, handler_timeout): (
            usize,
            usize,
            usize,
            usize,
            usize,
            usize,
            Duration,
        ) = match profile {
            RuntimeProfile::Eco => (64, 2 * 1024 * 1024, 1, 8, 32, 8, Duration::from_secs(30)),
            RuntimeProfile::Balanced => (
                512,
                16 * 1024 * 1024,
                4,
                32,
                128,
                32,
                Duration::from_secs(60),
            ),
            RuntimeProfile::Throughput => (
                4096,
                128 * 1024 * 1024,
                16,
                128,
                512,
                64,
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
            max_handler_replies: handler_replies,
            handler_timeout: Some(handler_timeout),
            command_attempt_timeout: Some(Duration::from_secs(15)),
            command_total_timeout: Some(Duration::from_secs(30)),
            command_max_retries: 3,
            command_retry_base: Duration::from_millis(25),
            command_retry_max: Duration::from_secs(5),
            shutdown_grace: Duration::from_secs(5),
        }
    }

    /// Configures inbound frame and event admission without exposing unrelated
    /// executor, command, or session settings at the call site.
    #[must_use]
    pub fn configure_ingress(mut self, configure: impl FnOnce(&mut IngressConfig<'_>)) -> Self {
        configure(&mut IngressConfig { config: &mut self });
        self
    }

    /// Configures handler execution and propagation limits.
    #[must_use]
    pub fn configure_execution(mut self, configure: impl FnOnce(&mut ExecutionConfig<'_>)) -> Self {
        configure(&mut ExecutionConfig { config: &mut self });
        self
    }

    /// Configures outbound platform API queues, retries, and deadlines.
    #[must_use]
    pub fn configure_commands(
        mut self,
        configure: impl FnOnce(&mut CommandRuntimeConfig<'_>),
    ) -> Self {
        configure(&mut CommandRuntimeConfig { config: &mut self });
        self
    }

    /// Configures bounded dialogue/session storage and routing.
    #[must_use]
    pub fn configure_sessions(
        mut self,
        configure: impl FnOnce(&mut SessionRuntimeConfig<'_>),
    ) -> Self {
        configure(&mut SessionRuntimeConfig { config: &mut self });
        self
    }

    /// Configures event deduplication and graceful shutdown.
    #[must_use]
    pub fn configure_resilience(
        mut self,
        configure: impl FnOnce(&mut ResilienceConfig<'_>),
    ) -> Self {
        configure(&mut ResilienceConfig { config: &mut self });
        self
    }

    /// Validates all cross-field safety invariants before the application is
    /// built. [`crate::OxideBot::build`] invokes this automatically.
    pub fn validate(&self) -> Result<(), BuildError> {
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
            || self.dedupe_max_bytes < MIN_DEDUPE_BYTES_FOR_MAX_ID
            || self.dedupe_ttl.is_zero()
            || self.max_handler_replies == 0
            || self.max_handler_replies > MAX_HANDLER_REPLIES_HARD_LIMIT
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
            || self.max_handler_replies > self.command.max_items
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

/// Named ingress section used by [`RuntimeConfig::configure_ingress`].
pub struct IngressConfig<'a> {
    config: &'a mut RuntimeConfig,
}

impl IngressConfig<'_> {
    /// Sets process-wide ingress item and byte budgets.
    pub fn global(&mut self, max_items: usize, max_bytes: usize) -> &mut Self {
        self.config.ingress = QueueBudget::new(max_items, max_bytes);
        self
    }

    /// Sets per-bot ingress item and byte budgets.
    pub fn per_bot(&mut self, max_items: usize, max_bytes: usize) -> &mut Self {
        self.config.ingress_per_bot = QueueBudget::new(max_items, max_bytes);
        self
    }

    /// Sets maximum retained bytes for one decoded frame.
    pub fn max_frame_bytes(&mut self, bytes: usize) -> &mut Self {
        self.config.max_frame_bytes = bytes;
        self
    }

    /// Sets maximum events emitted from one frame.
    pub fn max_frame_events(&mut self, events: usize) -> &mut Self {
        self.config.max_frame_events = events;
        self
    }

    /// Sets maximum retained bytes for one event.
    pub fn max_event_bytes(&mut self, bytes: usize) -> &mut Self {
        self.config.max_event_bytes = bytes;
        self
    }
}

/// Named execution section used by [`RuntimeConfig::configure_execution`].
pub struct ExecutionConfig<'a> {
    config: &'a mut RuntimeConfig,
}

impl ExecutionConfig<'_> {
    /// Sets process-wide executor item and byte budgets.
    pub fn global(&mut self, max_items: usize, max_bytes: usize) -> &mut Self {
        self.config.executor = QueueBudget::new(max_items, max_bytes);
        self
    }

    /// Sets per-bot executor item and byte budgets.
    pub fn per_bot(&mut self, max_items: usize, max_bytes: usize) -> &mut Self {
        self.config.executor_per_bot = QueueBudget::new(max_items, max_bytes);
        self
    }

    /// Sets queue overload behavior for handler execution.
    pub fn overload(&mut self, policy: OverloadPolicy) -> &mut Self {
        self.config.executor_overload = policy;
        self
    }

    /// Sets executor shard count.
    pub fn shards(&mut self, value: usize) -> &mut Self {
        self.config.executor_shards = value;
        self
    }

    /// Sets per-shard concurrent handler limit.
    pub fn in_flight_per_shard(&mut self, value: usize) -> &mut Self {
        self.config.executor_in_flight_per_shard = value;
        self
    }

    /// Sets per-bot concurrent handler limit.
    pub fn in_flight_per_bot(&mut self, value: usize) -> &mut Self {
        self.config.executor_in_flight_per_bot = value;
        self
    }

    /// Sets maximum deferred replies accepted from one handler outcome.
    pub fn max_replies(&mut self, value: usize) -> &mut Self {
        self.config.max_handler_replies = value;
        self
    }

    /// Sets an optional handler deadline.
    pub fn handler_timeout(&mut self, value: Option<Duration>) -> &mut Self {
        self.config.handler_timeout = value;
        self
    }
}

/// Named outbound-command section used by
/// [`RuntimeConfig::configure_commands`].
pub struct CommandRuntimeConfig<'a> {
    config: &'a mut RuntimeConfig,
}

impl CommandRuntimeConfig<'_> {
    /// Sets per-bot command queue item and byte budgets.
    pub fn per_bot_queue(&mut self, max_items: usize, max_bytes: usize) -> &mut Self {
        self.config.command = QueueBudget::new(max_items, max_bytes);
        self
    }

    /// Sets process-wide command queue item and byte budgets.
    pub fn global_queue(&mut self, max_items: usize, max_bytes: usize) -> &mut Self {
        self.config.global_command = QueueBudget::new(max_items, max_bytes);
        self
    }

    /// Sets maximum retained bytes for one command operation.
    pub fn max_payload_bytes(&mut self, bytes: usize) -> &mut Self {
        self.config.max_command_bytes = bytes;
        self
    }

    /// Sets command queue overload behavior.
    pub fn overload(&mut self, policy: OverloadPolicy) -> &mut Self {
        self.config.command_overload = policy;
        self
    }

    /// Sets process-wide concurrent platform command limit.
    pub fn global_in_flight(&mut self, value: usize) -> &mut Self {
        self.config.command_in_flight_global = value;
        self
    }

    /// Reserves process-wide command slots for high-priority interactions.
    pub fn reserved_high_global(&mut self, value: usize) -> &mut Self {
        self.config.command_in_flight_reserved_high_global = value;
        self
    }

    /// Sets concurrent platform command limit per bot.
    pub fn per_bot_in_flight(&mut self, value: usize) -> &mut Self {
        self.config.command_in_flight_per_bot = value;
        self
    }

    /// Reserves per-bot command slots for high-priority interactions.
    pub fn reserved_high_per_bot(&mut self, value: usize) -> &mut Self {
        self.config.command_in_flight_reserved_high_per_bot = value;
        self
    }

    /// Sets high-priority command burst size before normal fairness resumes.
    pub fn high_priority_burst(&mut self, value: usize) -> &mut Self {
        self.config.command_high_priority_burst = value;
        self
    }

    /// Sets retry count after the initial command attempt.
    pub fn retries(&mut self, value: u8) -> &mut Self {
        self.config.command_max_retries = value;
        self
    }

    /// Sets an optional deadline for one command attempt.
    pub fn attempt_timeout(&mut self, value: Option<Duration>) -> &mut Self {
        self.config.command_attempt_timeout = value;
        self
    }

    /// Sets an optional total command deadline including retries.
    pub fn total_timeout(&mut self, value: Option<Duration>) -> &mut Self {
        self.config.command_total_timeout = value;
        self
    }

    /// Sets initial and maximum command retry backoff durations.
    pub fn retry_backoff(&mut self, base: Duration, maximum: Duration) -> &mut Self {
        self.config.command_retry_base = base;
        self.config.command_retry_max = maximum;
        self
    }
}

/// Named dialogue/session section used by
/// [`RuntimeConfig::configure_sessions`].
pub struct SessionRuntimeConfig<'a> {
    config: &'a mut RuntimeConfig,
}

impl SessionRuntimeConfig<'_> {
    /// Sets dialogue/session registry shard count.
    pub fn shards(&mut self, value: usize) -> &mut Self {
        self.config.session_shards = value;
        self
    }

    /// Sets session command channel capacity in each shard.
    pub fn commands_per_shard(&mut self, value: usize) -> &mut Self {
        self.config.session_commands_per_shard = value;
        self
    }

    /// Sets maximum concurrent dialogue sessions.
    pub fn max_sessions(&mut self, value: usize) -> &mut Self {
        self.config.max_sessions = value;
        self
    }

    /// Sets the execution key used to serialize session messages.
    pub fn message_partition(&mut self, value: MessageExecutionPartition) -> &mut Self {
        self.config.message_execution_partition = value;
        self
    }
}

/// Named resilience section used by
/// [`RuntimeConfig::configure_resilience`].
pub struct ResilienceConfig<'a> {
    config: &'a mut RuntimeConfig,
}

impl ResilienceConfig<'_> {
    /// Sets dedupe record capacity, byte budget, and lifetime.
    pub fn dedupe(&mut self, capacity: usize, max_bytes: usize, ttl: Duration) -> &mut Self {
        self.config.dedupe_capacity = capacity;
        self.config.dedupe_max_bytes = max_bytes;
        self.config.dedupe_ttl = ttl;
        self
    }

    /// Sets the shutdown task-drain grace period.
    pub fn shutdown_grace(&mut self, value: Duration) -> &mut Self {
        self.config.shutdown_grace = value;
        self
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
