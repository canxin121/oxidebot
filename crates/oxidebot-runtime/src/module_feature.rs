//! One-handler feature construction and generated-feature installation.

use super::*;

impl<S> Feature<S>
where
    S: Send + Sync + 'static,
{
    fn new(definition: HandlerDefinition<S>) -> Self {
        Self {
            definition,
            completers: Vec::new(),
            errors: Vec::new(),
            runtime_shortcuts: false,
        }
    }

    /// Creates a feature that handles one exact canonical event type.
    #[must_use]
    pub fn event<E, H, T>(_event: E, handler: H) -> Self
    where
        E: EventTag,
        H: IntoHandler<T, S>,
    {
        Self::new(HandlerDefinition {
            selector: Selector::Event {
                event_type: E::TYPE,
                kind: E::TYPE.dispatch_kind(),
            },
            endpoint: EndpointKind::Handler(handler.into_endpoint()),
            guards: Vec::new(),
            before: Vec::new(),
            after: Vec::new(),
            scope: RouteScope::global(),
            default_block: false,
            branch_filter: None,
        })
    }

    /// Creates a feature for ordinary message events.
    #[must_use]
    pub fn message<H, T>(handler: H) -> Self
    where
        H: IntoHandler<T, S>,
    {
        Self::event(tags::Message, handler)
    }

    /// Creates a feature for one command definition.
    #[must_use]
    pub fn command<H, T>(command: Command, handler: H) -> Self
    where
        H: IntoHandler<T, S>,
    {
        Self::new(HandlerDefinition {
            selector: Selector::Command(command),
            endpoint: EndpointKind::Handler(handler.into_endpoint()),
            guards: Vec::new(),
            before: Vec::new(),
            after: Vec::new(),
            scope: RouteScope::global(),
            default_block: true,
            branch_filter: None,
        })
    }

    /// Creates a feature for a generated command tree.
    #[must_use]
    pub fn command_tree<C, H, T>(handler: H) -> Self
    where
        C: crate::command::CommandTree,
        H: IntoHandler<T, S>,
    {
        Self::command(C::command(), handler)
    }

    /// Creates a feature for a generated command-tree branch.
    #[must_use]
    pub fn command_branch<B, H, T>(_branch: B, handler: H) -> Self
    where
        B: crate::CommandBranchTag,
        H: IntoHandler<T, S>,
    {
        Self::new(HandlerDefinition {
            selector: Selector::Command(<B::Command as crate::command::CommandTree>::command()),
            endpoint: EndpointKind::Handler(handler.into_endpoint()),
            guards: Vec::new(),
            before: Vec::new(),
            after: Vec::new(),
            scope: RouteScope::global(),
            default_block: true,
            branch_filter: Some((
                B::PATH.iter().map(|value| Arc::from(*value)).collect(),
                B::MATCH_DESCENDANTS,
            )),
        })
    }

    /// Creates a feature for one interaction custom ID.
    #[must_use]
    pub fn interaction<H, T>(custom_id: impl Into<Arc<str>>, handler: H) -> Self
    where
        H: IntoHandler<T, S>,
    {
        Self::new(HandlerDefinition {
            selector: Selector::Interaction(custom_id.into()),
            endpoint: EndpointKind::Handler(handler.into_endpoint()),
            guards: Vec::new(),
            before: Vec::new(),
            after: Vec::new(),
            scope: RouteScope::global(),
            default_block: true,
            branch_filter: None,
        })
    }

    /// Creates a feature for one adapter-native event kind.
    #[must_use]
    pub fn native<H, T>(kind: impl Into<Arc<str>>, handler: H) -> Self
    where
        H: IntoHandler<T, S>,
    {
        Self::new(HandlerDefinition {
            selector: Selector::Native(kind.into()),
            endpoint: EndpointKind::Handler(handler.into_endpoint()),
            guards: Vec::new(),
            before: Vec::new(),
            after: Vec::new(),
            scope: RouteScope::global(),
            default_block: false,
            branch_filter: None,
        })
    }

    /// Adds a guard that runs before this feature's handler.
    #[must_use]
    pub fn guard<G>(mut self, guard: G) -> Self
    where
        G: Guard<S>,
    {
        self.definition.guards.push(Arc::new(guard));
        self
    }

    /// Adds a hook that runs before this feature's handler.
    #[must_use]
    pub fn before<B>(mut self, hook: B) -> Self
    where
        B: Before<S>,
    {
        self.definition.before.push(Arc::new(hook));
        self
    }

    /// Adds a hook that runs after this feature's handler.
    #[must_use]
    pub fn after<A>(mut self, hook: A) -> Self
    where
        A: After<S>,
    {
        self.definition.after.push(Arc::new(hook));
        self
    }

    /// Restricts this feature to one platform.
    #[must_use]
    pub fn for_platform(mut self, platform: PlatformId) -> Self {
        let restriction = RouteScope::for_platform(platform);
        match self.definition.scope.intersect(&restriction) {
            Ok(scope) => self.definition.scope = scope,
            Err(error) => self.errors.push(error),
        }
        self
    }

    /// Restricts this feature to one bot.
    #[must_use]
    pub fn for_bot(mut self, bot: BotIdentity) -> Self {
        let restriction = RouteScope::for_bot(bot);
        match self.definition.scope.intersect(&restriction) {
            Ok(scope) => self.definition.scope = scope,
            Err(error) => self.errors.push(error),
        }
        self
    }

    /// Sets whether successful handling stops further route matching.
    #[must_use]
    pub const fn block(mut self, block: bool) -> Self {
        self.definition.default_block = block;
        self
    }

    /// Lets routing continue after successful handling.
    #[must_use]
    pub const fn continue_after(self) -> Self {
        self.block(false)
    }

    /// Stops routing after successful handling.
    #[must_use]
    pub const fn stop_after(self) -> Self {
        self.block(true)
    }

    /// Adds a static shortcut to this command feature.
    #[must_use]
    pub fn shortcut(mut self, shortcut: Shortcut) -> Self {
        match &mut self.definition.selector {
            Selector::Command(command) => *command = command.clone().shortcut(shortcut),
            _ => self
                .errors
                .push("shortcuts can only be attached to command features".into()),
        }
        self
    }

    /// Enables the bounded runtime shortcut registry for this command.
    #[must_use]
    pub const fn runtime_shortcuts(mut self) -> Self {
        self.runtime_shortcuts = true;
        self
    }

    /// Adds declarative completion metadata to this command feature.
    #[must_use]
    pub fn completion(mut self, completion: CompletionConfig) -> Self {
        match &mut self.definition.selector {
            Selector::Command(command) => *command = command.clone().completion(completion),
            _ => self
                .errors
                .push("completion can only be attached to command features".into()),
        }
        self
    }

    /// Attaches a dynamic completion provider to a named command field.
    #[must_use]
    pub fn complete<F, C>(mut self, _field: F, provider: C) -> Self
    where
        F: CommandFieldTag,
        C: DynamicCompleter<S>,
    {
        let branch = self
            .definition
            .branch_filter
            .as_ref()
            .map_or(&[][..], |(path, _)| path.as_ref());
        let branch_names = branch.iter().map(AsRef::as_ref).collect::<Vec<_>>();
        match &self.definition.selector {
            Selector::Command(command) => match command.field_id(&branch_names, F::NAME) {
                Some(field) => self.completers.push(CompleterBinding {
                    command: command.id(),
                    field,
                    provider: Arc::new(provider),
                }),
                None => self.errors.push(format!(
                    "command `{}` has no field `{}` on branch `{}`",
                    command.name(),
                    F::NAME,
                    branch_names.join(" "),
                )),
            },
            _ => self
                .errors
                .push("dynamic completion can only be attached to command features".into()),
        }
        self
    }

    /// Converts this one-handler authoring value into an ordinary flat module.
    #[must_use]
    pub fn into_module(self) -> Module<S> {
        let mut module = Module::new();
        module.handlers.push(self.definition);
        module.scope_errors.extend(self.errors);
        module.runtime_shortcuts = self.runtime_shortcuts;
        for binding in self.completers {
            if module
                .completers
                .insert((binding.command, binding.field), binding.provider)
                .is_some()
            {
                module.scope_errors.push(format!(
                    "more than one dynamic completer is registered for command {:016x}, field {:08x}",
                    binding.command.0,
                    binding.field.0,
                ));
            }
        }
        module
    }
}

impl<S> IntoFeature<S> for Feature<S>
where
    S: Send + Sync + 'static,
{
    fn install(self, module: Module<S>) -> Module<S> {
        module.include(self.into_module())
    }
}

impl<S> IntoFeature<S> for Module<S>
where
    S: Send + Sync + 'static,
{
    fn install(self, module: Module<S>) -> Module<S> {
        module.include(self)
    }
}
