use crate::{
    adapter::AdapterMode,
    handler::{prepare_handler, PreparedHandler, RouteSpec},
    Adapter, AuthoringRuntime, BuildError, CatalogCommandRenderer, CommandCatalog,
    CommandMiddleware, CommandOutputMiddleware, CommandRegistry, CommandRenderer, CommandRewriter,
    DeliveryMiddleware, Filter, LocaleResolver, MessageNormalizer, MetricsHandle, Result,
    RuntimeConfig, RuntimeMetrics, RuntimeProfile, Service,
};
use oxidebot_core::event::kernel::{DispatchKind, MAX_ROUTE_KEY_BYTES};
use oxidebot_core::{TranslationCatalog, TranslationError};
use std::{
    collections::HashSet,
    future::{pending, Future},
    path::Path,
    sync::Arc,
};

#[path = "app_adapter.rs"]
mod app_adapter;
#[path = "app_dispatch.rs"]
mod app_dispatch;
#[path = "app_runtime.rs"]
mod app_runtime;

pub(crate) use app_adapter::publish_command_definitions;
use app_adapter::{prepare_adapters, PreparedAdapter};
use app_runtime::run_application;

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
    module: crate::Module<S>,
    filters: Vec<Arc<dyn Filter<S>>>,
    services: Vec<Arc<dyn Service<S>>>,
    plugin_requirements: Vec<crate::PluginRequirement>,
    metrics: MetricsHandle,
    authoring: AuthoringRuntime<S>,
}

impl OxideBot<()> {
    /// Creates an application with unit root state and balanced defaults.
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
    /// Creates an application with caller-owned immutable root state.
    #[must_use]
    pub fn with_state(state: S) -> Self {
        Self {
            state: Arc::new(state),
            config: RuntimeConfig::default(),
            adapters: Vec::new(),
            module: crate::Module::new(),
            filters: Vec::new(),
            services: Vec::new(),
            plugin_requirements: Vec::new(),
            metrics: Arc::new(RuntimeMetrics::default()),
            authoring: AuthoringRuntime::default(),
        }
    }

    /// Replaces configuration with one predefined resource profile.
    #[must_use]
    pub fn profile(mut self, profile: RuntimeProfile) -> Self {
        self.config = RuntimeConfig::for_profile(profile);
        self
    }

    /// Replaces complete runtime configuration.
    #[must_use]
    pub fn config(mut self, config: RuntimeConfig) -> Self {
        self.config = config;
        self
    }

    /// Replaces runtime metrics implementation.
    #[must_use]
    pub fn metrics(mut self, metrics: MetricsHandle) -> Self {
        self.metrics = metrics;
        self
    }

    /// Returns a clone of the configured metrics handle.
    #[must_use]
    pub fn metrics_handle(&self) -> MetricsHandle {
        Arc::clone(&self.metrics)
    }

    /// Replaces the command output renderer.
    #[must_use]
    pub fn command_renderer<R>(mut self, renderer: R) -> Self
    where
        R: CommandRenderer,
    {
        self.authoring.renderer = Arc::new(renderer);
        self
    }

    /// Replaces locale resolution for application authoring.
    #[must_use]
    pub fn locale_resolver<R>(mut self, resolver: R) -> Self
    where
        R: LocaleResolver<S>,
    {
        self.authoring.locale_resolver = Arc::new(resolver);
        self
    }

    /// Installs the bounded message-template catalog used by the `I18n`
    /// extractor and application-authored localized messages.
    #[must_use]
    pub fn translations(mut self, catalog: TranslationCatalog) -> Self {
        self.authoring.translations = Some(catalog);
        self
    }

    /// Installs one catalog for application messages and command output. The
    /// resource-backed renderer falls back to the built-in renderer when an
    /// `oxidebot.command.*` template is absent.
    #[must_use]
    pub fn localization(mut self, catalog: TranslationCatalog) -> Self {
        self.authoring.renderer = Arc::new(CatalogCommandRenderer::new(catalog.clone()));
        self.authoring.translations = Some(catalog);
        self
    }

    /// Loads bounded structure-preserving message translations from a
    /// directory containing `<locale>.json` files.
    pub fn translations_from_dir(
        self,
        directory: impl AsRef<Path>,
        default_locale: impl Into<Arc<str>>,
        capacity: usize,
    ) -> std::result::Result<Self, TranslationError> {
        Ok(self.translations(TranslationCatalog::from_dir(
            capacity,
            default_locale,
            directory,
        )?))
    }

    /// Loads `<locale>.json` resources and enables both `I18n` messages and
    /// resource-backed command rendering.
    pub fn localization_from_dir(
        self,
        directory: impl AsRef<Path>,
        default_locale: impl Into<Arc<str>>,
        capacity: usize,
    ) -> std::result::Result<Self, TranslationError> {
        Ok(self.localization(TranslationCatalog::from_dir(
            capacity,
            default_locale,
            directory,
        )?))
    }

    /// Replaces the command registry.
    #[must_use]
    pub fn command_registry(mut self, registry: CommandRegistry) -> Self {
        self.authoring.registry = registry;
        self
    }

    /// Adds a command input rewriter.
    #[must_use]
    pub fn command_rewriter<R>(mut self, rewriter: R) -> Self
    where
        R: CommandRewriter<S>,
    {
        self.authoring.rewriters.push(Arc::new(rewriter));
        self
    }

    /// Adds a portable message normalizer.
    #[must_use]
    pub fn message_normalizer<N>(mut self, normalizer: N) -> Self
    where
        N: MessageNormalizer<S>,
    {
        self.authoring.normalizers.push(Arc::new(normalizer));
        self
    }

    /// Adds command input middleware.
    #[must_use]
    pub fn command_middleware<M>(mut self, middleware: M) -> Self
    where
        M: CommandMiddleware<S>,
    {
        self.authoring.command_middleware.push(Arc::new(middleware));
        self
    }

    /// Adds command output middleware.
    #[must_use]
    pub fn command_output_middleware<M>(mut self, middleware: M) -> Self
    where
        M: CommandOutputMiddleware<S>,
    {
        self.authoring.output_middleware.push(Arc::new(middleware));
        self
    }

    /// Adds delivery middleware.
    #[must_use]
    pub fn delivery_middleware<M>(mut self, middleware: M) -> Self
    where
        M: DeliveryMiddleware<S>,
    {
        self.authoring
            .delivery_middleware
            .push(Arc::new(middleware));
        self
    }

    /// Registers an adapter.
    #[must_use]
    pub fn adapter<A>(mut self, adapter: A) -> Self
    where
        A: Adapter,
    {
        self.adapters.push(Box::new(adapter));
        self
    }

    /// Installs one generated command, locally configured feature, or complete
    /// module directly into this application.
    #[must_use]
    #[allow(
        clippy::should_implement_trait,
        reason = "`add` installs a feature into this fluent application builder; it is not arithmetic"
    )]
    pub fn add<F>(mut self, feature: F) -> Self
    where
        F: crate::IntoFeature<S>,
    {
        self.module = self.module.add(feature);
        self
    }

    /// Includes one reusable Bot feature module.
    ///
    /// Modules remain uncompiled until `build`, so commands from separate
    /// includes share one help catalog and duplicate help handlers are removed.
    #[must_use]
    pub fn include(mut self, module: crate::Module<S>) -> Self {
        self.module = self.module.include(module);
        self
    }

    /// Installs one reusable plugin bundle.
    ///
    /// The bundle contributes its flat handlers and supervised services to
    /// this same application; it never creates a nested runtime or router.
    #[must_use]
    pub fn plugin(mut self, plugin: crate::PluginBundle<S>) -> Self {
        let parts = plugin.into_parts();
        let _metadata = parts.metadata;
        self.module = self.module.include(parts.module);
        self.services.extend(parts.services);
        self.plugin_requirements.extend(parts.requirements);
        self
    }

    #[must_use]
    /// Adds an application-wide event filter.
    pub fn filter<F>(mut self, filter: F) -> Self
    where
        F: Filter<S>,
    {
        self.filters.push(Arc::new(filter));
        self
    }

    #[must_use]
    /// Adds a supervised background service.
    pub fn service<T>(mut self, service: T) -> Self
    where
        T: Service<S>,
    {
        self.services.push(Arc::new(service));
        self
    }

    /// Validates and compiles the immutable runnable application.
    pub fn build(self) -> std::result::Result<Application<S>, BuildError> {
        let Self {
            state,
            config,
            adapters,
            module,
            filters,
            services,
            plugin_requirements,
            metrics,
            mut authoring,
        } = self;
        config.validate()?;
        module.install_completers(&mut authoring.completers)?;
        let dynamic_shortcuts = module.runtime_shortcuts_enabled();
        let command_catalog = module.catalog();
        let has_static_shortcuts = command_catalog
            .commands()
            .iter()
            .any(|command| !command.shortcuts().is_empty());
        authoring.registry.configure_matching(
            dynamic_shortcuts || has_static_shortcuts,
            !authoring.rewriters.is_empty() || !authoring.normalizers.is_empty(),
        );
        for command in command_catalog.commands() {
            authoring
                .registry
                .register(command)
                .map_err(|error| BuildError::InvalidRoute(error.to_string()))?;
        }
        let handlers = module.into_handlers()?;
        if handlers.len() > MAX_RUNTIME_HANDLERS {
            return Err(BuildError::InvalidConfig(
                "handler count exceeds the runtime safety limit",
            ));
        }
        let handlers = handlers
            .into_iter()
            .map(prepare_handler)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        validate_routes(&handlers)?;
        let adapters = prepare_adapters(adapters)?;
        validate_plugin_requirements(&plugin_requirements, &adapters)?;
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
            command_catalog,
            metrics,
            authoring: Arc::new(authoring),
        })
    }

    /// Runs until Ctrl-C (with the default `signal` feature), fatal failure, or
    /// natural completion of every finite adapter.
    /// Runs until shutdown, fatal failure, or finite adapter completion.
    pub async fn run(self) -> Result<()> {
        self.build()?.run().await
    }

    /// Runs a finite replay/import application to completion.
    pub async fn run_to_completion(self) -> Result<()> {
        self.build()?.run_to_completion().await
    }
}

fn validate_plugin_requirements(
    requirements: &[crate::PluginRequirement],
    adapters: &[PreparedAdapter],
) -> std::result::Result<(), BuildError> {
    for requirement in requirements {
        for adapter in adapters {
            if !(requirement.predicate)(adapter.services.bot_capabilities()) {
                return Err(BuildError::InvalidRoute(format!(
                    "plugin requirement {:?} is not satisfied by {}:{}",
                    requirement.description, adapter.identity.platform, adapter.identity.bot
                )));
            }
        }
    }
    Ok(())
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
    command_catalog: CommandCatalog,
    metrics: MetricsHandle,
    authoring: Arc<AuthoringRuntime<S>>,
}

impl<S> Application<S>
where
    S: Send + Sync + 'static,
{
    /// Returns a clone of this application's metrics handle.
    #[must_use]
    pub fn metrics(&self) -> MetricsHandle {
        Arc::clone(&self.metrics)
    }

    /// Runs until Ctrl-C, fatal failure, or finite adapter completion.
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

    /// Runs until fatal failure or finite adapter completion when signals are disabled.
    #[cfg(not(feature = "signal"))]
    pub async fn run(self) -> Result<()> {
        self.run_internal(pending(), RunMode::Normal).await
    }

    /// Runs only finite adapters until every adapter completes.
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

    /// Runs until the caller-provided shutdown future resolves.
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
            RouteSpec::Event(event_type) => event_type.dispatch_kind(),
            RouteSpec::Command(_) => DispatchKind::Message,
            RouteSpec::Interaction(_) => DispatchKind::Interaction,
            RouteSpec::Native(_) => DispatchKind::Native,
        };
        if handler.event_kind != expected_kind {
            return Err(BuildError::InvalidRoute(
                "handler event type differs from its compiled dispatch category".into(),
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
            RouteSpec::Event(_) => continue,
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
