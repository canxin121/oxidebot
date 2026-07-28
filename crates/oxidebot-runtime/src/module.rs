use crate::{
    command::tokenize_segments,
    function::IntoHandler,
    handler::{ErasedHandler, RouteScope, RouteSpec},
    hooks::{
        After, Before, Endpoint, Guard, GuardDecision, SharedAfter, SharedBefore, SharedGuard,
    },
    BotHandle, BuildError, Command, CommandCatalog, CommandParseError, CommandResult, Context,
    Dialogue, Extract, HandlerError, HandlerResult, Outcome, SessionRegistry, ShutdownSignal,
};
use futures_util::future::BoxFuture;
use oxidebot_core::{
    event::{
        kernel::{DispatchEnvelope, DispatchKind},
        tags, EventTag, EventType,
    },
    BotIdentity, Event, PlatformId,
};
use std::{marker::PhantomData, sync::Arc};

/// A flat, reusable Bot feature module.
///
/// A module groups commands, event handlers, guards, and lifecycle hooks. It is
/// deliberately not a URL tree: multi-word commands are declared as ordinary
/// command names, and including another module never creates hidden nesting.
pub struct Module<S = ()>
where
    S: Send + Sync + 'static,
{
    handlers: Vec<HandlerDefinition<S>>,
    guards: Vec<SharedGuard<S>>,
    before: Vec<SharedBefore<S>>,
    after: Vec<SharedAfter<S>>,
    default_scope: RouteScope,
    scope_errors: Vec<String>,
    has_help: bool,
}

impl<S> Default for Module<S>
where
    S: Send + Sync + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<S> Module<S>
where
    S: Send + Sync + 'static,
{
    #[must_use]
    pub fn new() -> Self {
        Self {
            handlers: Vec::new(),
            guards: Vec::new(),
            before: Vec::new(),
            after: Vec::new(),
            default_scope: RouteScope::global(),
            scope_errors: Vec::new(),
            has_help: false,
        }
    }

    /// Handles one exact 0.1.8 event type.
    #[must_use]
    pub fn on<E, H, T>(mut self, _event: E, handler: H) -> Self
    where
        E: EventTag,
        H: IntoHandler<T, S>,
    {
        self.handlers.push(HandlerDefinition {
            selector: Selector::Event {
                event_type: E::TYPE,
                kind: E::TYPE.dispatch_kind(),
            },
            endpoint: EndpointKind::Handler(handler.into_endpoint()),
            guards: Vec::new(),
            before: Vec::new(),
            after: Vec::new(),
            scope: self.default_scope.clone(),
            default_block: false,
        });
        self
    }

    /// Handles every ordinary message event.
    #[must_use]
    pub fn message<H, T>(self, handler: H) -> Self
    where
        H: IntoHandler<T, S>,
    {
        self.on(tags::Message, handler)
    }

    /// Handles one strongly typed command.
    #[must_use]
    pub fn command<H, T>(mut self, command: Command, handler: H) -> Self
    where
        H: IntoHandler<T, S>,
    {
        self.handlers.push(HandlerDefinition {
            selector: Selector::Command(command),
            endpoint: EndpointKind::Handler(handler.into_endpoint()),
            guards: Vec::new(),
            before: Vec::new(),
            after: Vec::new(),
            scope: self.default_scope.clone(),
            default_block: true,
        });
        self
    }

    /// Handles one interaction custom identifier.
    #[must_use]
    pub fn interaction<H, T>(mut self, custom_id: impl Into<Arc<str>>, handler: H) -> Self
    where
        H: IntoHandler<T, S>,
    {
        self.handlers.push(HandlerDefinition {
            selector: Selector::Interaction(custom_id.into()),
            endpoint: EndpointKind::Handler(handler.into_endpoint()),
            guards: Vec::new(),
            before: Vec::new(),
            after: Vec::new(),
            scope: self.default_scope.clone(),
            default_block: true,
        });
        self
    }

    /// Handles one platform-native lifecycle identifier.
    #[must_use]
    pub fn native<H, T>(mut self, kind: impl Into<Arc<str>>, handler: H) -> Self
    where
        H: IntoHandler<T, S>,
    {
        self.handlers.push(HandlerDefinition {
            selector: Selector::Native(kind.into()),
            endpoint: EndpointKind::Handler(handler.into_endpoint()),
            guards: Vec::new(),
            before: Vec::new(),
            after: Vec::new(),
            scope: self.default_scope.clone(),
            default_block: false,
        });
        self
    }

    /// Includes another feature module in registration order.
    ///
    /// No command prefix, route tree, runtime, or event bus is introduced.
    #[must_use]
    pub fn include(mut self, mut other: Self) -> Self {
        other.bake_hooks();
        self.scope_errors.append(&mut other.scope_errors);
        if !self.default_scope.is_global() {
            for handler in &mut other.handlers {
                match handler.scope.intersect(&self.default_scope) {
                    Ok(scope) => handler.scope = scope,
                    Err(error) => self.scope_errors.push(error),
                }
            }
        }
        if self.has_help && other.has_help {
            other
                .handlers
                .retain(|handler| !matches!(&handler.endpoint, EndpointKind::Help));
        } else {
            self.has_help |= other.has_help;
        }
        self.handlers.append(&mut other.handlers);
        self
    }

    /// Applies an async admission rule to every handler in this module,
    /// including handlers from included modules.
    #[must_use]
    pub fn guard<G>(mut self, guard: G) -> Self
    where
        G: Guard<S>,
    {
        self.guards.push(Arc::new(guard));
        self
    }

    /// Runs an async hook before each matched handler.
    #[must_use]
    pub fn before<B>(mut self, hook: B) -> Self
    where
        B: Before<S>,
    {
        self.before.push(Arc::new(hook));
        self
    }

    /// Runs an async hook after each matched handler.
    #[must_use]
    pub fn after<A>(mut self, hook: A) -> Self
    where
        A: After<S>,
    {
        self.after.push(Arc::new(hook));
        self
    }

    /// Restricts every handler in this module to one platform, regardless of declaration order.
    #[must_use]
    pub fn for_platform(mut self, platform: PlatformId) -> Self {
        let restriction = RouteScope::for_platform(platform);
        self.restrict_scope(&restriction);
        self
    }

    /// Restricts every handler in this module to one bot, regardless of declaration order.
    #[must_use]
    pub fn for_bot(mut self, bot: BotIdentity) -> Self {
        let restriction = RouteScope::for_bot(bot);
        self.restrict_scope(&restriction);
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
        self.handlers.push(HandlerDefinition {
            selector: Selector::Command(command),
            endpoint: EndpointKind::Help,
            guards: Vec::new(),
            before: Vec::new(),
            after: Vec::new(),
            scope: self.default_scope.clone(),
            default_block: true,
        });
        self
    }

    #[must_use]
    pub fn catalog(&self) -> CommandCatalog {
        CommandCatalog::new(self.handlers.iter().filter_map(|handler| {
            if let Selector::Command(command) = &handler.selector {
                Some(command.clone())
            } else {
                None
            }
        }))
    }

    pub(crate) fn into_handlers(mut self) -> Result<Vec<Arc<dyn ErasedHandler<S>>>, BuildError> {
        if let Some(error) = self.scope_errors.first().cloned() {
            return Err(BuildError::InvalidRoute(error));
        }
        self.bake_hooks();
        let catalog = Arc::new(self.catalog());
        let mut output: Vec<Arc<dyn ErasedHandler<S>>> = Vec::new();

        for definition in self.handlers {
            let endpoint: Arc<dyn Endpoint<S>> = match definition.endpoint {
                EndpointKind::Handler(endpoint) => endpoint,
                EndpointKind::Help => Arc::new(HelpEndpoint::<S> {
                    catalog: Arc::clone(&catalog),
                    _state: PhantomData,
                }),
            };
            match definition.selector {
                Selector::Event { event_type, kind } => {
                    output.push(Arc::new(ModuleHandler {
                        spec: RouteSpec::Event(event_type),
                        event_kind: kind,
                        scope: definition.scope,
                        endpoint,
                        guards: definition.guards.into(),
                        before: definition.before.into(),
                        after: definition.after.into(),
                        command: None,
                        default_block: definition.default_block,
                    }));
                }
                Selector::Interaction(custom_id) => {
                    output.push(Arc::new(ModuleHandler {
                        spec: RouteSpec::Interaction(custom_id),
                        event_kind: DispatchKind::Interaction,
                        scope: definition.scope,
                        endpoint,
                        guards: definition.guards.into(),
                        before: definition.before.into(),
                        after: definition.after.into(),
                        command: None,
                        default_block: definition.default_block,
                    }));
                }
                Selector::Native(kind) => {
                    output.push(Arc::new(ModuleHandler {
                        spec: RouteSpec::Native(kind),
                        event_kind: DispatchKind::Native,
                        scope: definition.scope,
                        endpoint,
                        guards: definition.guards.into(),
                        before: definition.before.into(),
                        after: definition.after.into(),
                        command: None,
                        default_block: definition.default_block,
                    }));
                }
                Selector::Command(command) => {
                    command.validate().map_err(BuildError::InvalidRoute)?;
                    let specs = command.fast_route_keys().map_or_else(
                        || vec![RouteSpec::Event(EventType::Message)],
                        |keys| keys.into_iter().map(RouteSpec::Command).collect(),
                    );
                    for spec in specs {
                        output.push(Arc::new(ModuleHandler {
                            spec,
                            event_kind: DispatchKind::Message,
                            scope: definition.scope.clone(),
                            endpoint: Arc::clone(&endpoint),
                            guards: definition.guards.clone().into(),
                            before: definition.before.clone().into(),
                            after: definition.after.clone().into(),
                            command: Some(command.clone()),
                            default_block: definition.default_block,
                        }));
                    }
                }
            }
        }
        Ok(output)
    }

    fn restrict_scope(&mut self, restriction: &RouteScope) {
        for handler in &mut self.handlers {
            match handler.scope.intersect(restriction) {
                Ok(scope) => handler.scope = scope,
                Err(error) => self.scope_errors.push(error),
            }
        }
        match self.default_scope.intersect(restriction) {
            Ok(scope) => self.default_scope = scope,
            Err(error) => self.scope_errors.push(error),
        }
    }

    fn bake_hooks(&mut self) {
        if self.guards.is_empty() && self.before.is_empty() && self.after.is_empty() {
            return;
        }
        for handler in &mut self.handlers {
            if !self.guards.is_empty() {
                let mut guards = self.guards.clone();
                guards.append(&mut handler.guards);
                handler.guards = guards;
            }
            if !self.before.is_empty() {
                let mut before = self.before.clone();
                before.append(&mut handler.before);
                handler.before = before;
            }
            if !self.after.is_empty() {
                handler.after.extend(self.after.iter().cloned());
            }
        }
        self.guards.clear();
        self.before.clear();
        self.after.clear();
    }
}

enum Selector {
    Event {
        event_type: EventType,
        kind: DispatchKind,
    },
    Command(Command),
    Interaction(Arc<str>),
    Native(Arc<str>),
}

enum EndpointKind<S>
where
    S: Send + Sync + 'static,
{
    Handler(Arc<dyn Endpoint<S>>),
    Help,
}

struct HandlerDefinition<S>
where
    S: Send + Sync + 'static,
{
    selector: Selector,
    endpoint: EndpointKind<S>,
    guards: Vec<SharedGuard<S>>,
    before: Vec<SharedBefore<S>>,
    after: Vec<SharedAfter<S>>,
    scope: RouteScope,
    default_block: bool,
}

struct ModuleHandler<S>
where
    S: Send + Sync + 'static,
{
    spec: RouteSpec,
    event_kind: DispatchKind,
    scope: RouteScope,
    endpoint: Arc<dyn Endpoint<S>>,
    guards: Arc<[SharedGuard<S>]>,
    before: Arc<[SharedBefore<S>]>,
    after: Arc<[SharedAfter<S>]>,
    command: Option<Command>,
    default_block: bool,
}

impl<S> ErasedHandler<S> for ModuleHandler<S>
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

    fn default_block(&self) -> bool {
        self.default_block
    }

    fn call(
        &self,
        event: Arc<DispatchEnvelope>,
        state: Arc<S>,
        bot: BotHandle,
        sessions: SessionRegistry,
        shutdown: ShutdownSignal,
    ) -> BoxFuture<'static, HandlerResult<Outcome>> {
        let endpoint = Arc::clone(&self.endpoint);
        let guards = Arc::clone(&self.guards);
        let before = Arc::clone(&self.before);
        let after = Arc::clone(&self.after);
        let command = self.command.clone();

        Box::pin(async move {
            let command = if let Some(command) = command {
                let Some(message) = event.event_as::<tags::Message>() else {
                    return Ok(Outcome::continue_());
                };
                let Some(result) = command.match_event(message) else {
                    return Ok(Outcome::continue_());
                };
                Some(result)
            } else {
                None
            };

            let mut context = Context::new(event, state, bot, sessions, shutdown, command);

            // Admission runs before interactive completion. An unauthorized or
            // rate-limited user must never be prompted for missing arguments.
            for guard in guards.iter() {
                match guard.check(context.clone()).await? {
                    GuardDecision::Allow => {}
                    GuardDecision::Skip => return Ok(Outcome::continue_()),
                    GuardDecision::Deny(outcome) => {
                        return Ok(outcome.with_propagation(crate::Propagation::Stop));
                    }
                }
            }

            if context.command().is_some() {
                context = prepare_command(context).await?;
            }
            for hook in before.iter() {
                hook.call(context.clone()).await?;
            }

            let mut outcome = endpoint.call(context.clone()).await?;
            for hook in after.iter() {
                outcome = hook.call(context.clone(), outcome).await?;
            }
            Ok(outcome)
        })
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
    fn call(&self, context: Context<S>) -> BoxFuture<'static, HandlerResult<Outcome>> {
        let catalog = Arc::clone(&self.catalog);
        let query = context
            .command()
            .map(|command| command.text_values().collect::<Vec<_>>().join(" "))
            .filter(|query| !query.is_empty());
        Box::pin(async move { Ok(Outcome::new().text(catalog.render(query.as_deref()))) })
    }
}

async fn prepare_command<S>(mut context: Context<S>) -> Result<Context<S>, HandlerError>
where
    S: Send + Sync + 'static,
{
    let Some(mut result) = context.command().cloned() else {
        return Ok(context);
    };
    let Some(schema) = result.command().schema_ref().cloned() else {
        return Ok(context);
    };

    match result.parse_with(&schema) {
        Ok(_) => {
            context.command = Some(result);
            return Ok(context);
        }
        Err(error) if error.missing_prompt().is_none() => {
            return Err(command_user_error(&result, error));
        }
        Err(error) if result.command().completion_ref().is_none() => {
            return Err(command_user_error(&result, error));
        }
        Err(_) => {}
    }

    let completion = result
        .command()
        .completion_ref()
        .cloned()
        .expect("missing arguments reach completion only when it is configured");
    let dialogue = Dialogue::extract(&context)
        .map_err(|error| HandlerError::user(error.to_string()))?
        .timeout(completion.timeout)
        .namespace(format!("command:{}", result.command().name()))?;

    for _ in 0..completion.max_rounds {
        let error = match result.parse_with(&schema) {
            Ok(_) => {
                context.command = Some(result);
                return Ok(context);
            }
            Err(error) => error,
        };
        let Some(prompt) = error.missing_prompt() else {
            return Err(command_user_error(&result, error));
        };
        let missing_name = error
            .missing_name()
            .expect("a missing prompt always belongs to a missing argument")
            .to_owned();
        let response = dialogue
            .ask_message(prompt)
            .await
            .map_err(|error| HandlerError::user(format!("命令补全失败：{error}")))?;
        let Event::MessageEvent(message) = response.event() else {
            return Err(HandlerError::user("命令补全需要一条消息回复"));
        };
        let raw_text = message.message.get_raw_text();
        if completion.is_cancelled(&raw_text) {
            return Err(command_user_error(&result, CommandParseError::Cancelled));
        }
        let values = tokenize_segments(&message.message.segments)
            .map_err(|error| command_user_error(&result, error))?;
        result = result.with_answer(&schema, &missing_name, values);
    }

    match result.parse_with(&schema) {
        Ok(_) => {
            context.command = Some(result);
            Ok(context)
        }
        Err(error) if error.missing_prompt().is_some() => Err(command_user_error(
            &result,
            CommandParseError::CompletionExhausted,
        )),
        Err(error) => Err(command_user_error(&result, error)),
    }
}

fn command_user_error(result: &CommandResult, error: CommandParseError) -> HandlerError {
    HandlerError::user(format!("{error}\n\n用法：{}", result.command().usage()))
}
