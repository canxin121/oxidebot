use crate::{
    adapter::{AdapterContext, AdapterLimits, AdapterMode, EventSink, IngressBatch},
    bot::{CommandWorker, GlobalCommandCapacity},
    budget::{HierarchicalLease, PriorityQueueLimiter},
    dedupe::{DedupeCache, DedupeCommit},
    executor::{ExecutorHandle, ExecutorSubmit},
    handler::{erase_handler, prepare_handler, ErasedHandler, PreparedHandler, RouteSpec},
    router::{CompiledRouter, RouterLimits},
    session::{SessionDelivery, SessionRegistry},
    Adapter, BotDescriptor, BotDirectory, BotServices, BuildError, Filter, Handler, MetricsHandle,
    Result, RuntimeConfig, RuntimeError, RuntimeMetrics, RuntimeProfile, Service, ServiceContext,
    ShutdownSignal,
};
use futures_util::{stream::FuturesUnordered, StreamExt};
use oxidebot_core::{BotIdentity, BotSlot, EventEnvelope, EventId, MAX_ROUTE_KEY_BYTES};
use std::{
    collections::{HashSet, VecDeque},
    future::{pending, Future},
    panic::{catch_unwind, AssertUnwindSafe},
    sync::Arc,
    time::Instant,
};
use tokio::{
    sync::mpsc,
    task::{JoinHandle, JoinSet},
};
use tokio_util::sync::CancellationToken;

/// Safety bound for process-local adapter fan-out and per-bot runtime tables.
const MAX_RUNTIME_BOTS: usize = 4_096;
/// Prevent pathological build-time route tables and RouteId exhaustion.
const MAX_RUNTIME_HANDLERS: usize = 1_000_000;

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
        let Self {
            state,
            config,
            adapters,
            handlers,
            filters,
            services,
            metrics,
        } = self;
        config.validate()?;
        if handlers.len() > MAX_RUNTIME_HANDLERS {
            return Err(BuildError::InvalidConfig(
                "route handler count exceeds the runtime safety limit",
            ));
        }
        let handlers = handlers
            .into_iter()
            .map(prepare_handler)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        validate_routes(&handlers)?;
        let adapters = prepare_adapters(adapters)?;
        if config.dedupe_capacity < adapters.len() {
            return Err(BuildError::InvalidConfig(
                "dedupe capacity must provide at least one slot per registered bot",
            ));
        }
        validate_route_targets(&handlers, &adapters)?;
        Ok(Application {
            state,
            config,
            adapters,
            handlers,
            filters,
            services,
            metrics,
        })
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
    state: Arc<S>,
    config: RuntimeConfig,
    adapters: Vec<PreparedAdapter>,
    handlers: Vec<PreparedHandler<S>>,
    filters: Vec<Arc<dyn Filter<S>>>,
    services: Vec<Arc<dyn Service<S>>>,
    metrics: MetricsHandle,
}

impl<S> Application<S>
where
    S: Send + Sync + 'static,
{
    #[must_use]
    pub fn metrics(&self) -> MetricsHandle {
        Arc::clone(&self.metrics)
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
            .adapters
            .iter()
            .any(|adapter| adapter.mode != AdapterMode::Finite)
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
        run_application(self, shutdown_signal, mode).await
    }
}

#[derive(Clone, Copy)]
enum RunMode {
    Normal,
    Finite,
}

fn validate_routes<S>(handlers: &[PreparedHandler<S>]) -> std::result::Result<(), BuildError>
where
    S: Send + Sync + 'static,
{
    for handler in handlers {
        let expected_kind = match &handler.spec {
            RouteSpec::Generic(kind) => *kind,
            RouteSpec::Command(_) => oxidebot_core::EventKind::MessageCreated,
            RouteSpec::Interaction(_) => oxidebot_core::EventKind::Interaction,
            RouteSpec::Native(_) => oxidebot_core::EventKind::Native,
        };
        if handler.event_kind != expected_kind {
            return Err(BuildError::InvalidRoute(
                "matcher event view differs from its compiled route category".into(),
            ));
        }
        if let Some(platform) = &handler.scope.platform {
            platform
                .validate()
                .map_err(|error| BuildError::InvalidRoute(error.to_string()))?;
        }
        if let Some(bot) = &handler.scope.bot {
            bot.platform
                .validate()
                .map_err(|error| BuildError::InvalidRoute(error.to_string()))?;
            bot.bot
                .validate()
                .map_err(|error| BuildError::InvalidRoute(error.to_string()))?;
            if handler
                .scope
                .platform
                .as_ref()
                .is_some_and(|platform| platform != &bot.platform)
            {
                return Err(BuildError::InvalidRoute(
                    "route bot identity disagrees with its platform scope".into(),
                ));
            }
        }
        let key = match &handler.spec {
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

fn validate_route_targets<S>(
    handlers: &[PreparedHandler<S>],
    adapters: &[PreparedAdapter],
) -> std::result::Result<(), BuildError>
where
    S: Send + Sync + 'static,
{
    let identities = adapters
        .iter()
        .map(|adapter| adapter.identity.clone())
        .collect::<HashSet<_>>();
    let platforms = adapters
        .iter()
        .map(|adapter| adapter.identity.platform.clone())
        .collect::<HashSet<_>>();

    for handler in handlers {
        if let Some(bot) = &handler.scope.bot {
            if !identities.contains(bot) {
                return Err(BuildError::InvalidRoute(format!(
                    "route targets unregistered bot {}:{}",
                    bot.platform, bot.bot
                )));
            }
        } else if let Some(platform) = &handler.scope.platform {
            if !platforms.contains(platform) {
                return Err(BuildError::InvalidRoute(format!(
                    "route targets unregistered platform {platform}"
                )));
            }
        }
    }
    Ok(())
}

struct PreparedAdapter {
    identity: BotIdentity,
    descriptor: BotDescriptor,
    services: BotServices,
    mode: AdapterMode,
    adapter: Box<dyn Adapter>,
}

fn prepare_adapters(
    adapters: Vec<Box<dyn Adapter>>,
) -> std::result::Result<Vec<PreparedAdapter>, BuildError> {
    if adapters.is_empty() {
        return Err(BuildError::NoAdapters);
    }
    if adapters.len() > MAX_RUNTIME_BOTS {
        return Err(BuildError::TooManyBots);
    }
    let mut identities = HashSet::<BotIdentity>::with_capacity(adapters.len());
    let mut prepared = Vec::with_capacity(adapters.len());
    for adapter in adapters {
        let (descriptor, mode, services) = catch_unwind(AssertUnwindSafe(|| {
            (adapter.descriptor(), adapter.mode(), adapter.services())
        }))
        .map_err(|_| BuildError::InvalidBot("adapter metadata panicked".into()))?;
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
        prepared.push(PreparedAdapter {
            identity,
            descriptor,
            services,
            mode,
            adapter,
        });
    }
    Ok(prepared)
}

struct RegisteredAdapter {
    slot: BotSlot,
    identity: BotIdentity,
    platform: oxidebot_core::PlatformId,
    mode: AdapterMode,
    adapter: Box<dyn Adapter>,
}

fn register_bots(
    adapters: Vec<PreparedAdapter>,
    config: &RuntimeConfig,
    metrics: MetricsHandle,
) -> std::result::Result<(Vec<RegisteredAdapter>, Vec<CommandWorker>, BotDirectory), BuildError> {
    let mut registered = Vec::with_capacity(adapters.len());
    let mut workers = Vec::with_capacity(adapters.len());
    let mut handles = Vec::with_capacity(adapters.len());
    let global_command_limiter =
        PriorityQueueLimiter::new(config.global_command, config.max_command_bytes);
    let global_command_capacity = GlobalCommandCapacity::new(
        config.command_in_flight_global,
        config.command_in_flight_reserved_high_global,
    );

    for (index, prepared) in adapters.into_iter().enumerate() {
        let slot = BotSlot(u32::try_from(index).map_err(|_| BuildError::TooManyBots)?);
        let PreparedAdapter {
            identity,
            descriptor,
            services,
            mode,
            adapter,
        } = prepared;
        let platform = descriptor.platform.clone();
        let (handle, worker) = CommandWorker::build(
            slot,
            descriptor,
            services,
            global_command_limiter.clone(),
            global_command_capacity.clone(),
            config.command,
            config.max_command_bytes,
            config.command_overload,
            config.command_in_flight_per_bot,
            config.command_in_flight_reserved_high_per_bot,
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
            identity,
            platform,
            mode,
            adapter,
        });
    }

    Ok((registered, workers, BotDirectory::new(handles)))
}

async fn run_application<S, F>(app: Application<S>, shutdown_signal: F, mode: RunMode) -> Result<()>
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
        ShutdownSignal::new(cancellation.child_token()),
        RouterLimits {
            handler_timeout: config.handler_timeout,
            max_handler_replies: config.max_handler_replies,
        },
        Arc::clone(&metrics),
    ));
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

struct PendingDispatch {
    slot: BotSlot,
    event: Arc<EventEnvelope>,
    bot: crate::BotHandle,
    ingress_retention: Arc<HierarchicalLease>,
}

enum AdmissionOutcome {
    SessionConsumed,
    Executor(ExecutorSubmit),
    SessionClosed,
}

struct AdmissionCompletion {
    slot: BotSlot,
    event_id: EventId,
    outcome: AdmissionOutcome,
}

#[allow(clippy::too_many_arguments)]
async fn dispatch_loop(
    mut receiver: mpsc::Receiver<IngressBatch>,
    bots: BotDirectory,
    sessions: SessionRegistry,
    executor: ExecutorHandle,
    max_admissions: usize,
    dedupe_capacity: usize,
    dedupe_max_bytes: usize,
    dedupe_ttl: std::time::Duration,
    metrics: MetricsHandle,
) -> Result<()> {
    let mut sequence = 0_u64;
    let mut dedupe = DedupeCache::new(dedupe_capacity, dedupe_max_bytes, dedupe_ttl, bots.len());
    let mut inflight_ids = HashSet::<(BotSlot, EventId)>::new();
    let mut pending = (0..bots.len())
        .map(|_| VecDeque::<PendingDispatch>::new())
        .collect::<Vec<_>>();
    let mut ready_bots = VecDeque::<BotSlot>::new();
    let mut ready_set = vec![false; bots.len()];
    let mut admission_active = vec![false; bots.len()];
    let mut admissions = FuturesUnordered::new();
    let mut input_closed = false;

    loop {
        let mut admission_attempts = ready_bots.len();
        while admission_attempts > 0 && admissions.len() < max_admissions {
            admission_attempts -= 1;
            let Some(slot) = ready_bots.pop_front() else {
                break;
            };
            let bot_index = slot.0 as usize;
            let Some(ready) = ready_set.get_mut(bot_index) else {
                continue;
            };
            *ready = false;
            if admission_active.get(bot_index).copied().unwrap_or(true) {
                continue;
            }
            let Some(item) = pending.get_mut(bot_index).and_then(VecDeque::pop_front) else {
                continue;
            };
            admission_active[bot_index] = true;
            admissions.push(admit_event(executor.clone(), sessions.clone(), item));
        }

        if input_closed && admissions.is_empty() && pending.iter().all(VecDeque::is_empty) {
            break;
        }

        tokio::select! {
            ingress = receiver.recv(), if !input_closed => {
                let Some(ingress) = ingress else {
                    input_closed = true;
                    continue;
                };
                let received_at = Instant::now();
                let retention = Arc::new(ingress.lease);
                let raw = ingress.batch.raw;
                for draft in ingress.batch.events {
                    let slot = draft.index.bot;
                    let bot = bots.get(slot).cloned().ok_or(RuntimeError::Channel(
                        "event references an unknown bot slot",
                    ))?;
                    let inflight_key = (slot, draft.id.clone());
                    if dedupe.contains(slot, &draft.id, received_at)
                        || !inflight_ids.insert(inflight_key.clone())
                    {
                        metrics.duplicate_event();
                        continue;
                    }

                    let event = Arc::new(draft.finalize(sequence, received_at, raw.clone()));
                    sequence = sequence
                        .checked_add(1)
                        .ok_or(RuntimeError::Channel("event sequence exhausted"))?;
                    let bot_index = slot.0 as usize;
                    let Some(queue) = pending.get_mut(bot_index) else {
                        inflight_ids.remove(&inflight_key);
                        return Err(RuntimeError::Channel(
                            "event references an unknown bot slot",
                        ));
                    };
                    let was_empty = queue.is_empty();
                    queue.push_back(PendingDispatch {
                        slot,
                        event,
                        bot,
                        ingress_retention: Arc::clone(&retention),
                    });
                    if was_empty && !admission_active[bot_index] && !ready_set[bot_index] {
                        ready_set[bot_index] = true;
                        ready_bots.push_back(slot);
                    }
                }
            }
            completion = admissions.next(), if !admissions.is_empty() => {
                if let Some(completion) = completion {
                    let bot_index = completion.slot.0 as usize;
                    if let Some(active) = admission_active.get_mut(bot_index) {
                        *active = false;
                    }
                    inflight_ids.remove(&(completion.slot, completion.event_id.clone()));
                    match completion.outcome {
                        AdmissionOutcome::SessionConsumed => {
                            metrics.session_consumed();
                            commit_dedupe(
                                &mut dedupe,
                                completion.slot,
                                completion.event_id,
                                Instant::now(),
                                &metrics,
                            );
                        }
                        AdmissionOutcome::Executor(
                            ExecutorSubmit::Accepted | ExecutorSubmit::DroppedByPolicy,
                        ) => {
                            // DropNewest is explicitly drop-and-ack: once shed, a
                            // redelivery is still considered a duplicate.
                            commit_dedupe(
                                &mut dedupe,
                                completion.slot,
                                completion.event_id,
                                Instant::now(),
                                &metrics,
                            );
                        }
                        AdmissionOutcome::Executor(ExecutorSubmit::RejectedTooLarge) => {
                            return Err(RuntimeError::EventTooLarge(
                                completion.event_id.to_string(),
                            ));
                        }
                        AdmissionOutcome::Executor(ExecutorSubmit::Closed) => {
                            return Err(RuntimeError::Channel("executor is closed"));
                        }
                        AdmissionOutcome::SessionClosed => {
                            return Err(RuntimeError::Channel("session registry is closed"));
                        }
                    }
                    if pending
                        .get(bot_index)
                        .is_some_and(|queue| !queue.is_empty())
                        && !ready_set[bot_index]
                    {
                        ready_set[bot_index] = true;
                        ready_bots.push_back(completion.slot);
                    }
                }
            }
        }
    }
    Ok(())
}

async fn admit_event(
    executor: ExecutorHandle,
    sessions: SessionRegistry,
    pending: PendingDispatch,
) -> AdmissionCompletion {
    let event_id = pending.event.id.clone();
    let delivery = sessions
        .deliver(
            Arc::clone(&pending.event),
            Arc::clone(&pending.ingress_retention),
        )
        .await;
    let outcome = match delivery {
        Ok(SessionDelivery::Consumed) => AdmissionOutcome::SessionConsumed,
        Ok(SessionDelivery::Tap | SessionDelivery::None) => AdmissionOutcome::Executor(
            executor
                .submit(pending.event, pending.bot, pending.ingress_retention)
                .await,
        ),
        Err(_) => AdmissionOutcome::SessionClosed,
    };
    AdmissionCompletion {
        slot: pending.slot,
        event_id,
        outcome,
    }
}

fn commit_dedupe(
    dedupe: &mut DedupeCache,
    slot: BotSlot,
    id: EventId,
    now: Instant,
    metrics: &MetricsHandle,
) {
    match dedupe.commit(slot, id.clone(), now) {
        DedupeCommit::Inserted => {}
        DedupeCommit::Duplicate => metrics.duplicate_event(),
        DedupeCommit::Uncacheable => {
            metrics.dedupe_uncacheable();
            tracing::warn!(bot_slot = slot.0, event_id = %id, "event id cannot fit in dedupe byte budget");
        }
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
