//! Scoped fluent configuration façades for one runtime configuration.

use super::*;

/// Named ingress section used by [`RuntimeConfig::configure_ingress`].
pub struct IngressConfig<'a> {
    pub(in crate::config) config: &'a mut RuntimeConfig,
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
    pub(in crate::config) config: &'a mut RuntimeConfig,
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
    pub(in crate::config) config: &'a mut RuntimeConfig,
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
    pub(in crate::config) config: &'a mut RuntimeConfig,
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
    pub(in crate::config) config: &'a mut RuntimeConfig,
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
