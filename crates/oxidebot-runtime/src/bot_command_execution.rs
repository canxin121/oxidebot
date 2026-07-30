use super::super::{command_api_error, platform_error, BotServices};
use super::{
    await_before, ApiCommandReply, ApiCommandResult, BotCooldown, CommandCompletion,
    CommandEnvelope, CommandKey, CommandOperation, CommandPriority, GlobalCommandCapacity,
};

use crate::{CommandError, MetricsHandle, PlatformError, PlatformErrorKind, RuntimeMetrics};
use futures_util::FutureExt;
use std::{
    hash::{Hash, Hasher},
    panic::AssertUnwindSafe,
    time::Duration,
};
use tokio::time::Instant;

#[allow(clippy::too_many_arguments)]
pub(super) async fn execute_command(
    key: CommandKey,
    envelope: CommandEnvelope,
    services: BotServices,
    global_capacity: GlobalCommandCapacity,
    cooldown: BotCooldown,
    attempt_timeout: Option<Duration>,
    max_retries: u8,
    retry_base: Duration,
    retry_max: Duration,
    metrics: MetricsHandle,
) -> CommandCompletion {
    let priority = envelope.priority;
    if envelope.is_abandoned() {
        metrics.cancelled_command();
        return CommandCompletion { key, priority };
    }
    metrics.command();
    metrics.command_started();
    let command_started = Instant::now();
    let result = AssertUnwindSafe(execute_with_retry(
        &envelope.operation,
        envelope.sequence,
        envelope.deadline,
        priority,
        &services,
        &global_capacity,
        &cooldown,
        attempt_timeout,
        max_retries,
        retry_base,
        retry_max,
        envelope.reply.as_ref(),
        &metrics,
    ))
    .catch_unwind()
    .await
    .unwrap_or(Err(CommandError::ServicePanicked));
    metrics.command_finished(command_started.elapsed());
    match &result {
        Err(CommandError::Cancelled) => metrics.cancelled_command(),
        Err(_) => metrics.command_error(),
        Ok(_) => {}
    }
    if let Some(reply) = envelope.reply {
        let _ = reply.send(result);
    } else if let Err(error) = result {
        tracing::warn!(%error, "deferred bot command failed");
    }
    drop(envelope._leases);
    CommandCompletion { key, priority }
}

#[allow(clippy::too_many_arguments)]
async fn execute_with_retry(
    operation: &CommandOperation,
    jitter_seed: u64,
    deadline: Option<Instant>,
    priority: CommandPriority,
    services: &BotServices,
    global_capacity: &GlobalCommandCapacity,
    cooldown: &BotCooldown,
    attempt_timeout: Option<Duration>,
    max_retries: u8,
    retry_base: Duration,
    retry_max: Duration,
    reply: Option<&ApiCommandReply>,
    metrics: &RuntimeMetrics,
) -> std::result::Result<ApiCommandResult, CommandError> {
    let idempotent = operation.idempotent(services.capabilities());
    let mut attempt = 0_u8;
    loop {
        if reply.is_some_and(|reply| reply.is_closed()) {
            return Err(CommandError::Cancelled);
        }
        cooldown.wait(deadline).await?;
        if reply.is_some_and(|reply| reply.is_closed()) {
            return Err(CommandError::Cancelled);
        }
        let permit = global_capacity.acquire(priority, deadline).await?;
        permit.touch();

        // Another in-flight command may have published a Retry-After while this
        // command was waiting for global capacity. Do not bypass that cooldown.
        if let Some(until) = cooldown.active_until() {
            drop(permit);
            if deadline.is_some_and(|deadline| until >= deadline) {
                return Err(CommandError::DeadlineExceeded);
            }
            await_before(deadline, tokio::time::sleep_until(until)).await?;
            continue;
        }
        if reply.is_some_and(|reply| reply.is_closed()) {
            drop(permit);
            return Err(CommandError::Cancelled);
        }

        // Calculate the attempt timeout after queue, cooldown, and global-capacity
        // waits, so a stale duration can never extend beyond the total deadline.
        let remaining = deadline.map(|deadline| deadline.saturating_duration_since(Instant::now()));
        if remaining.is_some_and(|remaining| remaining.is_zero()) {
            drop(permit);
            return Err(CommandError::DeadlineExceeded);
        }
        let timeout = match (attempt_timeout, remaining) {
            (Some(attempt_timeout), Some(remaining)) => Some(attempt_timeout.min(remaining)),
            (Some(attempt_timeout), None) => Some(attempt_timeout),
            (None, Some(remaining)) => Some(remaining),
            (None, None) => None,
        };

        let result = if let Some(timeout) = timeout {
            match tokio::time::timeout(timeout, execute_once(operation, services)).await {
                Ok(result) => result,
                Err(_) => Err(CommandError::Platform(PlatformError::new(
                    PlatformErrorKind::Timeout,
                    "platform command attempt timed out",
                ))),
            }
        } else {
            execute_once(operation, services).await
        };
        drop(permit);

        if reply.is_some_and(|reply| reply.is_closed()) {
            return Err(CommandError::Cancelled);
        }

        match result {
            Ok(value) => return Ok(value),
            Err(CommandError::Platform(error))
                if idempotent && error.is_retryable() && attempt < max_retries =>
            {
                metrics.command_retry();
                let exponential = 1_u32.checked_shl(attempt.into()).unwrap_or(u32::MAX);
                let delay = if let Some(retry_after) = error.retry_after {
                    retry_after
                } else {
                    let base = retry_base.saturating_mul(exponential).min(retry_max);
                    let jitter = deterministic_jitter(base, jitter_seed, attempt);
                    base.saturating_add(jitter).min(retry_max)
                };
                if deadline.is_some_and(|deadline| {
                    Instant::now()
                        .checked_add(delay)
                        .is_none_or(|next_attempt| next_attempt >= deadline)
                }) {
                    return Err(CommandError::Platform(error));
                }
                if error.kind == PlatformErrorKind::RateLimited {
                    metrics.rate_limit();
                    cooldown.extend(delay)?;
                } else {
                    await_before(deadline, tokio::time::sleep(delay)).await?;
                }
                attempt = attempt.saturating_add(1);
            }
            Err(CommandError::Platform(error)) => {
                if error.kind == PlatformErrorKind::RateLimited {
                    metrics.rate_limit();
                    if let Some(retry_after) = error.retry_after {
                        let _ = cooldown.extend(retry_after);
                    }
                }
                return Err(CommandError::Platform(error));
            }
            Err(error) => return Err(error),
        }
    }
}

fn deterministic_jitter(base: Duration, seed: u64, attempt: u8) -> Duration {
    if base.is_zero() {
        return Duration::ZERO;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    seed.hash(&mut hasher);
    attempt.hash(&mut hasher);
    let fraction = hasher.finish() % 1_000;
    let max_nanos = base.as_nanos() / 4;
    let nanos = max_nanos.saturating_mul(u128::from(fraction)) / 1_000;
    Duration::from_nanos(u64::try_from(nanos.min(u128::from(u64::MAX))).unwrap_or(u64::MAX))
}

async fn execute_once(
    operation: &CommandOperation,
    services: &BotServices,
) -> std::result::Result<ApiCommandResult, CommandError> {
    match operation {
        CommandOperation::Deliver { target, plan } => services
            .api
            .as_ref()
            .ok_or(CommandError::ApiUnsupported)?
            .send_delivery_plan(target.clone(), plan.clone())
            .await
            .map(ApiCommandResult::Delivery)
            .map_err(command_api_error),
        CommandOperation::EditPublic {
            message,
            replacement,
        } => services
            .api
            .as_ref()
            .ok_or(CommandError::ApiUnsupported)?
            .edit_outgoing_message(message.clone(), replacement.clone())
            .await
            .map(|()| ApiCommandResult::Unit)
            .map_err(|error| CommandError::Platform(platform_error(error))),
        CommandOperation::DeletePublic { message } => {
            let result = services
                .api
                .as_ref()
                .ok_or(CommandError::ApiUnsupported)?
                .delete_message_ref(message.clone())
                .await;
            match result {
                Ok(()) => Ok(ApiCommandResult::Unit),
                Err(error) => {
                    let error = platform_error(error);
                    if services.capabilities().delete_idempotent
                        && error.kind == PlatformErrorKind::NotFound
                    {
                        Ok(ApiCommandResult::Unit)
                    } else {
                        Err(CommandError::Platform(error))
                    }
                }
            }
        }
        CommandOperation::ReactPublic {
            message,
            reaction,
            options,
        } => services
            .api
            .as_ref()
            .ok_or(CommandError::ApiUnsupported)?
            .add_message_reaction(message.clone(), reaction.clone(), options.clone())
            .await
            .map(|()| ApiCommandResult::Unit)
            .map_err(|error| CommandError::Platform(platform_error(error))),
        CommandOperation::AnswerInteraction { handle, response } => services
            .api
            .as_ref()
            .ok_or(CommandError::ApiUnsupported)?
            .answer_interaction(handle.id.clone(), response.clone())
            .await
            .map(|()| ApiCommandResult::Unit)
            .map_err(|error| CommandError::Platform(platform_error(error))),
        CommandOperation::DeferInteraction { handle, visibility } => services
            .api
            .as_ref()
            .ok_or(CommandError::ApiUnsupported)?
            .defer_interaction(handle.clone(), *visibility)
            .await
            .map(|()| ApiCommandResult::Unit)
            .map_err(|error| CommandError::Platform(platform_error(error))),
        CommandOperation::InteractionFollowup { handle, message } => services
            .api
            .as_ref()
            .ok_or(CommandError::ApiUnsupported)?
            .send_interaction_followup(handle.clone(), message.clone())
            .await
            .map(ApiCommandResult::Messages)
            .map_err(|error| CommandError::Platform(platform_error(error))),
        CommandOperation::EditInteraction { handle, message } => services
            .api
            .as_ref()
            .ok_or(CommandError::ApiUnsupported)?
            .edit_interaction_response(handle.clone(), message.clone())
            .await
            .map(|()| ApiCommandResult::Unit)
            .map_err(|error| CommandError::Platform(platform_error(error))),
        CommandOperation::PublishCommands { definitions } => services
            .api
            .as_ref()
            .ok_or(CommandError::ApiUnsupported)?
            .set_command_definitions(definitions.clone())
            .await
            .map(|()| ApiCommandResult::Unit)
            .map_err(|error| CommandError::Platform(platform_error(error))),
    }
}
