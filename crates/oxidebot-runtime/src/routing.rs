use crate::{
    function::IntoHandler,
    handler::{erase_handler, ErasedHandler, EventView, Handler, Matcher, RouteScope, RouteSpec},
    middleware::{Endpoint, Middleware, Next},
    BotHandle, BuildError, Command, CommandCatalog, Request, Response, SessionRegistry,
    ShutdownSignal,
};
use futures_util::future::BoxFuture;
use oxidebot_core::{
    event::{
        kernel::{DispatchEnvelope, DispatchKind},
        tags, EventTag, EventType,
    },
    BotIdentity, PlatformId,
};
use std::{marker::PhantomData, sync::Arc};

/// The single compositional unit for commands, events, middleware, and plugins.
pub struct Router<S = ()>
where
    S: Send + Sync + 'static,
{
    endpoints: Vec<EndpointDefinition<S>>,
    legacy: Vec<Arc<dyn ErasedHandler<S>>>,
    layers: Vec<Arc<dyn Middleware<S>>>,
    default_scope: RouteScope,
    has_help: bool,
}

impl<S> Default for Router<S>
where
    S: Send + Sync + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<S> Router<S>
where
    S: Send + Sync + 'static,
{
    #[must_use]
    pub fn new() -> Self {
        Self {
            endpoints: Vec::new(),
            legacy: Vec::new(),
            layers: Vec::new(),
            default_scope: RouteScope::global(),
            has_help: false,
        }
    }

    /// Adds a route using the compile-time matcher API.
    #[must_use]
    pub fn route<M, H, T>(mut self, matcher: M, handler: H) -> Self
    where
        M: Matcher,
        H: IntoHandler<T, S>,
    {
        self.endpoints.push(EndpointDefinition {
            selector: Selector::Route {
                spec: matcher.route_spec(),
                kind: <M::Event as EventView>::KIND,
            },
            endpoint: EndpointKind::Handler(handler.into_endpoint()),
            layers: Vec::new(),
            scope: self.default_scope.clone(),
        });
        self
    }

    /// Adds one exact event-tag route. Passing the zero-sized marker lets Rust
    /// infer the handler and extractor tuple without turbofish placeholders.
    #[must_use]
    pub fn event<E, H, T>(self, _event: E, handler: H) -> Self
    where
        E: EventTag,
        H: IntoHandler<T, S>,
    {
        self.route(crate::event::<E>(), handler)
    }

    /// Adds a route for every ordinary message event.
    #[must_use]
    pub fn message<H, T>(self, handler: H) -> Self
    where
        H: IntoHandler<T, S>,
    {
        self.route(crate::message(), handler)
    }

    /// Adds a strongly typed command route.
    #[must_use]
    pub fn command<H, T>(mut self, command: Command, handler: H) -> Self
    where
        H: IntoHandler<T, S>,
    {
        self.endpoints.push(EndpointDefinition {
            selector: Selector::Command(command),
            endpoint: EndpointKind::Handler(handler.into_endpoint()),
            layers: Vec::new(),
            scope: self.default_scope.clone(),
        });
        self
    }

    /// Adds one interaction custom-id route.
    #[must_use]
    pub fn interaction<H, T>(self, custom_id: impl Into<Arc<str>>, handler: H) -> Self
    where
        H: IntoHandler<T, S>,
    {
        self.route(crate::interaction(custom_id), handler)
    }

    /// Adds one platform-native event route.
    #[must_use]
    pub fn native<H, T>(self, kind: impl Into<Arc<str>>, handler: H) -> Self
    where
        H: IntoHandler<T, S>,
    {
        self.route(crate::native(kind), handler)
    }

    /// Keeps the original typed-context route API available as an escape hatch.
    #[must_use]
    pub fn handler<H>(mut self, handler: H) -> Self
    where
        H: Handler<S>,
    {
        self.legacy.push(erase_handler(handler));
        self
    }

    /// Merges another router without introducing another runtime or event bus.
    #[must_use]
    pub fn merge(mut self, mut other: Self) -> Self {
        other.bake_layers();
        self.endpoints.append(&mut other.endpoints);
        self.legacy.append(&mut other.legacy);
        self.has_help |= other.has_help;
        self
    }

    /// Mounts commands below a space-separated command prefix. Event routes are
    /// merged unchanged.
    #[must_use]
    pub fn mount(mut self, prefix: impl AsRef<str>, mut other: Self) -> Self {
        other.bake_layers();
        let prefix = prefix.as_ref().trim();
        for endpoint in &mut other.endpoints {
            if let Selector::Command(command) = &mut endpoint.selector {
                *command = command.clone().mounted(prefix);
            }
        }
        self.endpoints.append(&mut other.endpoints);
        self.legacy.append(&mut other.legacy);
        self.has_help |= other.has_help;
        self
    }

    /// Installs a feature module represented by a router or router factory.
    #[must_use]
    pub fn plugin<P>(self, plugin: P) -> Self
    where
        P: Plugin<S>,
    {
        self.merge(plugin.into_router())
    }

    /// Applies middleware to every endpoint in this router, including endpoints
    /// merged later.
    #[must_use]
    pub fn layer<L>(mut self, layer: L) -> Self
    where
        L: Middleware<S>,
    {
        self.layers.push(Arc::new(layer));
        self
    }

    /// Applies middleware only to routes already present.
    #[must_use]
    pub fn route_layer<L>(mut self, layer: L) -> Self
    where
        L: Middleware<S>,
    {
        let layer: Arc<dyn Middleware<S>> = Arc::new(layer);
        for endpoint in &mut self.endpoints {
            endpoint.layers.push(Arc::clone(&layer));
        }
        self
    }

    /// Scopes every current and subsequently added route to a platform.
    #[must_use]
    pub fn platform(mut self, platform: PlatformId) -> Self {
        let scope = RouteScope::for_platform(platform);
        for endpoint in &mut self.endpoints {
            endpoint.scope = scope.clone();
        }
        self.default_scope = scope;
        self
    }

    /// Scopes every current and subsequently added route to one bot connection.
    #[must_use]
    pub fn bot(mut self, bot: BotIdentity) -> Self {
        let scope = RouteScope::for_bot(bot);
        for endpoint in &mut self.endpoints {
            endpoint.scope = scope.clone();
        }
        self.default_scope = scope;
        self
    }

    /// Adds `/help [command]`, generated from the same schemas used for parsing.
    #[must_use]
    pub fn help(self) -> Self {
        self.help_command(
            crate::command("help")
                .alias("帮助")
                .description("显示命令目录或某个命令的详细帮助"),
        )
    }

    #[must_use]
    pub fn help_command(mut self, command: Command) -> Self {
        if self.has_help {
            return self;
        }
        self.has_help = true;
        self.endpoints.push(EndpointDefinition {
            selector: Selector::Command(command),
            endpoint: EndpointKind::Help,
            layers: Vec::new(),
            scope: self.default_scope.clone(),
        });
        self
    }

    #[must_use]
    pub fn catalog(&self) -> CommandCatalog {
        CommandCatalog::new(self.endpoints.iter().filter_map(|endpoint| {
            if let Selector::Command(command) = &endpoint.selector {
                Some(command.clone())
            } else {
                None
            }
        }))
    }

    pub(crate) fn into_handlers(mut self) -> Result<Vec<Arc<dyn ErasedHandler<S>>>, BuildError> {
        self.bake_layers();
        let catalog = Arc::new(self.catalog());
        let mut output = self.legacy;

        for definition in self.endpoints {
            let endpoint: Arc<dyn Endpoint<S>> = match definition.endpoint {
                EndpointKind::Handler(endpoint) => endpoint,
                EndpointKind::Help => Arc::new(HelpEndpoint::<S> {
                    catalog: Arc::clone(&catalog),
                    _state: PhantomData,
                }),
            };
            match definition.selector {
                Selector::Route { spec, kind } => {
                    output.push(Arc::new(RouteEndpoint {
                        spec,
                        event_kind: kind,
                        scope: definition.scope,
                        endpoint,
                        layers: definition.layers.into(),
                        command: None,
                    }));
                }
                Selector::Command(command) => {
                    command.validate().map_err(BuildError::InvalidRoute)?;
                    let specs = command.fast_route_keys().map_or_else(
                        || vec![RouteSpec::Event(EventType::Message)],
                        |keys| keys.into_iter().map(RouteSpec::Command).collect(),
                    );
                    for spec in specs {
                        output.push(Arc::new(RouteEndpoint {
                            spec,
                            event_kind: DispatchKind::Message,
                            scope: definition.scope.clone(),
                            endpoint: Arc::clone(&endpoint),
                            layers: definition.layers.clone().into(),
                            command: Some(command.clone()),
                        }));
                    }
                }
            }
        }
        Ok(output)
    }

    fn bake_layers(&mut self) {
        if self.layers.is_empty() {
            return;
        }
        for endpoint in &mut self.endpoints {
            endpoint.layers.extend(self.layers.iter().cloned());
        }
        self.layers.clear();
    }
}

/// A plugin is only a reusable router module; it shares state, middleware,
/// queues, sessions, events, and APIs with the application.
pub trait Plugin<S>
where
    S: Send + Sync + 'static,
{
    fn into_router(self) -> Router<S>;
}

impl<S> Plugin<S> for Router<S>
where
    S: Send + Sync + 'static,
{
    fn into_router(self) -> Router<S> {
        self
    }
}

impl<S, F> Plugin<S> for F
where
    S: Send + Sync + 'static,
    F: FnOnce() -> Router<S>,
{
    fn into_router(self) -> Router<S> {
        self()
    }
}

enum Selector {
    Route { spec: RouteSpec, kind: DispatchKind },
    Command(Command),
}

enum EndpointKind<S>
where
    S: Send + Sync + 'static,
{
    Handler(Arc<dyn Endpoint<S>>),
    Help,
}

struct EndpointDefinition<S>
where
    S: Send + Sync + 'static,
{
    selector: Selector,
    endpoint: EndpointKind<S>,
    layers: Vec<Arc<dyn Middleware<S>>>,
    scope: RouteScope,
}

struct RouteEndpoint<S>
where
    S: Send + Sync + 'static,
{
    spec: RouteSpec,
    event_kind: DispatchKind,
    scope: RouteScope,
    endpoint: Arc<dyn Endpoint<S>>,
    layers: Arc<[Arc<dyn Middleware<S>>]>,
    command: Option<Command>,
}

impl<S> ErasedHandler<S> for RouteEndpoint<S>
where
    S: Send + Sync + 'static,
{
    fn route_spec(&self) -> RouteSpec {
        self.spec.clone()
    }

    fn route_scope(&self) -> RouteScope {
        self.scope.clone()
    }

    fn event_kind(&self) -> DispatchKind {
        self.event_kind
    }

    fn call(
        &self,
        event: Arc<DispatchEnvelope>,
        state: Arc<S>,
        bot: BotHandle,
        sessions: SessionRegistry,
        shutdown: ShutdownSignal,
    ) -> BoxFuture<'static, crate::HandlerResult> {
        let command = if let Some(command) = &self.command {
            let Some(message) = event.event_as::<tags::Message>() else {
                return Box::pin(async { Ok(Response::continue_()) });
            };
            let Some(result) = command.match_event(message) else {
                return Box::pin(async { Ok(Response::continue_()) });
            };
            Some(result)
        } else {
            None
        };
        let request = Request::new(event, state, bot, sessions, shutdown, command);
        let next = Next::new(Arc::clone(&self.endpoint), Arc::clone(&self.layers));
        Box::pin(async move { Ok(next.run(request).await) })
    }
}

struct HelpEndpoint<S>
where
    S: Send + Sync + 'static,
{
    catalog: Arc<CommandCatalog>,
    _state: PhantomData<fn() -> S>,
}

impl<S> Endpoint<S> for HelpEndpoint<S>
where
    S: Send + Sync + 'static,
{
    fn call(&self, request: Request<S>) -> BoxFuture<'static, Response> {
        let catalog = Arc::clone(&self.catalog);
        let query = request
            .command()
            .map(|command| command.text_values().collect::<Vec<_>>().join(" "))
            .filter(|query| !query.is_empty());
        Box::pin(async move { Response::stop().text(catalog.render(query.as_deref())) })
    }
}
