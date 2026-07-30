use crate::{
    BotDirectory, BotHandle, Command, CommandFieldId, CommandId, CommandMatch, CommandOutput,
    CommandParseError, CommandRegistry, CommandRenderer, CompletionItem, Context, Extract,
    ExtractError, FromCommandMatch, FromCommandValue, Guard, GuardDecision, HandlerError,
    HandlerResult,
};
use async_trait::async_trait;
use futures_util::{future::BoxFuture, FutureExt};
use oxidebot_core::{
    conversation::MessageTarget,
    source::message::{DeliveryPlan, DeliveryReport, FallbackPolicy, Message},
    LocalizedMessage, TemplateValue, TranslationCatalog,
};
use std::{
    collections::HashMap,
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
        LocalMediaResolver, RuntimeConfig, RuntimeMetrics, Shortcut,
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
