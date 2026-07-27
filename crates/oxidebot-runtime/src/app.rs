use crate::{
    adapter::{AdapterContext, AdapterMode, EventSink, IngressBatch},
    bot::CommandWorker,
    budget::QueueLimiter,
    dedupe::DedupeCache,
    executor::{ExecutorHandle, ExecutorSubmit},
    handler::{erase_handler, ErasedHandler, RouteSpec},
    router::CompiledRouter,
    session::{SessionDelivery, SessionRegistry},
    Adapter, BotDirectory, BuildError, Filter, Handler, MetricsHandle, Result, RuntimeConfig,
    RuntimeError, RuntimeMetrics, RuntimeProfile, Service, ServiceContext, ShutdownSignal,
};
use oxidebot_core::{BotIdentity, BotSlot, MAX_ROUTE_KEY_BYTES};
use std::{
    collections::HashSet,
    future::{pending, Future},
    sync::Arc,
    time::Instant,
};
use tokio::{
    sync::{mpsc, Semaphore},
    task::{JoinHandle, JoinSet},
};
use tokio_util::sync::CancellationToken;

/// Chainable OxideBot application builder.
pub struct OxideBot<S = ()>
where
    S: Send + Sync + 'static,
{
    state: Arc<S>,
    config: RuntimeConfig,
    adapters: Vec<Box<dyn Adapter>>,
    handlers: Vec<Arc<dyn ErasedHandler<S>>>,
    filters: Vec<Arc<dyn Filter<S>>>,
    services: Vec<Arc<dyn Service<S>>>,
    metrics: MetricsHandle,
}

impl OxideBot<()> {
    #[must_use]
    pub fn new() -> Self {
        Self::with_state(())
    }
}

impl Default for OxideBot<()> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> OxideBot<S>
where
    S: Send + Sync + 'static,
{
    #[must_use]
    pub fn with_state(state: S) -> Self {
        Self {
            state: Arc::new(state),
            config: RuntimeConfig::default(),
            adapters: Vec::new(),
            handlers: Vec::new(),
            filters: Vec::new(),
            services: Vec::new(),
            metrics: Arc::new(RuntimeMetrics::default()),
        }
    }

    #[must_use]
    pub fn profile(mut self, profile: RuntimeProfile) -> Self {
        self.config = RuntimeConfig::for_profile(profile);
        self
    }

    #[must_use]
    pub fn config(mut self, config: RuntimeConfig) -> Self {
        self.config = config;
        self
    }

    #[must_use]
    pub fn metrics(mut self, metrics: MetricsHandle) -> Self {
        self.metrics = metrics;
        self
    }

    #[must_use]
    pub fn metrics_handle(&self) -> MetricsHandle {
        Arc::clone(&self.metrics)
    }

    #[must_use]
    pub fn bot<A>(mut self, adapter: A) -> Self
    where
        A: Adapter,
    {
        self.adapters.push(Box::new(adapter));
        self
    }

    #[must_use]
    pub fn handler<H>(mut self, handler: H) -> Self
    where
        H: Handler<S>,
    {
        self.handlers.push(erase_handler(handler));
        self
    }

    #[must_use]
    pub fn filter<F>(mut self, filter: F) -> Self
    where
        F: Filter<S>,
    {
        self.filters.push(Arc::new(filter));
        self
    }

    #[must_use]
    pub fn service<T>(mut self, service: T) -> Self
    where
        T: Service<S>,
    {
        self.services.push(Arc::new(service));
        self
    }

    pub fn build(self) -> std::result::Result<Application<S>, BuildError> {
        self.config.validate()?;
        validate_adapters(&self.adapters)?;
        if self.handlers.len() > u32::MAX as usize {
            return Err(BuildError::InvalidConfig("too many route handlers"));
        }
        validate_routes(&self.handlers)?;
        Ok(Application { inner: self })
    }

    /// Runs until Ctrl-C (with the default `signal` feature), fatal failure, or
    /// natural completion of every finite adapter.
    pub async fn run(self) -> Result<()> {
        self.build()?.run().await
    }

    /// Runs a finite replay/import application to completion.
    pub async fn run_to_completion(self) -> Result<()> {
        self.build()?.run_to_completion().await
    }
}

/// Validated application ready to run.
pub struct Application<S = ()>
where
    S: Send + Sync + 'static,
{
    inner: OxideBot<S>,
}

impl<S> Application<S>
where
    S: Send + Sync + 'static,
{
    #[must_use]
    pub fn metrics(&self) -> MetricsHandle {
        Arc::clone(&self.inner.metrics)
    }

    #[cfg(feature = "signal")]
    pub async fn run(self) -> Result<()> {
        self.run_internal(
            async {
                if let Err(error) = tokio::signal::ctrl_c().await {
                    tracing::error!(%error, "failed to listen for Ctrl-C");
                }
            },
            RunMode::Normal,
        )
        .await
    }

    #[cfg(not(feature = "signal"))]
    pub async fn run(self) -> Result<()> {
        self.run_internal(pending(), RunMode::Normal).await
    }

    pub async fn run_to_completion(self) -> Result<()> {
        if self
            .inner
            .adapters
            .iter()
            .any(|adapter| adapter.mode() != AdapterMode::Finite)
        {
            return Err(BuildError::PersistentAdapterInFiniteRun.into());
        }
        self.run_internal(pending(), RunMode::Finite).await
    }

    pub async fn run_until<F>(self, shutdown_signal: F) -> Result<()>
    where
        F: Future<Output = ()>,
    {
        self.run_internal(shutdown_signal, RunMode::Normal).await
    }

    async fn run_internal<F>(self, shutdown_signal: F, mode: RunMode) -> Result<()>
    where
        F: Future<Output = ()>,
    {
        run_application(self.inner, shutdown_signal, mode).await
    }
}

#[derive(Clone, Copy)]
enum RunMode {
    Normal,
    Finite,
}

fn validate_routes<S>(handlers: &[Arc<dyn ErasedHandler<S>>]) -> std::result::Result<(), BuildError>
where
    S: Send + Sync + 'static,
{
    for handler in handlers {
        let spec = handler.route_spec();
        let event_kind = handler.event_kind();
        let expected_kind = match &spec {
            RouteSpec::Generic(kind) => *kind,
            RouteSpec::Command(_) => oxidebot_core::EventKind::MessageCreated,
            RouteSpec::Interaction(_) => oxidebot_core::EventKind::Interaction,
            RouteSpec::Native(_) => oxidebot_core::EventKind::Native,
        };
        if event_kind != expected_kind {
            return Err(BuildError::InvalidRoute(
                "matcher event view differs from its compiled route category".into(),
            ));
        }
        let key = match &spec {
            RouteSpec::Generic(_) => continue,
            RouteSpec::Command(value)
            | RouteSpec::Interaction(value)
            | RouteSpec::Native(value) => value,
        };
        if key.is_empty() || key.len() > MAX_ROUTE_KEY_BYTES {
            return Err(BuildError::InvalidRoute(format!(
                "route key must contain 1..={MAX_ROUTE_KEY_BYTES} bytes"
            )));
        }
    }
    Ok(())
}

fn validate_adapters(adapters: &[Box<dyn Adapter>]) -> std::result::Result<(), BuildError> {
    if adapters.is_empty() {
        return Err(BuildError::NoAdapters);
    }
    let mut identities = HashSet::<BotIdentity>::with_capacity(adapters.len());
    for adapter in adapters {
        let descriptor = adapter.descriptor();
        descriptor
            .validate()
            .map_err(|error| BuildError::InvalidBot(error.to_string()))?;
        if descriptor
            .display_name
            .as_ref()
            .is_some_and(|value| value.len() > 4 * 1024)
        {
            return Err(BuildError::InvalidBot(
                "display name exceeds 4096 bytes".into(),
            ));
        }
        let identity = descriptor.identity();
        if !identities.insert(identity.clone()) {
            return Err(BuildError::DuplicateBot(format!(
                "{}:{}",
                identity.platform, identity.bot
            )));
        }
    }
    Ok(())
}

struct RegisteredAdapter {
    slot: BotSlot,
    platform: oxidebot_core::PlatformId,
    mode: AdapterMode,
    adapter: Box<dyn Adapter>,
}

fn register_bots(
    adapters: Vec<Box<dyn Adapter>>,
    config: &RuntimeConfig,
    metrics: MetricsHandle,
) -> std::result::Result<(Vec<RegisteredAdapter>, Vec<CommandWorker>, BotDirectory), BuildError> {
    let mut registered = Vec::with_capacity(adapters.len());
    let mut workers = Vec::with_capacity(adapters.len());
    let mut handles = Vec::with_capacity(adapters.len());
    let global_command_limiter = QueueLimiter::new(config.global_command);
    let global_command_in_flight = Arc::new(Semaphore::new(config.command_in_flight_global));

    for (index, adapter) in adapters.into_iter().enumerate() {
        let slot = BotSlot(u32::try_from(index).map_err(|_| BuildError::TooManyBots)?);
        let descriptor = adapter.descriptor();
        let platform = descriptor.platform.clone();
        let mode = adapter.mode();
        let services = adapter.services();
        let (handle, worker) = CommandWorker::build(
            slot,
            descriptor,
            services,
            global_command_limiter.clone(),
            Arc::clone(&global_command_in_flight),
            config.command,
            config.command_overload,
            config.command_in_flight_per_bot,
            config.command_high_priority_burst,
            config.command_attempt_timeout,
            config.command_total_timeout,
            config.command_max_retries,
            config.command_retry_base,
            config.command_retry_max,
            Arc::clone(&metrics),
        );
        handles.push(handle);
        workers.push(worker);
        registered.push(RegisteredAdapter {
            slot,
            platform,
            mode,
            adapter,
        });
    }

    Ok((registered, workers, BotDirectory::new(handles)))
}

async fn run_application<S, F>(app: OxideBot<S>, shutdown_signal: F, mode: RunMode) -> Result<()>
where
    S: Send + Sync + 'static,
    F: Future<Output = ()>,
{
    let OxideBot {
        state,
        config,
        adapters,
        handlers,
        filters,
        services,
        metrics,
    } = app;

    let (registered, command_workers, bot_directory) =
        register_bots(adapters, &config, Arc::clone(&metrics))?;
    let bot_count = bot_directory.len();
    let cancellation = CancellationToken::new();

    let (sessions, session_workers) = SessionRegistry::new(
        config.session_shards,
        config.session_commands_per_shard,
        config.max_sessions,
        Arc::clone(&metrics),
    );
    let router = Arc::new(CompiledRouter::compile(
        handlers,
        filters,
        Arc::clone(&state),
        sessions.clone(),
        cancellation.child_token(),
        config.handler_timeout,
        Arc::clone(&metrics),
    ));
    let interest = router.interest();

    let mut command_tasks = JoinSet::new();
    for worker in command_workers {
        command_tasks.spawn(worker.run());
    }

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
    drop(router);

    let (event_sink, ingress_receiver) = EventSink::channel(config.ingress);
    let mut dispatcher = tokio::spawn(dispatch_loop(
        ingress_receiver,
        bot_directory.clone(),
        sessions.clone(),
        executor,
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
        let context = AdapterContext::new(
            registered.slot,
            registered.platform,
            event_sink.clone(),
            interest.clone(),
            cancellation.child_token(),
            Arc::clone(&metrics),
        );
        adapter_tasks.spawn(async move { (adapter_mode, registered.adapter.run(context).await) });
    }

    // Keep one ingress sender under the supervisor's control. If adapters own
    // every sender, a finite adapter can drop its context just before its task
    // completion becomes visible to `JoinSet`; the dispatcher and executor
    // then finish first and are incorrectly reported as early exits.

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
                if adapters_remaining > 0 && fatal_error.is_none() {
                    fatal_error = Some(RuntimeError::Channel(
                        "event dispatcher exited while adapters are still running",
                    ));
                } else if let Err(error) = result {
                    fatal_error = Some(error);
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

#[allow(clippy::too_many_arguments)]
async fn dispatch_loop(
    mut receiver: mpsc::Receiver<IngressBatch>,
    bots: BotDirectory,
    sessions: SessionRegistry,
    executor: ExecutorHandle,
    dedupe_capacity: usize,
    dedupe_max_bytes: usize,
    dedupe_ttl: std::time::Duration,
    metrics: MetricsHandle,
) -> Result<()> {
    let mut sequence = 0_u64;
    let mut dedupe = DedupeCache::new(dedupe_capacity, dedupe_max_bytes, dedupe_ttl, bots.len());

    while let Some(ingress) = receiver.recv().await {
        let received_at = Instant::now();
        let retention = Arc::new(ingress.lease);
        let raw = ingress.batch.raw;
        for draft in ingress.batch.events {
            let slot = draft.index.bot;
            let bot = bots.get(slot).cloned().ok_or(RuntimeError::Channel(
                "event references an unknown bot slot",
            ))?;
            if !dedupe.insert(slot, draft.id.clone(), received_at) {
                metrics.duplicate_event();
                continue;
            }
            let event = Arc::new(draft.finalize(sequence, received_at, raw.clone()));
            sequence = sequence
                .checked_add(1)
                .ok_or(RuntimeError::Channel("event sequence exhausted"))?;

            let delivery = sessions
                .deliver(Arc::clone(&event), Arc::clone(&retention))
                .await
                .map_err(|_| RuntimeError::Channel("session registry is closed"))?;
            match delivery {
                SessionDelivery::Consumed => {
                    metrics.session_consumed();
                    continue;
                }
                SessionDelivery::Tap | SessionDelivery::None => {}
            }

            match executor.submit(event, bot, Arc::clone(&retention)).await {
                ExecutorSubmit::Accepted | ExecutorSubmit::Dropped => {}
                ExecutorSubmit::Closed => {
                    return Err(RuntimeError::Channel("executor is closed"));
                }
            }
        }
    }
    Ok(())
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
