use super::{app_adapter::register_bots, app_dispatch::dispatch_loop, Application, RunMode};

use crate::{
    adapter::{AdapterContext, AdapterLimits, AdapterMode, EventSink},
    executor::ExecutorHandle,
    router::{CompiledRouter, RouterLimits, RouterRuntime},
    session::SessionRegistry,
    Result, RuntimeError, ServiceContext, ServiceError, ShutdownSignal,
};
use std::{future::Future, sync::Arc, time::Instant};
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;

pub(super) async fn run_application<S, F>(
    app: Application<S>,
    shutdown_signal: F,
    mode: RunMode,
) -> Result<()>
where
    S: Send + Sync + 'static,
    F: Future<Output = ()>,
{
    let Application {
        state,
        config,
        adapters,
        handlers,
        filters,
        services,
        command_catalog,
        metrics,
        authoring,
    } = app;

    let (registered, command_workers, bot_directory) =
        register_bots(adapters, &config, Arc::clone(&metrics))?;
    let bot_count = bot_directory.len();
    let cancellation = CancellationToken::new();

    // API calls are serialized by the per-bot command workers. Start them
    // before publishing the shared command IR, otherwise publication would
    // enqueue work and wait on workers that are not running yet.
    let mut command_tasks = JoinSet::new();
    for worker in command_workers {
        command_tasks.spawn(worker.run());
    }
    authoring.attach_bots(bot_directory.clone());
    authoring
        .registry
        .attach_publication(bot_directory.clone(), command_catalog.clone());
    if let Err(error) = authoring.registry.refresh_publication().await {
        cancellation.cancel();
        authoring.registry.detach_publication();
        authoring.detach_bots();
        command_tasks.abort_all();
        while command_tasks.join_next().await.is_some() {}
        return Err(RuntimeError::Service(ServiceError::new(format!(
            "could not publish startup command definitions: {error}"
        ))));
    }

    let (sessions, session_workers) = SessionRegistry::new(
        config.session_shards,
        config.session_commands_per_shard,
        config.max_sessions,
        Arc::clone(&metrics),
    );
    let router = Arc::new(CompiledRouter::compile(
        handlers,
        filters,
        RouterRuntime {
            state: Arc::clone(&state),
            sessions: sessions.clone(),
            shutdown: ShutdownSignal::new(cancellation.child_token()),
            metrics: Arc::clone(&metrics),
            authoring: Arc::clone(&authoring),
        },
        RouterLimits {
            handler_timeout: config.handler_timeout,
            max_handler_replies: config.max_handler_replies,
        },
    ));
    let mut session_tasks = JoinSet::new();
    for worker in session_workers {
        session_tasks.spawn(worker.run());
    }

    let (executor, executor_workers) = ExecutorHandle::new(
        config.executor_shards,
        bot_count,
        config.executor,
        config.executor_per_bot,
        config.executor_in_flight_per_shard,
        config.executor_in_flight_per_bot,
        config.executor_overload,
        config.message_execution_partition,
        Arc::clone(&router),
        Arc::clone(&metrics),
    );
    let mut executor_tasks = JoinSet::new();
    for worker in executor_workers {
        executor_tasks.spawn(worker.run());
    }
    let (event_sink, ingress_receiver) = EventSink::channel(config.ingress);
    let mut dispatcher = tokio::spawn(dispatch_loop(
        ingress_receiver,
        bot_directory.clone(),
        sessions.clone(),
        executor,
        config.executor.max_items,
        config.dedupe_capacity,
        config.dedupe_max_bytes,
        config.dedupe_ttl,
        Arc::clone(&metrics),
    ));

    let mut service_tasks = JoinSet::new();
    for service in services {
        let context = ServiceContext::new(
            Arc::clone(&state),
            bot_directory.clone(),
            ShutdownSignal::new(cancellation.child_token()),
        );
        service_tasks.spawn(async move { service.run(context).await });
    }

    let mut adapter_tasks = JoinSet::new();
    for registered in registered {
        let adapter_mode = registered.mode;
        let interest = router.interest_for(registered.identity);
        let context = AdapterContext::new(
            registered.slot,
            registered.platform,
            event_sink.clone(),
            AdapterLimits {
                ingress: config.ingress_per_bot,
                max_frame_bytes: config.max_frame_bytes,
                max_frame_events: config.max_frame_events,
                max_event_bytes: config.max_event_bytes,
            },
            interest,
            cancellation.child_token(),
            Arc::clone(&metrics),
        );
        adapter_tasks.spawn(async move { (adapter_mode, registered.adapter.run(context).await) });
    }
    drop(router);
    // The supervisor retains one ingress sender until every adapter has drained.

    tokio::pin!(shutdown_signal);
    let mut adapters_remaining = adapter_tasks.len();
    let mut fatal_error = None;
    let mut dispatcher_result = None;

    while adapters_remaining > 0 && fatal_error.is_none() {
        tokio::select! {
            _ = &mut shutdown_signal => break,
            completed = adapter_tasks.join_next(), if !adapter_tasks.is_empty() => {
                adapters_remaining = adapters_remaining.saturating_sub(1);
                match completed {
                    Some(Ok((adapter_mode, Ok(())))) => {
                        if adapter_mode == AdapterMode::Persistent && !cancellation.is_cancelled() {
                            fatal_error = Some(RuntimeError::Channel(
                                "a persistent adapter exited without cancellation",
                            ));
                        }
                    }
                    Some(Ok((_, Err(error)))) => fatal_error = Some(RuntimeError::Adapter(error)),
                    Some(Err(error)) => fatal_error = Some(RuntimeError::Join(error.to_string())),
                    None => adapters_remaining = 0,
                }
            }
            completed = service_tasks.join_next(), if !service_tasks.is_empty() => {
                fatal_error = Some(match completed {
                    Some(Ok(Ok(()))) => RuntimeError::Channel("a supervised service exited early"),
                    Some(Ok(Err(error))) => RuntimeError::Service(error),
                    Some(Err(error)) => RuntimeError::Join(error.to_string()),
                    None => RuntimeError::Channel("all supervised services exited early"),
                });
            }
            completed = &mut dispatcher => {
                let result = match completed {
                    Ok(result) => result,
                    Err(error) => Err(RuntimeError::Join(error.to_string())),
                };
                if let Err(error) = result {
                    fatal_error = Some(error);
                } else if adapters_remaining > 0 && fatal_error.is_none() {
                    fatal_error = Some(RuntimeError::Channel(
                        "event dispatcher exited while adapters are still running",
                    ));
                }
                dispatcher_result = Some(Ok(()));
                break;
            }
            completed = executor_tasks.join_next(), if !executor_tasks.is_empty() => {
                fatal_error = Some(match completed {
                    Some(Ok(())) => RuntimeError::Channel("an executor shard exited early"),
                    Some(Err(error)) => RuntimeError::Join(error.to_string()),
                    None => RuntimeError::Channel("all executor shards exited early"),
                });
            }
            completed = session_tasks.join_next(), if !session_tasks.is_empty() => {
                fatal_error = Some(match completed {
                    Some(Ok(())) => RuntimeError::Channel("a session shard exited early"),
                    Some(Err(error)) => RuntimeError::Join(error.to_string()),
                    None => RuntimeError::Channel("all session shards exited early"),
                });
            }
            completed = command_tasks.join_next(), if !command_tasks.is_empty() => {
                fatal_error = Some(match completed {
                    Some(Ok(())) => RuntimeError::Channel("a bot command scheduler exited early"),
                    Some(Err(error)) => RuntimeError::Join(error.to_string()),
                    None => RuntimeError::Channel("all bot command schedulers exited early"),
                });
            }
        }
    }

    if matches!(mode, RunMode::Finite) && adapters_remaining > 0 && fatal_error.is_none() {
        fatal_error = Some(RuntimeError::Channel(
            "finite run stopped before adapters completed",
        ));
    }

    cancellation.cancel();
    let shutdown_deadline = Instant::now()
        .checked_add(config.shutdown_grace)
        .expect("runtime configuration validated the shutdown deadline");
    record_first(
        &mut fatal_error,
        drain_adapters(&mut adapter_tasks, remaining(shutdown_deadline)).await,
    );
    drop(event_sink);
    record_first(
        &mut fatal_error,
        drain_services(&mut service_tasks, remaining(shutdown_deadline)).await,
    );

    if dispatcher_result.is_none() {
        dispatcher_result =
            Some(wait_dispatcher(&mut dispatcher, remaining(shutdown_deadline)).await);
    }
    if let Some(result) = dispatcher_result {
        record_first(&mut fatal_error, result.err());
    }

    record_first(
        &mut fatal_error,
        drain_unit_tasks(
            &mut executor_tasks,
            remaining(shutdown_deadline),
            "executor shards",
        )
        .await,
    );

    drop(sessions);
    record_first(
        &mut fatal_error,
        drain_unit_tasks(
            &mut session_tasks,
            remaining(shutdown_deadline),
            "session shards",
        )
        .await,
    );

    // Publication keeps bot handles so runtime command changes can republish
    // native definitions. Release that attachment before waiting for the
    // per-bot command channels to close.
    authoring.registry.detach_publication();
    authoring.detach_bots();
    drop(bot_directory);
    record_first(
        &mut fatal_error,
        drain_unit_tasks(
            &mut command_tasks,
            remaining(shutdown_deadline),
            "bot command schedulers",
        )
        .await,
    );

    match fatal_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn remaining(deadline: Instant) -> std::time::Duration {
    deadline.saturating_duration_since(Instant::now())
}

async fn wait_dispatcher(
    dispatcher: &mut JoinHandle<Result<()>>,
    grace: std::time::Duration,
) -> Result<()> {
    if grace.is_zero() {
        dispatcher.abort();
        let _ = dispatcher.await;
        return Err(RuntimeError::ShutdownTimeout("event dispatcher"));
    }
    match tokio::time::timeout(grace, &mut *dispatcher).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => Err(RuntimeError::Join(error.to_string())),
        Err(_) => {
            dispatcher.abort();
            let _ = dispatcher.await;
            Err(RuntimeError::ShutdownTimeout("event dispatcher"))
        }
    }
}

async fn drain_adapters(
    tasks: &mut JoinSet<(AdapterMode, std::result::Result<(), crate::AdapterError>)>,
    grace: std::time::Duration,
) -> Option<RuntimeError> {
    drain_join_set(tasks, grace, "adapters", |completed| match completed {
        Ok((_, Ok(()))) => None,
        Ok((_, Err(error))) if error.is_cancelled() => None,
        Ok((_, Err(error))) => Some(RuntimeError::Adapter(error)),
        Err(error) => Some(RuntimeError::Join(error.to_string())),
    })
    .await
}

async fn drain_services(
    tasks: &mut JoinSet<std::result::Result<(), crate::ServiceError>>,
    grace: std::time::Duration,
) -> Option<RuntimeError> {
    drain_join_set(tasks, grace, "services", |completed| match completed {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(RuntimeError::Service(error)),
        Err(error) => Some(RuntimeError::Join(error.to_string())),
    })
    .await
}

async fn drain_unit_tasks(
    tasks: &mut JoinSet<()>,
    grace: std::time::Duration,
    phase: &'static str,
) -> Option<RuntimeError> {
    drain_join_set(tasks, grace, phase, |completed| {
        completed
            .err()
            .map(|error| RuntimeError::Join(error.to_string()))
    })
    .await
}

async fn drain_join_set<T, F>(
    tasks: &mut JoinSet<T>,
    grace: std::time::Duration,
    phase: &'static str,
    mut map: F,
) -> Option<RuntimeError>
where
    T: 'static,
    F: FnMut(std::result::Result<T, tokio::task::JoinError>) -> Option<RuntimeError>,
{
    if grace.is_zero() {
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
        return Some(RuntimeError::ShutdownTimeout(phase));
    }
    let drain = async {
        let mut first = None;
        while let Some(completed) = tasks.join_next().await {
            record_first(&mut first, map(completed));
        }
        first
    };
    match tokio::time::timeout(grace, drain).await {
        Ok(error) => error,
        Err(_) => {
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
            Some(RuntimeError::ShutdownTimeout(phase))
        }
    }
}

fn record_first(target: &mut Option<RuntimeError>, candidate: Option<RuntimeError>) {
    if target.is_none() {
        *target = candidate;
    }
}
