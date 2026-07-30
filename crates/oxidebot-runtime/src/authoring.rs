use crate::{
    BotDirectory, BotHandle, Command, CommandFieldId, CommandId, CommandMatch, CommandOutput,
    CommandParseError, CommandRenderer, CompletionItem, Context, Extract, ExtractError,
    FromCommandMatch, FromCommandValue, Guard, GuardDecision, HandlerError, HandlerResult,
    Shortcut, MAX_REGISTRY_SHORTCUTS, MAX_SHORTCUT_SCAN_PER_MESSAGE,
};
use async_trait::async_trait;
use futures_util::{future::BoxFuture, FutureExt};
use oxidebot_core::{
    conversation::MessageTarget,
    source::message::{DeliveryPlan, DeliveryReport, FallbackPolicy, Message},
    LocalizedMessage, TemplateValue, TranslationCatalog,
};
use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    ops::Deref,
    panic::AssertUnwindSafe,
    sync::{Arc, RwLock},
};

/// Input passed to a [`CommandRewriter`].
#[derive(Clone, Debug)]
pub struct RewriteInput {
    /// Canonical message whose command text may be rewritten.
    pub message: Message,
    /// Resolved locale for this command invocation, when available.
    pub locale: Option<Arc<str>>,
}

/// Asynchronously rewrites a message before command parsing.
#[async_trait]
pub trait CommandRewriter<S>: Send + Sync + 'static
where
    S: Send + Sync + 'static,
{
    /// Rewrites `input` for the current handler context.
    async fn rewrite(
        &self,
        context: &Context<S>,
        input: RewriteInput,
    ) -> HandlerResult<RewriteInput>;
}

#[async_trait]
impl<S, F, Fut> CommandRewriter<S> for F
where
    S: Send + Sync + 'static,
    F: Fn(Context<S>, RewriteInput) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = HandlerResult<RewriteInput>> + Send,
{
    async fn rewrite(
        &self,
        context: &Context<S>,
        input: RewriteInput,
    ) -> HandlerResult<RewriteInput> {
        (self)(context.clone(), input).await
    }
}

/// Input passed to a [`DynamicCompleter`].
#[derive(Clone, Debug)]
pub struct CompletionInput {
    /// Command whose argument is being completed.
    pub command: Command,
    /// Stable ID of the field requesting completion.
    pub field: CommandFieldId,
    /// Partial text currently supplied for the field.
    pub partial: Arc<str>,
    /// Source span to replace when inserting a completion.
    pub replace: crate::SourceSpan,
    /// Resolved locale for this completion request, when available.
    pub locale: Option<Arc<str>>,
    /// Maximum number of completion items requested.
    pub limit: usize,
}

/// Produces completion items for a specific command field.
#[async_trait]
pub trait DynamicCompleter<S>: Send + Sync + 'static
where
    S: Send + Sync + 'static,
{
    /// Computes bounded suggestions for `input` in the current handler context.
    async fn complete(
        &self,
        context: &Context<S>,
        input: CompletionInput,
    ) -> HandlerResult<Vec<CompletionItem>>;
}

#[async_trait]
impl<S, F, Fut> DynamicCompleter<S> for F
where
    S: Send + Sync + 'static,
    F: Fn(Context<S>, CompletionInput) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = HandlerResult<Vec<CompletionItem>>> + Send,
{
    async fn complete(
        &self,
        context: &Context<S>,
        input: CompletionInput,
    ) -> HandlerResult<Vec<CompletionItem>> {
        (self)(context.clone(), input).await
    }
}

/// Resolves the locale used for localized command presentation and parsing.
#[async_trait]
pub trait LocaleResolver<S>: Send + Sync + 'static
where
    S: Send + Sync + 'static,
{
    /// Resolves a locale for `context`, or leaves it unspecified.
    async fn resolve(&self, context: &Context<S>) -> Option<Arc<str>>;
}

#[async_trait]
impl<S, F, Fut> LocaleResolver<S> for F
where
    S: Send + Sync + 'static,
    F: Fn(Context<S>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Option<Arc<str>>> + Send,
{
    async fn resolve(&self, context: &Context<S>) -> Option<Arc<str>> {
        (self)(context.clone()).await
    }
}

/// Default locale resolver that reads command and interaction locale metadata.
#[derive(Default)]
pub struct EventLocaleResolver;

#[async_trait]
impl<S> LocaleResolver<S> for EventLocaleResolver
where
    S: Send + Sync + 'static,
{
    async fn resolve(&self, context: &Context<S>) -> Option<Arc<str>> {
        context
            .command()
            .and_then(CommandMatch::locale)
            .map(Arc::from)
            .or_else(|| match context.event() {
                oxidebot_core::Event::Interaction(event) => event.locale.clone().map(Arc::from),
                _ => None,
            })
    }
}

/// Normalizes a portable message before it is rendered or delivered.
#[async_trait]
pub trait MessageNormalizer<S>: Send + Sync + 'static
where
    S: Send + Sync + 'static,
{
    /// Normalizes `message` in the current handler context.
    async fn normalize(&self, context: &Context<S>, message: Message) -> HandlerResult<Message>;
}

#[async_trait]
impl<S, F, Fut> MessageNormalizer<S> for F
where
    S: Send + Sync + 'static,
    F: Fn(Context<S>, Message) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = HandlerResult<Message>> + Send,
{
    async fn normalize(&self, context: &Context<S>, message: Message) -> HandlerResult<Message> {
        (self)(context.clone(), message).await
    }
}

/// Transforms a parsed command match before its handler is invoked.
#[async_trait]
pub trait CommandMiddleware<S>: Send + Sync + 'static
where
    S: Send + Sync + 'static,
{
    /// Receives a successfully parsed command and may transform or reject it.
    async fn after_parse(
        &self,
        context: &Context<S>,
        command: CommandMatch,
    ) -> HandlerResult<CommandMatch>;
}

#[async_trait]
impl<S, F, Fut> CommandMiddleware<S> for F
where
    S: Send + Sync + 'static,
    F: Fn(Context<S>, CommandMatch) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = HandlerResult<CommandMatch>> + Send,
{
    async fn after_parse(
        &self,
        context: &Context<S>,
        command: CommandMatch,
    ) -> HandlerResult<CommandMatch> {
        (self)(context.clone(), command).await
    }
}

/// Transforms command output before it becomes delivery effects.
#[async_trait]
pub trait CommandOutputMiddleware<S>: Send + Sync + 'static
where
    S: Send + Sync + 'static,
{
    /// Transforms `output` for the current command handler.
    async fn transform(
        &self,
        context: &Context<S>,
        output: CommandOutput,
    ) -> HandlerResult<CommandOutput>;
}

#[async_trait]
impl<S, F, Fut> CommandOutputMiddleware<S> for F
where
    S: Send + Sync + 'static,
    F: Fn(Context<S>, CommandOutput) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = HandlerResult<CommandOutput>> + Send,
{
    async fn transform(
        &self,
        context: &Context<S>,
        output: CommandOutput,
    ) -> HandlerResult<CommandOutput> {
        (self)(context.clone(), output).await
    }
}

/// Observes or transforms a capability-planned outbound delivery.
#[async_trait]
pub trait DeliveryMiddleware<S>: Send + Sync + 'static
where
    S: Send + Sync + 'static,
{
    /// Runs after planning and before the bounded adapter delivery begins.
    async fn before_delivery(
        &self,
        context: Option<&Context<S>>,
        target: &MessageTarget,
        plan: DeliveryPlan,
    ) -> HandlerResult<DeliveryPlan>;

    /// Runs after delivery and may transform the resulting report.
    async fn after_delivery(
        &self,
        _context: Option<&Context<S>>,
        _target: &MessageTarget,
        report: DeliveryReport,
    ) -> HandlerResult<DeliveryReport> {
        Ok(report)
    }
}

#[async_trait]
impl<S, F, Fut> DeliveryMiddleware<S> for F
where
    S: Send + Sync + 'static,
    F: Fn(Option<Context<S>>, MessageTarget, DeliveryPlan) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = HandlerResult<DeliveryPlan>> + Send,
{
    async fn before_delivery(
        &self,
        context: Option<&Context<S>>,
        target: &MessageTarget,
        plan: DeliveryPlan,
    ) -> HandlerResult<DeliveryPlan> {
        (self)(context.cloned(), target.clone(), plan).await
    }
}

#[derive(Clone)]
pub(crate) struct AuthoringRuntime<S>
where
    S: Send + Sync + 'static,
{
    pub renderer: Arc<dyn CommandRenderer>,
    pub locale_resolver: Arc<dyn LocaleResolver<S>>,
    pub registry: CommandRegistry,
    pub rewriters: Vec<Arc<dyn CommandRewriter<S>>>,
    pub normalizers: Vec<Arc<dyn MessageNormalizer<S>>>,
    pub command_middleware: Vec<Arc<dyn CommandMiddleware<S>>>,
    pub output_middleware: Vec<Arc<dyn CommandOutputMiddleware<S>>>,
    pub delivery_middleware: Vec<Arc<dyn DeliveryMiddleware<S>>>,
    pub completers: HashMap<(CommandId, CommandFieldId), Arc<dyn DynamicCompleter<S>>>,
    pub translations: Option<TranslationCatalog>,
    bots: Arc<RwLock<Option<BotDirectory>>>,
}

impl<S> Default for AuthoringRuntime<S>
where
    S: Send + Sync + 'static,
{
    fn default() -> Self {
        Self {
            renderer: Arc::new(crate::DefaultCommandRenderer),
            locale_resolver: Arc::new(EventLocaleResolver),
            registry: CommandRegistry::default(),
            rewriters: Vec::new(),
            normalizers: Vec::new(),
            command_middleware: Vec::new(),
            output_middleware: Vec::new(),
            delivery_middleware: Vec::new(),
            completers: HashMap::new(),
            translations: None,
            bots: Arc::new(RwLock::new(None)),
        }
    }
}

impl<S> AuthoringRuntime<S>
where
    S: Send + Sync + 'static,
{
    pub async fn locale(&self, context: &Context<S>) -> Option<Arc<str>> {
        self.locale_resolver.resolve(context).await
    }

    pub(crate) fn attach_bots(&self, bots: BotDirectory) {
        *self
            .bots
            .write()
            .expect("authoring bot directory lock poisoned") = Some(bots);
    }

    pub(crate) fn detach_bots(&self) {
        *self
            .bots
            .write()
            .expect("authoring bot directory lock poisoned") = None;
    }

    pub(crate) fn bots(&self) -> Option<BotDirectory> {
        self.bots
            .read()
            .expect("authoring bot directory lock poisoned")
            .clone()
    }

    pub async fn render(
        &self,
        context: &Context<S>,
        mut output: CommandOutput,
    ) -> HandlerResult<Message> {
        for middleware in self.output_middleware.iter() {
            output = middleware.transform(context, output).await?;
        }
        let locale = self.locale(context).await;
        Ok(self.renderer.render(&output, locale.as_deref()))
    }

    pub async fn apply_command_middleware(
        &self,
        context: &Context<S>,
        mut command: CommandMatch,
    ) -> HandlerResult<CommandMatch> {
        for middleware in self.command_middleware.iter() {
            command = middleware.after_parse(context, command).await?;
        }
        Ok(command)
    }

    pub async fn complete(
        &self,
        context: &Context<S>,
        input: CompletionInput,
    ) -> HandlerResult<Vec<CompletionItem>> {
        let Some(provider) = self.completers.get(&(input.command.id(), input.field)) else {
            return Ok(Vec::new());
        };
        let mut values = provider.complete(context, input.clone()).await?;
        values.truncate(input.limit);
        Ok(values)
    }

    pub async fn deliver(
        &self,
        context: Option<&Context<S>>,
        bot: &BotHandle,
        target: MessageTarget,
        mut message: Message,
        policy: FallbackPolicy,
    ) -> HandlerResult<DeliveryReport> {
        if message.options.idempotency_key.is_none()
            && bot.capabilities().send_idempotency != crate::IdempotencyGuarantee::Unsupported
        {
            if let Some(context) = context {
                use std::hash::{Hash, Hasher};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                context.dispatch_envelope().id.hash(&mut hasher);
                let sequence = context.next_outbound_sequence();
                message.options.idempotency_key = Some(format!(
                    "oxidebot-event-{:016x}-{sequence}",
                    hasher.finish()
                ));
            }
        }
        let mut plan = bot
            .plan_outgoing_message(&target, &message, policy)
            .map_err(HandlerError::from)?;
        for middleware in &self.delivery_middleware {
            plan = AssertUnwindSafe(middleware.before_delivery(context, &target, plan))
                .catch_unwind()
                .await
                .map_err(|_| HandlerError::DeliveryPanicked)??;
        }
        let mut report = bot
            .send_delivery_plan(target.clone(), plan)
            .await
            .map_err(HandlerError::from)?;
        for middleware in self.delivery_middleware.iter().rev() {
            report = AssertUnwindSafe(middleware.after_delivery(context, &target, report))
                .catch_unwind()
                .await
                .map_err(|_| HandlerError::DeliveryPanicked)??;
        }
        Ok(report)
    }
}

#[async_trait]
pub(crate) trait ErasedDeliveryPipeline: Send + Sync + 'static {
    async fn deliver(
        &self,
        bot: &BotHandle,
        target: MessageTarget,
        message: Message,
        policy: FallbackPolicy,
    ) -> HandlerResult<DeliveryReport>;
}

pub(crate) struct BoundDeliveryPipeline<S>
where
    S: Send + Sync + 'static,
{
    pub runtime: Arc<AuthoringRuntime<S>>,
    pub context: Context<S>,
}

#[async_trait]
impl<S> ErasedDeliveryPipeline for BoundDeliveryPipeline<S>
where
    S: Send + Sync + 'static,
{
    async fn deliver(
        &self,
        bot: &BotHandle,
        target: MessageTarget,
        message: Message,
        policy: FallbackPolicy,
    ) -> HandlerResult<DeliveryReport> {
        self.runtime
            .deliver(Some(&self.context), bot, target, message, policy)
            .await
    }
}

/// Handler-local localization facade backed by the application's bounded
/// [`TranslationCatalog`] and locale resolver.
#[derive(Clone)]
pub struct I18n {
    inner: Arc<dyn ErasedI18n>,
}

#[async_trait]
trait ErasedI18n: Send + Sync + 'static {
    async fn render(&self, message: LocalizedMessage) -> HandlerResult<Message>;
}

struct BoundI18n<S>
where
    S: Send + Sync + 'static,
{
    context: Context<S>,
}

#[async_trait]
impl<S> ErasedI18n for BoundI18n<S>
where
    S: Send + Sync + 'static,
{
    async fn render(&self, message: LocalizedMessage) -> HandlerResult<Message> {
        let catalog = self
            .context
            .authoring()
            .translations
            .as_ref()
            .ok_or_else(|| HandlerError::internal("no TranslationCatalog is configured"))?;
        let locale = self.context.authoring().locale(&self.context).await;
        message
            .render(catalog, locale.as_deref())
            .map_err(|error| HandlerError::internal(error.to_string()))
    }
}

impl I18n {
    /// Starts an awaitable localized message identified by `key`.
    #[must_use]
    pub fn message(&self, key: impl Into<Arc<str>>) -> I18nMessage {
        I18nMessage {
            i18n: self.clone(),
            message: LocalizedMessage::new(key),
        }
    }

    /// Renders a localized message using the current handler locale.
    pub async fn render(&self, message: LocalizedMessage) -> HandlerResult<Message> {
        self.inner.render(message).await
    }
}

impl std::fmt::Debug for I18n {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("I18n").finish_non_exhaustive()
    }
}

/// Awaitable localized message builder.
#[derive(Clone)]
pub struct I18nMessage {
    i18n: I18n,
    message: LocalizedMessage,
}

impl I18nMessage {
    /// Supplies one template argument.
    #[must_use]
    pub fn arg(mut self, name: impl Into<Arc<str>>, value: impl Into<TemplateValue>) -> Self {
        self.message = self.message.arg(name, value);
        self
    }

    /// Renders this localized message with its accumulated arguments.
    pub async fn render(self) -> HandlerResult<Message> {
        self.i18n.render(self.message).await
    }
}

impl std::future::IntoFuture for I18nMessage {
    type Output = HandlerResult<Message>;
    type IntoFuture = BoxFuture<'static, Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.render())
    }
}

impl<S> Extract<S> for I18n
where
    S: Send + Sync + 'static,
{
    fn extract(context: &Context<S>) -> Result<Self, ExtractError> {
        Ok(Self {
            inner: Arc::new(BoundI18n {
                context: context.clone(),
            }),
        })
    }
}

/// Bounded, mutable state for runtime command enablement and shortcuts.
#[derive(Clone, Debug)]
pub struct CommandRegistry {
    inner: Arc<RwLock<CommandRegistryState>>,
    publication: Arc<RwLock<Option<CommandPublicationState>>>,
    publication_gate: Arc<tokio::sync::Mutex<()>>,
}

#[derive(Clone, Debug)]
struct CommandPublicationState {
    bots: BotDirectory,
    catalog: crate::CommandCatalog,
    published_revision: Option<u64>,
    pending_revision: Option<u64>,
    last_error: Option<Arc<str>>,
}

/// Snapshot of local-to-platform command-definition synchronization.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CommandPublicationStatus {
    /// Current local command-registry revision requested for publication.
    pub desired_revision: u64,
    /// Most recently published revision, if synchronization has succeeded.
    pub published_revision: Option<u64>,
    /// Revision currently waiting to be published, if any.
    pub pending_revision: Option<u64>,
    /// Last publication failure, if one occurred.
    pub last_error: Option<Arc<str>>,
    /// Whether this registry is attached to connected bots for publication.
    pub attached: bool,
}

#[derive(Debug)]
struct CommandRegistryState {
    capacity: usize,
    max_bytes: usize,
    retained_bytes: usize,
    shortcut_count: usize,
    commands: HashMap<CommandId, RuntimeCommandState>,
    order: VecDeque<CommandId>,
    shortcut_order: VecDeque<CommandId>,
    revision: u64,
    dynamic_shortcuts: bool,
    broad_command_matching: bool,
}

/// Current runtime state for a registered command.
#[derive(Clone, Debug)]
pub struct RuntimeCommandState {
    /// Canonical command name.
    pub name: Arc<str>,
    /// Whether dispatch currently permits this command.
    pub enabled: bool,
    schema_fingerprint: u64,
    /// Shortcuts compiled with the command definition. They are immutable at runtime.
    pub static_shortcuts: Vec<Shortcut>,
    /// Bounded shortcuts added through the runtime command registry.
    pub shortcuts: Vec<Shortcut>,
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::bounded(16_384)
    }
}

impl CommandRegistry {
    /// Creates a registry bounded by the number of commands it retains.
    #[must_use]
    pub fn bounded(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        let max_bytes = capacity.saturating_mul(32 * 1024).min(512 * 1024 * 1024);
        Self::bounded_bytes(capacity, max_bytes)
    }

    /// Creates a registry bounded by command count, total shortcut count, and
    /// retained command/shortcut bytes.
    #[must_use]
    pub fn bounded_bytes(capacity: usize, max_bytes: usize) -> Self {
        Self {
            inner: Arc::new(RwLock::new(CommandRegistryState {
                capacity: capacity.max(1),
                max_bytes: max_bytes.max(1),
                retained_bytes: 0,
                shortcut_count: 0,
                commands: HashMap::new(),
                order: VecDeque::new(),
                shortcut_order: VecDeque::new(),
                revision: 0,
                dynamic_shortcuts: false,
                broad_command_matching: false,
            })),
            publication: Arc::new(RwLock::new(None)),
            publication_gate: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// Registers static metadata for a command and initializes its runtime state.
    pub fn register(&self, command: &Command) -> Result<(), HandlerError> {
        let mut state = self.inner.write().expect("command registry lock poisoned");
        for shortcut in command.shortcuts() {
            shortcut.validate()?;
        }
        let schema_fingerprint = command.structural_fingerprint();
        if let Some(existing) = state.commands.get(&command.id()) {
            if existing.name.as_ref() != command.name()
                || existing.schema_fingerprint != schema_fingerprint
            {
                return Err(HandlerError::internal(format!(
                    "command ID collision or incompatible grammar between `{}` and `{}`",
                    existing.name,
                    command.name(),
                )));
            }
            let additions = command
                .shortcuts()
                .iter()
                .filter(|shortcut| {
                    !existing.static_shortcuts.iter().any(|current| {
                        current.is_regex() == shortcut.is_regex()
                            && current.pattern_text() == shortcut.pattern_text()
                    })
                })
                .cloned()
                .collect::<Vec<_>>();
            for shortcut in &additions {
                if state.commands.iter().any(|(registered_id, registered)| {
                    *registered_id != command.id()
                        && registered
                            .static_shortcuts
                            .iter()
                            .chain(registered.shortcuts.iter())
                            .any(|current| {
                                current.is_regex() == shortcut.is_regex()
                                    && current.pattern_text() == shortcut.pattern_text()
                            })
                }) {
                    return Err(HandlerError::user(
                        "a static shortcut conflicts with an already registered command",
                    ));
                }
            }
            if !additions.is_empty() {
                let additions_len = additions.len();
                if existing
                    .static_shortcuts
                    .len()
                    .saturating_add(additions.len())
                    > 256
                {
                    return Err(HandlerError::internal("shortcut limit exceeded"));
                }
                let added_bytes = additions
                    .iter()
                    .map(Shortcut::retained_bytes)
                    .sum::<usize>();
                if state.shortcut_count.saturating_add(additions.len()) > MAX_REGISTRY_SHORTCUTS {
                    return Err(HandlerError::internal("global shortcut capacity exceeded"));
                }
                if state.retained_bytes.saturating_add(added_bytes) > state.max_bytes {
                    return Err(HandlerError::internal(
                        "command registry byte capacity exceeded",
                    ));
                }
                let had_shortcuts =
                    !existing.static_shortcuts.is_empty() || !existing.shortcuts.is_empty();
                state
                    .commands
                    .get_mut(&command.id())
                    .expect("command existence checked above")
                    .static_shortcuts
                    .extend(additions);
                state.shortcut_count = state.shortcut_count.saturating_add(additions_len);
                state.retained_bytes = state.retained_bytes.saturating_add(added_bytes);
                if !had_shortcuts {
                    state.shortcut_order.push_back(command.id());
                }
                state.revision = state.revision.wrapping_add(1);
            }
            return Ok(());
        }
        if state.commands.len() >= state.capacity {
            return Err(HandlerError::internal("command registry capacity exceeded"));
        }
        if command.shortcuts().len() > 256 {
            return Err(HandlerError::internal("shortcut limit exceeded"));
        }
        if state
            .shortcut_count
            .saturating_add(command.shortcuts().len())
            > MAX_REGISTRY_SHORTCUTS
        {
            return Err(HandlerError::internal("global shortcut capacity exceeded"));
        }
        let added_bytes = command
            .name()
            .len()
            .saturating_add(
                command
                    .shortcuts()
                    .iter()
                    .map(Shortcut::retained_bytes)
                    .sum::<usize>(),
            )
            .saturating_add(256);
        if state.retained_bytes.saturating_add(added_bytes) > state.max_bytes {
            return Err(HandlerError::internal(
                "command registry byte capacity exceeded",
            ));
        }
        for shortcut in command.shortcuts() {
            if state.commands.values().any(|registered| {
                registered
                    .static_shortcuts
                    .iter()
                    .chain(registered.shortcuts.iter())
                    .any(|existing| {
                        existing.is_regex() == shortcut.is_regex()
                            && existing.pattern_text() == shortcut.pattern_text()
                    })
            }) {
                return Err(HandlerError::user(
                    "a static shortcut conflicts with an already registered command",
                ));
            }
        }
        state.order.push_back(command.id());
        if !command.shortcuts().is_empty() {
            state.shortcut_order.push_back(command.id());
        }
        state.commands.insert(
            command.id(),
            RuntimeCommandState {
                name: Arc::from(command.name()),
                enabled: true,
                schema_fingerprint,
                static_shortcuts: command.shortcuts().to_vec(),
                shortcuts: Vec::new(),
            },
        );
        state.shortcut_count = state
            .shortcut_count
            .saturating_add(command.shortcuts().len());
        state.retained_bytes = state.retained_bytes.saturating_add(added_bytes);
        state.revision = state.revision.wrapping_add(1);
        Ok(())
    }

    /// Finds a registered command by canonical name, case-insensitively.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<CommandId> {
        self.inner
            .read()
            .expect("command registry lock poisoned")
            .commands
            .iter()
            .find_map(|(id, command)| command.name.eq_ignore_ascii_case(name).then_some(*id))
    }

    /// Returns whether a registered command is enabled; unknown IDs are treated as enabled.
    #[must_use]
    pub fn is_enabled(&self, id: CommandId) -> bool {
        self.inner
            .read()
            .expect("command registry lock poisoned")
            .commands
            .get(&id)
            .is_none_or(|command| command.enabled)
    }

    /// Enables a command and returns whether its state changed.
    pub fn enable(&self, id: CommandId) -> bool {
        self.set_enabled(id, true)
    }

    /// Disables a command and returns whether its state changed.
    pub fn disable(&self, id: CommandId) -> bool {
        self.set_enabled(id, false)
    }

    fn set_enabled(&self, id: CommandId, enabled: bool) -> bool {
        let mut state = self.inner.write().expect("command registry lock poisoned");
        let Some(command) = state.commands.get_mut(&id) else {
            return false;
        };
        if command.enabled == enabled {
            return false;
        }
        command.enabled = enabled;
        state.revision = state.revision.wrapping_add(1);
        true
    }

    /// Adds a bounded runtime shortcut to a registered command.
    pub fn add_shortcut(&self, id: CommandId, shortcut: Shortcut) -> Result<(), HandlerError> {
        shortcut.validate()?;
        let mut state = self.inner.write().expect("command registry lock poisoned");
        let command_state = state
            .commands
            .get(&id)
            .ok_or_else(|| HandlerError::internal("unknown command id"))?;
        let existing_count = command_state.shortcuts.len();
        let had_any_shortcuts =
            !command_state.static_shortcuts.is_empty() || !command_state.shortcuts.is_empty();
        if existing_count >= 256 {
            return Err(HandlerError::internal("shortcut limit exceeded"));
        }
        if state.shortcut_count >= MAX_REGISTRY_SHORTCUTS {
            return Err(HandlerError::internal("global shortcut capacity exceeded"));
        }
        let shortcut_bytes = shortcut.retained_bytes();
        if state.retained_bytes.saturating_add(shortcut_bytes) > state.max_bytes {
            return Err(HandlerError::internal(
                "command registry byte capacity exceeded",
            ));
        }
        let duplicate = state.commands.iter().any(|(_, command)| {
            command
                .static_shortcuts
                .iter()
                .chain(command.shortcuts.iter())
                .any(|existing| {
                    existing.is_regex() == shortcut.is_regex()
                        && existing.pattern_text() == shortcut.pattern_text()
                })
        });
        if duplicate {
            return Err(HandlerError::user(
                "the shortcut conflicts with an already registered command",
            ));
        }
        if !had_any_shortcuts {
            state.shortcut_order.push_back(id);
        }
        state
            .commands
            .get_mut(&id)
            .expect("command existence checked above")
            .shortcuts
            .push(shortcut);
        state.shortcut_count = state.shortcut_count.saturating_add(1);
        state.retained_bytes = state.retained_bytes.saturating_add(shortcut_bytes);
        state.revision = state.revision.wrapping_add(1);
        Ok(())
    }

    /// Removes runtime shortcuts matching `pattern`, returning whether any were removed.
    pub fn remove_shortcut(&self, id: CommandId, pattern: &str) -> Result<bool, HandlerError> {
        let mut state = self.inner.write().expect("command registry lock poisoned");
        let (changed, removed, removed_bytes, has_any_shortcuts) = {
            let command = state
                .commands
                .get_mut(&id)
                .ok_or_else(|| HandlerError::internal("unknown command id"))?;
            let before = command.shortcuts.len();
            let removed_bytes = command
                .shortcuts
                .iter()
                .filter(|shortcut| shortcut.pattern_text() == pattern)
                .map(Shortcut::retained_bytes)
                .sum::<usize>();
            command
                .shortcuts
                .retain(|shortcut| shortcut.pattern_text() != pattern);
            let removed = before.saturating_sub(command.shortcuts.len());
            (
                removed > 0,
                removed,
                removed_bytes,
                !command.static_shortcuts.is_empty() || !command.shortcuts.is_empty(),
            )
        };
        if changed {
            state.shortcut_count = state.shortcut_count.saturating_sub(removed);
            state.retained_bytes = state.retained_bytes.saturating_sub(removed_bytes);
            if !has_any_shortcuts {
                state.shortcut_order.retain(|command_id| *command_id != id);
            }
            state.revision = state.revision.wrapping_add(1);
        }
        Ok(changed)
    }

    /// Returns up to `limit` enabled command IDs whose shortcuts match `input`.
    #[must_use]
    pub fn matching_shortcut_commands(&self, input: &str, limit: usize) -> Vec<CommandId> {
        let candidates = {
            let state = self.inner.read().expect("command registry lock poisoned");
            let mut remaining = MAX_SHORTCUT_SCAN_PER_MESSAGE;
            let mut candidates = Vec::new();
            for id in &state.shortcut_order {
                if remaining == 0 {
                    break;
                }
                let Some(command) = state.commands.get(id) else {
                    continue;
                };
                if !command.enabled {
                    continue;
                }
                let shortcuts = command
                    .static_shortcuts
                    .iter()
                    .chain(command.shortcuts.iter())
                    .take(remaining)
                    .cloned()
                    .collect::<Vec<_>>();
                remaining = remaining.saturating_sub(shortcuts.len());
                candidates.push((*id, shortcuts));
            }
            candidates
        };
        let mut matches = Vec::new();
        for (id, shortcuts) in candidates {
            if shortcuts
                .iter()
                .any(|shortcut| shortcut.rewrite(input).is_some())
            {
                matches.push(id);
                if matches.len() >= limit {
                    break;
                }
            }
        }
        matches
    }

    /// Removes all mutable shortcuts for a command and returns the number removed.
    pub fn clear_shortcuts(&self, id: CommandId) -> usize {
        let mut state = self.inner.write().expect("command registry lock poisoned");
        let Some(command) = state.commands.get_mut(&id) else {
            return 0;
        };
        let removed = command.shortcuts.len();
        let removed_bytes = command
            .shortcuts
            .iter()
            .map(Shortcut::retained_bytes)
            .sum::<usize>();
        command.shortcuts.clear();
        let has_static_shortcuts = !command.static_shortcuts.is_empty();
        if removed > 0 {
            state.shortcut_count = state.shortcut_count.saturating_sub(removed);
            state.retained_bytes = state.retained_bytes.saturating_sub(removed_bytes);
            if !has_static_shortcuts {
                state.shortcut_order.retain(|command_id| *command_id != id);
            }
            state.revision = state.revision.wrapping_add(1);
        }
        removed
    }

    /// Returns static and mutable shortcuts for a registered command.
    #[must_use]
    pub fn shortcuts(&self, id: CommandId) -> Vec<Shortcut> {
        self.inner
            .read()
            .expect("command registry lock poisoned")
            .commands
            .get(&id)
            .map_or_else(Vec::new, |command| {
                command
                    .static_shortcuts
                    .iter()
                    .chain(command.shortcuts.iter())
                    .cloned()
                    .collect()
            })
    }

    #[must_use]
    pub(crate) fn runtime_shortcuts_for(&self, id: CommandId) -> Vec<Shortcut> {
        self.inner
            .read()
            .expect("command registry lock poisoned")
            .commands
            .get(&id)
            .map_or_else(Vec::new, |command| command.shortcuts.clone())
    }

    pub(crate) fn configure_matching(&self, dynamic_shortcuts: bool, broad_command_matching: bool) {
        let mut state = self.inner.write().expect("command registry lock poisoned");
        state.dynamic_shortcuts = dynamic_shortcuts;
        state.broad_command_matching = broad_command_matching;
    }

    #[must_use]
    pub(crate) fn dynamic_shortcuts_enabled(&self) -> bool {
        self.inner
            .read()
            .expect("command registry lock poisoned")
            .dynamic_shortcuts
    }

    #[must_use]
    pub(crate) fn broad_command_matching(&self) -> bool {
        self.inner
            .read()
            .expect("command registry lock poisoned")
            .broad_command_matching
    }

    /// Returns the monotonically changing local command-registry revision.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.inner
            .read()
            .expect("command registry lock poisoned")
            .revision
    }

    pub(crate) fn attach_publication(&self, bots: BotDirectory, catalog: crate::CommandCatalog) {
        let revision = self.revision();
        *self
            .publication
            .write()
            .expect("command publication lock poisoned") = Some(CommandPublicationState {
            bots,
            catalog,
            published_revision: None,
            pending_revision: Some(revision),
            last_error: None,
        });
    }

    pub(crate) fn detach_publication(&self) {
        *self
            .publication
            .write()
            .expect("command publication lock poisoned") = None;
    }

    /// Synchronizes changed command definitions with every attached adapter.
    pub async fn refresh_publication(&self) -> HandlerResult<()> {
        let _gate = self.publication_gate.lock().await;
        let desired_revision = self.revision();
        {
            let mut publication = self
                .publication
                .write()
                .expect("command publication lock poisoned");
            if let Some(publication) = publication.as_mut() {
                if publication.published_revision != Some(desired_revision) {
                    publication.pending_revision = Some(desired_revision);
                }
            }
        }
        let publication = self
            .publication
            .read()
            .expect("command publication lock poisoned")
            .clone();
        let Some(publication) = publication else {
            return Ok(());
        };
        if publication.pending_revision.is_none()
            && publication.published_revision == Some(desired_revision)
        {
            return Ok(());
        }
        match crate::app::publish_command_definitions(&publication.bots, &publication.catalog, self)
            .await
        {
            Ok(()) => {
                let current_revision = self.revision();
                let mut state = self
                    .publication
                    .write()
                    .expect("command publication lock poisoned");
                if let Some(state) = state.as_mut() {
                    state.published_revision = Some(desired_revision);
                    state.last_error = None;
                    state.pending_revision =
                        (current_revision != desired_revision).then_some(current_revision);
                }
                Ok(())
            }
            Err(error) => {
                let message: Arc<str> = Arc::from(error.to_string());
                let mut state = self
                    .publication
                    .write()
                    .expect("command publication lock poisoned");
                if let Some(state) = state.as_mut() {
                    state.pending_revision = Some(self.revision());
                    state.last_error = Some(Arc::clone(&message));
                }
                Err(HandlerError::Api(message.to_string()))
            }
        }
    }

    /// Enables a command and synchronizes changes with attached adapters.
    pub async fn enable_and_publish(&self, id: CommandId) -> HandlerResult<bool> {
        let changed = self.enable(id);
        if changed || self.publication_status().pending_revision.is_some() {
            self.refresh_publication().await?;
        }
        Ok(changed)
    }

    /// Disables a command and synchronizes changes with attached adapters.
    pub async fn disable_and_publish(&self, id: CommandId) -> HandlerResult<bool> {
        let changed = self.disable(id);
        if changed || self.publication_status().pending_revision.is_some() {
            self.refresh_publication().await?;
        }
        Ok(changed)
    }

    /// Returns whether platform command definitions have converged to the
    /// registry's current revision and retains the last retryable failure.
    #[must_use]
    pub fn publication_status(&self) -> CommandPublicationStatus {
        let desired_revision = self.revision();
        let publication = self
            .publication
            .read()
            .expect("command publication lock poisoned");
        publication.as_ref().map_or(
            CommandPublicationStatus {
                desired_revision,
                ..CommandPublicationStatus::default()
            },
            |state| CommandPublicationStatus {
                desired_revision,
                published_revision: state.published_revision,
                pending_revision: state.pending_revision.or_else(|| {
                    (state.published_revision != Some(desired_revision)).then_some(desired_revision)
                }),
                last_error: state.last_error.clone(),
                attached: true,
            },
        )
    }

    /// Returns registered command states in deterministic registration order.
    pub fn snapshot(&self) -> Vec<(CommandId, RuntimeCommandState)> {
        let state = self.inner.read().expect("command registry lock poisoned");
        state
            .order
            .iter()
            .filter_map(|id| state.commands.get(id).cloned().map(|value| (*id, value)))
            .collect()
    }
}

impl<S> Extract<S> for CommandRegistry
where
    S: Send + Sync + 'static,
{
    fn extract(context: &Context<S>) -> Result<Self, ExtractError> {
        Ok(context.authoring().registry.clone())
    }
}

/// Compile-time metadata for one generated command-tree branch.
pub trait CommandBranchTag: Send + Sync + Sized + 'static {
    /// Generated command tree that owns this branch.
    type Command: crate::CommandTree;
    /// Generated arguments parsed for this branch.
    type Arguments: FromCommandMatch;
    /// Case-insensitive branch path beneath the command root.
    const PATH: &'static [&'static str];
    /// Whether this branch handler also matches descendant paths.
    const MATCH_DESCENDANTS: bool = false;
    /// Number of path components removed before parsing [`Self::Arguments`].
    const STRIP_PREFIX: usize = 0;

    /// Binds this generated branch marker to its handler as one feature.
    #[must_use]
    fn handle<S, H, T>(self, handler: H) -> crate::Feature<S>
    where
        S: Send + Sync + 'static,
        H: crate::IntoHandler<T, S>,
    {
        crate::Feature::command_branch(self, handler)
    }
}

/// Empty argument type for a command branch without fields.
#[derive(Clone, Copy, Debug, Default)]
pub struct UnitBranch;

impl FromCommandMatch for UnitBranch {
    fn from_match(_result: &CommandMatch) -> Result<Self, CommandParseError> {
        Ok(Self)
    }
}

/// Extracted, strongly typed arguments for a generated command branch.
#[derive(Clone, Debug)]
pub struct BranchArgs<B>(pub B::Arguments)
where
    B: CommandBranchTag;

impl<B> Deref for BranchArgs<B>
where
    B: CommandBranchTag,
{
    type Target = B::Arguments;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<S, B> Extract<S> for BranchArgs<B>
where
    S: Send + Sync + 'static,
    B: CommandBranchTag,
{
    fn extract(context: &Context<S>) -> Result<Self, ExtractError> {
        let command = context
            .command()
            .ok_or_else(|| ExtractError::new("BranchArgs requires a command match"))?;
        let matches = if B::MATCH_DESCENDANTS {
            command.branch_names().len() >= B::PATH.len()
                && command
                    .branch_names()
                    .iter()
                    .take(B::PATH.len())
                    .zip(B::PATH)
                    .all(|(actual, expected)| actual.eq_ignore_ascii_case(expected))
        } else {
            command.is_branch(B::PATH)
        };
        if !matches {
            return Err(ExtractError::new(
                "the selected command branch does not match this handler",
            ));
        }
        let command = command.descend(B::STRIP_PREFIX);
        B::Arguments::from_match(&command)
            .map(Self)
            .map_err(|error| ExtractError::new(error.localized_message(command.locale())))
    }
}

/// Creates a guard that admits only one selected command-tree path.
#[must_use]
pub fn when_branch<S>(path: impl IntoIterator<Item = impl Into<Arc<str>>>) -> impl Guard<S>
where
    S: Send + Sync + 'static,
{
    let path: Arc<[Arc<str>]> = path.into_iter().map(Into::into).collect::<Vec<_>>().into();
    move |context: Context<S>| {
        let path = Arc::clone(&path);
        async move {
            let Some(command) = context.command() else {
                return GuardDecision::skip();
            };
            let expected = path.iter().map(AsRef::as_ref).collect::<Vec<_>>();
            if command.is_branch(&expected) {
                GuardDecision::allow()
            } else {
                GuardDecision::skip()
            }
        }
    }
}

/// Creates a statically typed command-result condition without introducing
/// stringly typed handler state.
#[must_use]
pub fn when_field_equals<S, T>(field: CommandFieldId, expected: T) -> impl Guard<S>
where
    S: Send + Sync + 'static,
    T: FromCommandValue + Clone + Eq + Send + Sync + 'static,
{
    move |context: Context<S>| {
        let expected = expected.clone();
        async move {
            let Some(command) = context.command() else {
                return Ok::<GuardDecision, HandlerError>(GuardDecision::skip());
            };
            let arguments = if let Some(arguments) = command.arguments() {
                arguments.clone()
            } else {
                command
                    .parse_active()
                    .map_err(|error| HandlerError::Parse(error.to_string()))?
            };
            let value = arguments
                .optional_id::<T>(field)
                .map_err(|error| HandlerError::Parse(error.to_string()))?;
            Ok(if value.as_ref() == Some(&expected) {
                GuardDecision::allow()
            } else {
                GuardDecision::skip()
            })
        }
    }
}

#[cfg(test)]
mod resource_limit_tests {
    use super::*;
    use crate::{
        bot::{CommandWorker, GlobalCommandCapacity},
        budget::PriorityQueueLimiter,
        LocalMediaResolver, RuntimeConfig, RuntimeMetrics,
    };
    use oxidebot_core::{
        application::CommandDefinition, BotCapabilities, BotId, CallApiTrait, CallError,
        CallResult, PlatformId, SupportLevel,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct PublicationApi {
        calls: AtomicUsize,
        failures: AtomicUsize,
    }

    #[async_trait]
    impl CallApiTrait for PublicationApi {
        fn bot_capabilities(&self) -> BotCapabilities {
            let mut capabilities = BotCapabilities::default();
            capabilities.application.structured_commands = SupportLevel::Native;
            capabilities
        }

        async fn set_command_definitions(
            &self,
            _commands: Vec<CommandDefinition>,
        ) -> CallResult<()> {
            self.calls.fetch_add(1, Ordering::AcqRel);
            let mut failures = self.failures.load(Ordering::Acquire);
            let should_fail = loop {
                let Some(next) = failures.checked_sub(1) else {
                    break false;
                };
                match self.failures.compare_exchange_weak(
                    failures,
                    next,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => break true,
                    Err(observed) => failures = observed,
                }
            };
            if should_fail {
                return Err(CallError::temporary("scripted publication failure"));
            }
            Ok(())
        }
    }

    #[test]
    fn command_registry_enforces_shortcut_and_byte_limits() {
        let registry = CommandRegistry::bounded_bytes(2, 512);
        let command = crate::command("bounded");
        registry.register(&command).expect("base command fits");

        let error = registry
            .add_shortcut(
                command.id(),
                Shortcut::literal("x".repeat(200), "y".repeat(200)),
            )
            .expect_err("shortcut exceeds registry byte budget");
        assert!(error.to_string().contains("byte capacity"));

        let oversized = crate::command("oversized").shortcut(Shortcut::literal(
            "x".repeat(crate::MAX_SHORTCUT_PATTERN_BYTES + 1),
            "/oversized",
        ));
        let error = CommandRegistry::bounded(2)
            .register(&oversized)
            .expect_err("oversized pattern is rejected");
        assert!(error.to_string().contains("pattern byte limit"));
    }

    #[tokio::test]
    async fn local_media_resolver_reads_only_limit_plus_one_bytes() {
        static NEXT_MEDIA_PATH: AtomicUsize = AtomicUsize::new(0);
        let sequence = NEXT_MEDIA_PATH.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "oxidebot-media-limit-{}-{}",
            std::process::id(),
            sequence
        ));
        std::fs::write(&path, b"123456789").expect("write temporary media");
        let result = LocalMediaResolver::new(8).read_path(path.clone()).await;
        let _ = std::fs::remove_file(path);
        let error = result.expect_err("media larger than the hard limit is rejected");
        assert!(error.to_string().contains("byte limit"));
    }

    #[tokio::test]
    async fn failed_publication_remains_pending_and_a_noop_toggle_retries_it() {
        let api = Arc::new(PublicationApi::default());
        let config = RuntimeConfig::default();
        let metrics = Arc::new(RuntimeMetrics::default());
        let descriptor = crate::BotDescriptor::new(
            PlatformId::new("publication-test").expect("static platform id"),
            BotId::new("bot").expect("static bot id"),
        );
        let services = crate::BotServices::new(api.clone());
        let global_limiter =
            PriorityQueueLimiter::new(config.global_command, config.max_command_bytes);
        let global_capacity = GlobalCommandCapacity::new(
            config.command_in_flight_global,
            config.command_in_flight_reserved_high_global,
        );
        let (handle, worker) = CommandWorker::build(
            oxidebot_core::BotSlot(0),
            descriptor,
            services,
            global_limiter,
            global_capacity,
            config.command,
            config.max_command_bytes,
            config.command_overload,
            config.command_in_flight_per_bot,
            config.command_in_flight_reserved_high_per_bot,
            config.command_high_priority_burst,
            config.command_attempt_timeout,
            config.command_total_timeout,
            0,
            config.command_retry_base,
            config.command_retry_max,
            metrics,
        );
        let worker_task = tokio::spawn(worker.run());
        let command = crate::command("publish-test");
        let registry = CommandRegistry::bounded(4);
        registry.register(&command).expect("register command");
        let directory = BotDirectory::new(vec![handle]);
        registry.attach_publication(
            directory.clone(),
            crate::CommandCatalog::new([command.clone()]),
        );
        registry
            .refresh_publication()
            .await
            .expect("initial publication succeeds");

        api.failures.store(1, Ordering::Release);
        registry
            .disable_and_publish(command.id())
            .await
            .expect_err("first dynamic publication fails");
        assert_eq!(
            registry.publication_status().pending_revision,
            Some(registry.revision())
        );

        let changed = registry
            .disable_and_publish(command.id())
            .await
            .expect("unchanged state retries pending publication");
        assert!(!changed);
        assert_eq!(registry.publication_status().pending_revision, None);
        assert_eq!(api.calls.load(Ordering::Acquire), 3);

        registry.detach_publication();
        drop(directory);
        worker_task.await.expect("command worker exits cleanly");
    }
}
