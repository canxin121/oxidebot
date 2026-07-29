use crate::{
    BotDirectory, BotHandle, Command, CommandFieldId, CommandId, CommandMatch, CommandOutput,
    CommandParseError, CommandRenderer, CompletionItem, Context, Extract, ExtractError,
    FromCommandMatch, FromCommandValue, Guard, GuardDecision, HandlerError, HandlerResult,
};
use async_trait::async_trait;
use futures_util::{future::BoxFuture, FutureExt};
use oxidebot_core::{
    conversation::{ConversationKind, ConversationRef, MessageTarget},
    source::message::{
        DeliveryPlan, DeliveryReport, FallbackPolicy, File, Message, MessageSegment,
    },
    BotIdentity, LocalizedMessage, Media, PlatformId, TemplateValue, TranslationCatalog,
};
use regex::Regex;
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    future::Future,
    marker::PhantomData,
    ops::Deref,
    panic::AssertUnwindSafe,
    path::PathBuf,
    sync::{Arc, RwLock},
};

/// Maximum UTF-8 byte length of a shortcut's literal or regex pattern.
pub const MAX_SHORTCUT_PATTERN_BYTES: usize = 4 * 1024;
/// Maximum UTF-8 byte length of a shortcut replacement template.
pub const MAX_SHORTCUT_REPLACEMENT_BYTES: usize = 16 * 1024;
/// Maximum UTF-8 byte length of a shortcut's human-readable display text.
pub const MAX_SHORTCUT_HUMANIZED_BYTES: usize = 4 * 1024;
/// Maximum input bytes inspected when looking up runtime shortcuts.
pub const MAX_SHORTCUT_SCAN_PER_MESSAGE: usize = 4_096;
/// Maximum runtime shortcuts retained by one command registry.
pub const MAX_REGISTRY_SHORTCUTS: usize = 65_536;

/// One command shortcut compiled at application build time.
#[derive(Clone, Debug)]
pub struct Shortcut {
    pattern: ShortcutPattern,
    replacement: Arc<str>,
    humanized: Option<Arc<str>>,
    keep_tail: bool,
    compact: bool,
}

/// Matching representation used by a [`Shortcut`].
#[derive(Clone, Debug)]
pub enum ShortcutPattern {
    /// Prefix match against literal command text.
    Literal(Arc<str>),
    /// Prefix-anchored regular-expression match.
    Regex(Arc<Regex>),
}

impl Shortcut {
    /// Creates a literal prefix shortcut with a replacement command text.
    #[must_use]
    pub fn literal(pattern: impl Into<Arc<str>>, replacement: impl Into<Arc<str>>) -> Self {
        Self {
            pattern: ShortcutPattern::Literal(pattern.into()),
            replacement: replacement.into(),
            humanized: None,
            keep_tail: true,
            compact: false,
        }
    }

    /// Compiles a regular-expression shortcut with a replacement template.
    pub fn regex(pattern: &str, replacement: impl Into<Arc<str>>) -> Result<Self, regex::Error> {
        if pattern.is_empty() || pattern.len() > MAX_SHORTCUT_PATTERN_BYTES {
            return Err(regex::Error::Syntax(format!(
                "shortcut regex must contain 1..={MAX_SHORTCUT_PATTERN_BYTES} bytes"
            )));
        }
        Ok(Self {
            pattern: ShortcutPattern::Regex(Arc::new(Regex::new(pattern)?)),
            replacement: replacement.into(),
            humanized: None,
            keep_tail: true,
            compact: false,
        })
    }

    /// Sets presentation text used instead of the raw pattern in user interfaces.
    #[must_use]
    pub fn humanized(mut self, value: impl Into<Arc<str>>) -> Self {
        self.humanized = Some(value.into());
        self
    }

    /// Controls whether unmatched input text after a match is preserved.
    #[must_use]
    pub const fn keep_tail(mut self, enabled: bool) -> Self {
        self.keep_tail = enabled;
        self
    }

    /// Allows a literal shortcut to be immediately followed by its tail.
    /// By default a literal shortcut must end at a token boundary.
    #[must_use]
    pub const fn compact(mut self, enabled: bool) -> Self {
        self.compact = enabled;
        self
    }

    /// Returns whether this shortcut uses a regular-expression pattern.
    #[must_use]
    pub fn is_regex(&self) -> bool {
        matches!(self.pattern, ShortcutPattern::Regex(_))
    }

    /// Returns the literal or regular-expression source text.
    #[must_use]
    pub fn pattern_text(&self) -> &str {
        match &self.pattern {
            ShortcutPattern::Literal(value) => value,
            ShortcutPattern::Regex(value) => value.as_str(),
        }
    }

    /// Returns human-readable shortcut text, falling back to its pattern.
    #[must_use]
    pub fn display(&self) -> &str {
        self.humanized
            .as_deref()
            .unwrap_or_else(|| match &self.pattern {
                ShortcutPattern::Literal(value) => value,
                ShortcutPattern::Regex(value) => value.as_str(),
            })
    }

    /// Rewrites matching command input, or returns `None` when it does not match.
    #[must_use]
    pub fn rewrite(&self, input: &str) -> Option<String> {
        match &self.pattern {
            ShortcutPattern::Literal(pattern) => {
                let tail = input.strip_prefix(pattern.as_ref())?;
                if !self.compact
                    && !tail.is_empty()
                    && !tail.chars().next().is_some_and(char::is_whitespace)
                {
                    return None;
                }
                if !self.keep_tail && !tail.trim().is_empty() {
                    return None;
                }
                Some(if self.keep_tail {
                    format!("{}{}", self.replacement, tail)
                } else {
                    self.replacement.to_string()
                })
            }
            ShortcutPattern::Regex(pattern) => {
                let captures = pattern.captures(input)?;
                if captures.get(0).is_some_and(|capture| capture.start() != 0) {
                    return None;
                }
                Some(expand_replacement(
                    &self.replacement,
                    &captures,
                    input,
                    self.keep_tail,
                ))
            }
        }
    }

    fn validate(&self) -> Result<(), HandlerError> {
        if self.pattern_text().is_empty() || self.pattern_text().len() > MAX_SHORTCUT_PATTERN_BYTES
        {
            return Err(HandlerError::internal(
                "shortcut pattern byte limit exceeded",
            ));
        }
        if self.replacement.len() > MAX_SHORTCUT_REPLACEMENT_BYTES {
            return Err(HandlerError::internal(
                "shortcut replacement byte limit exceeded",
            ));
        }
        if self
            .humanized
            .as_ref()
            .is_some_and(|value| value.len() > MAX_SHORTCUT_HUMANIZED_BYTES)
        {
            return Err(HandlerError::internal(
                "shortcut display text byte limit exceeded",
            ));
        }
        Ok(())
    }

    fn retained_bytes(&self) -> usize {
        self.pattern_text()
            .len()
            .saturating_add(self.replacement.len())
            .saturating_add(self.humanized.as_ref().map_or(0, |value| value.len()))
            .saturating_add(128)
    }
}

/// Data-only command metadata overlay. It can adjust presentation, prefixes,
/// aliases, and shortcuts without replacing the statically compiled handler or
/// executing code from configuration files.
#[derive(Clone, Debug)]
pub struct CommandOverlay {
    target: Arc<str>,
    description: Option<Arc<str>>,
    translations: Vec<(Arc<str>, Arc<str>)>,
    aliases: Vec<Arc<str>>,
    prefixes: Option<Vec<Arc<str>>>,
    shortcuts: Vec<Shortcut>,
    hidden: bool,
}

impl CommandOverlay {
    /// Creates a data-only overlay targeting a command name or alias.
    #[must_use]
    pub fn new(target: impl Into<Arc<str>>) -> Self {
        Self {
            target: target.into(),
            description: None,
            translations: Vec::new(),
            aliases: Vec::new(),
            prefixes: None,
            shortcuts: Vec::new(),
            hidden: false,
        }
    }

    /// Replaces the command's default description.
    #[must_use]
    pub fn description(mut self, value: impl Into<Arc<str>>) -> Self {
        self.description = Some(value.into());
        self
    }

    /// Adds a localized description override for `locale`.
    #[must_use]
    pub fn description_translation(
        mut self,
        locale: impl Into<Arc<str>>,
        value: impl Into<Arc<str>>,
    ) -> Self {
        self.translations.push((locale.into(), value.into()));
        self
    }

    /// Adds an alias to the targeted command.
    #[must_use]
    pub fn alias(mut self, value: impl Into<Arc<str>>) -> Self {
        self.aliases.push(value.into());
        self
    }

    /// Replaces command prefixes with `values`.
    #[must_use]
    pub fn prefixes<I, T>(mut self, values: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<Arc<str>>,
    {
        self.prefixes = Some(values.into_iter().map(Into::into).collect());
        self
    }

    /// Adds a static shortcut to the targeted command.
    #[must_use]
    pub fn shortcut(mut self, value: Shortcut) -> Self {
        self.shortcuts.push(value);
        self
    }

    /// Hides the targeted command from generated catalogs and help.
    #[must_use]
    pub const fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }

    pub(crate) fn matches(&self, command: &Command) -> bool {
        command.name().eq_ignore_ascii_case(&self.target)
            || command
                .aliases_list()
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(&self.target))
    }

    pub(crate) fn apply(&self, mut command: Command) -> Command {
        if let Some(description) = &self.description {
            command = command.description(Arc::clone(description));
        }
        for (locale, value) in &self.translations {
            command = command.description_translation(Arc::clone(locale), Arc::clone(value));
        }
        command = command.aliases(self.aliases.iter().cloned());
        if let Some(prefixes) = &self.prefixes {
            command = command.prefixes(prefixes.iter().cloned());
        }
        for shortcut in &self.shortcuts {
            command = command.shortcut(shortcut.clone());
        }
        if self.hidden {
            command = command.hidden();
        }
        command
    }
}

fn expand_replacement(
    template: &str,
    captures: &regex::Captures<'_>,
    input: &str,
    keep_tail: bool,
) -> String {
    let mut output = String::with_capacity(template.len().saturating_add(input.len()));
    let mut used_tail = false;
    let mut chars = template.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '{' {
            output.push(ch);
            continue;
        }
        let mut key = String::new();
        for next in chars.by_ref() {
            if next == '}' {
                break;
            }
            key.push(next);
        }
        if key == "*" {
            used_tail = true;
            if keep_tail {
                let end = captures.get(0).map_or(0, |capture| capture.end());
                output.push_str(input.get(end..).unwrap_or_default().trim_start());
            }
        } else if let Ok(index) = key.parse::<usize>() {
            if let Some(value) = captures.get(index) {
                output.push_str(value.as_str());
            }
        } else if let Some(value) = captures.name(&key) {
            output.push_str(value.as_str());
        }
    }
    if keep_tail && !used_tail {
        let end = captures.get(0).map_or(0, |capture| capture.end());
        output.push_str(input.get(end..).unwrap_or_default());
    }
    output
}

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

    #[must_use]
    pub fn find(&self, name: &str) -> Option<CommandId> {
        self.inner
            .read()
            .expect("command registry lock poisoned")
            .commands
            .iter()
            .find_map(|(id, command)| command.name.eq_ignore_ascii_case(name).then_some(*id))
    }

    #[must_use]
    pub fn is_enabled(&self, id: CommandId) -> bool {
        self.inner
            .read()
            .expect("command registry lock poisoned")
            .commands
            .get(&id)
            .is_none_or(|command| command.enabled)
    }

    pub fn enable(&self, id: CommandId) -> bool {
        self.set_enabled(id, true)
    }

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

    pub async fn enable_and_publish(&self, id: CommandId) -> HandlerResult<bool> {
        let changed = self.enable(id);
        if changed || self.publication_status().pending_revision.is_some() {
            self.refresh_publication().await?;
        }
        Ok(changed)
    }

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

/// Selects the bot used when sending to an [`Address`].
#[derive(Clone, Debug)]
pub enum BotSelection {
    /// Use the bot currently processing a handler event.
    Current,
    /// Use the connection with this exact identity.
    Exact(BotIdentity),
    /// Use the sole connected bot for this platform.
    Platform(PlatformId),
}

/// A message target plus deterministic bot-selection policy.
#[derive(Clone, Debug)]
pub struct Address {
    /// Destination conversation and optional recipients.
    pub target: MessageTarget,
    /// Bot selection used to reach the destination.
    pub bot: BotSelection,
}

impl From<MessageTarget> for Address {
    fn from(target: MessageTarget) -> Self {
        Self {
            target,
            bot: BotSelection::Current,
        }
    }
}

impl Address {
    #[must_use]
    pub fn direct(user_id: impl Into<String>) -> Self {
        Self {
            target: MessageTarget::new(ConversationRef::direct(user_id.into())),
            bot: BotSelection::Current,
        }
    }

    #[must_use]
    pub fn group(group_id: impl Into<String>) -> Self {
        Self {
            target: MessageTarget::new(ConversationRef::group(group_id.into())),
            bot: BotSelection::Current,
        }
    }

    #[must_use]
    pub fn channel(channel_id: impl Into<String>) -> Self {
        Self {
            target: MessageTarget::new(ConversationRef::new(
                channel_id.into(),
                ConversationKind::Channel,
            )),
            bot: BotSelection::Current,
        }
    }

    #[must_use]
    pub fn thread(thread_id: impl Into<String>, parent: ConversationRef) -> Self {
        Self {
            target: MessageTarget::new(
                ConversationRef::new(thread_id.into(), ConversationKind::Thread).child_of(parent),
            ),
            bot: BotSelection::Current,
        }
    }

    #[must_use]
    pub fn recipients(mut self, recipients: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.target = self.target.recipients(recipients);
        self
    }

    #[must_use]
    pub fn through(mut self, bot: BotSelection) -> Self {
        self.bot = bot;
        self
    }
}

impl BotDirectory {
    pub fn select(&self, selection: &BotSelection) -> Result<BotHandle, HandlerError> {
        match selection {
            BotSelection::Current => Err(HandlerError::internal(
                "BotSelection::Current requires a handler Context",
            )),
            BotSelection::Exact(identity) => self
                .iter()
                .find(|bot| bot.identity() == identity)
                .cloned()
                .ok_or_else(|| HandlerError::Api("the selected bot is not connected".into())),
            BotSelection::Platform(platform) => {
                let mut matches = self
                    .iter()
                    .filter(|bot| &bot.identity().platform == platform);
                let first = matches.next().cloned();
                if matches.next().is_some() {
                    return Err(HandlerError::Api(
                        "more than one bot matches the platform; select an exact bot".into(),
                    ));
                }
                first.ok_or_else(|| HandlerError::Api("no bot matches the platform".into()))
            }
        }
    }

    pub async fn send_address(
        &self,
        address: Address,
        message: impl Into<Message>,
        fallback: FallbackPolicy,
    ) -> Result<DeliveryReport, HandlerError> {
        let bot = self.select(&address.bot)?;
        bot.send_outgoing_message_with(address.target, message.into(), fallback)
            .await
            .map_err(HandlerError::from)
    }
}

#[derive(Clone, Debug)]
pub struct TargetDirectory {
    inner: Arc<RwLock<TargetDirectoryState>>,
}

#[derive(Debug)]
struct TargetDirectoryState {
    capacity: usize,
    max_bytes: usize,
    retained_bytes: usize,
    aliases: BTreeMap<Arc<str>, Address>,
}

impl TargetDirectory {
    #[must_use]
    pub fn bounded(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self::bounded_bytes(capacity, capacity.saturating_mul(8 * 1024))
    }

    /// Creates a target directory bounded by aliases and retained address
    /// bytes.
    #[must_use]
    pub fn bounded_bytes(capacity: usize, max_bytes: usize) -> Self {
        Self {
            inner: Arc::new(RwLock::new(TargetDirectoryState {
                capacity: capacity.max(1),
                max_bytes: max_bytes.max(1),
                retained_bytes: 0,
                aliases: BTreeMap::new(),
            })),
        }
    }

    pub fn insert(&self, alias: impl Into<Arc<str>>, address: Address) -> Result<(), HandlerError> {
        let mut state = self.inner.write().expect("target directory lock poisoned");
        let alias = alias.into();
        if alias.is_empty() || alias.len() > 4 * 1024 {
            return Err(HandlerError::internal(
                "target alias must contain 1..=4096 bytes",
            ));
        }
        if !state.aliases.contains_key(&alias) && state.aliases.len() >= state.capacity {
            return Err(HandlerError::internal("target directory capacity exceeded"));
        }
        let entry_bytes = target_entry_bytes(&alias, &address);
        let previous_bytes = state
            .aliases
            .get(&alias)
            .map_or(0, |previous| target_entry_bytes(&alias, previous));
        let retained_bytes = state
            .retained_bytes
            .saturating_sub(previous_bytes)
            .saturating_add(entry_bytes);
        if retained_bytes > state.max_bytes {
            return Err(HandlerError::internal(
                "target directory byte capacity exceeded",
            ));
        }
        state.aliases.insert(alias, address);
        state.retained_bytes = retained_bytes;
        Ok(())
    }

    #[must_use]
    pub fn resolve(&self, alias: &str) -> Option<Address> {
        self.inner
            .read()
            .expect("target directory lock poisoned")
            .aliases
            .get(alias)
            .cloned()
    }

    pub fn remove(&self, alias: &str) -> Option<Address> {
        let mut state = self.inner.write().expect("target directory lock poisoned");
        let removed = state.aliases.remove(alias);
        if let Some(address) = &removed {
            state.retained_bytes = state
                .retained_bytes
                .saturating_sub(target_entry_bytes(alias, address));
        }
        removed
    }

    #[must_use]
    pub fn aliases(&self) -> Vec<Arc<str>> {
        self.inner
            .read()
            .expect("target directory lock poisoned")
            .aliases
            .keys()
            .cloned()
            .collect()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.inner
            .read()
            .expect("target directory lock poisoned")
            .aliases
            .len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn target_entry_bytes(alias: &str, address: &Address) -> usize {
    let target_bytes = serde_json::to_vec(&address.target).map_or(usize::MAX, |value| value.len());
    let selection_bytes = match &address.bot {
        BotSelection::Current => 0,
        BotSelection::Exact(identity) => identity
            .platform
            .as_str()
            .len()
            .saturating_add(identity.bot.as_str().len()),
        BotSelection::Platform(platform) => platform.as_str().len(),
    };
    alias
        .len()
        .saturating_add(target_bytes)
        .saturating_add(selection_bytes)
        .saturating_add(128)
}

#[derive(Clone, Debug)]
pub struct ResolvedMedia {
    pub bytes: Arc<[u8]>,
    pub mime: Option<Arc<str>>,
    pub name: Option<Arc<str>>,
}

#[async_trait]
pub trait MediaResolver: Send + Sync + 'static {
    async fn resolve(&self, media: &Media) -> HandlerResult<ResolvedMedia>;
    async fn resolve_file(&self, file: &File) -> HandlerResult<ResolvedMedia>;
}

#[derive(Clone, Debug)]
pub struct LocalMediaResolver {
    max_bytes: usize,
}

impl LocalMediaResolver {
    #[must_use]
    pub const fn new(max_bytes: usize) -> Self {
        Self { max_bytes }
    }

    async fn read_path(&self, path: PathBuf) -> HandlerResult<ResolvedMedia> {
        use tokio::io::AsyncReadExt;

        let file = tokio::fs::File::open(&path)
            .await
            .map_err(|error| HandlerError::Api(error.to_string()))?;
        let limit = u64::try_from(self.max_bytes.saturating_add(1)).unwrap_or(u64::MAX);
        let mut reader = file.take(limit);
        let mut bytes = Vec::with_capacity(self.max_bytes.min(64 * 1024));
        reader
            .read_to_end(&mut bytes)
            .await
            .map_err(|error| HandlerError::Api(error.to_string()))?;
        if bytes.len() > self.max_bytes {
            return Err(HandlerError::Api(
                "media exceeds configured byte limit".into(),
            ));
        }
        let mime = mime_guess::from_path(&path).first_raw().map(Arc::from);
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .map(Arc::from);
        Ok(ResolvedMedia {
            bytes: bytes.into(),
            mime,
            name,
        })
    }
}

#[async_trait]
impl MediaResolver for LocalMediaResolver {
    async fn resolve(&self, media: &Media) -> HandlerResult<ResolvedMedia> {
        self.resolve_file(&media.file).await
    }

    async fn resolve_file(&self, file: &File) -> HandlerResult<ResolvedMedia> {
        if let Some(path) = &file.path {
            return self.read_path(path.clone()).await;
        }
        if let Some(base64) = &file.base64 {
            if base64.len() > self.max_bytes.saturating_mul(2) {
                return Err(HandlerError::Api(
                    "file exceeds configured byte limit".into(),
                ));
            }
            let bytes = decode_base64(base64)?;
            if bytes.len() > self.max_bytes {
                return Err(HandlerError::Api(
                    "file exceeds configured byte limit".into(),
                ));
            }
            return Ok(ResolvedMedia {
                bytes: bytes.into(),
                mime: file.mime.as_deref().map(Arc::from),
                name: (!file.name.is_empty()).then(|| Arc::from(file.name.as_str())),
            });
        }
        Err(HandlerError::Api(
            "the default media resolver only supports local paths and retained base64 payloads"
                .into(),
        ))
    }
}

fn decode_base64(input: &str) -> HandlerResult<Vec<u8>> {
    fn value(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'+' | b'-' => Some(62),
            b'/' | b'_' => Some(63),
            _ => None,
        }
    }

    let payload = input
        .strip_prefix("data:")
        .and_then(|value| value.split_once(','))
        .map_or(input, |(_, payload)| payload);
    let bytes = payload
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    if bytes.len() % 4 == 1 {
        return Err(HandlerError::Parse("invalid base64 media payload".into()));
    }
    let mut output = Vec::with_capacity(bytes.len().saturating_mul(3) / 4);
    let mut index = 0;
    while index < bytes.len() {
        let remaining = bytes.len() - index;
        let take = remaining.min(4);
        let chunk = &bytes[index..index + take];
        let padding = chunk.iter().rev().take_while(|byte| **byte == b'=').count();
        if padding > 2
            || (padding > 0 && index + take != bytes.len())
            || chunk[..chunk.len().saturating_sub(padding)].contains(&b'=')
        {
            return Err(HandlerError::Parse("invalid base64 media payload".into()));
        }
        let mut values = [0_u8; 4];
        for (slot, byte) in chunk.iter().enumerate() {
            if *byte != b'=' {
                values[slot] = value(*byte)
                    .ok_or_else(|| HandlerError::Parse("invalid base64 media payload".into()))?;
            }
        }
        output.push((values[0] << 2) | (values[1] >> 4));
        if take >= 3 && padding < 2 {
            output.push((values[1] << 4) | (values[2] >> 2));
        }
        if take == 4 && padding == 0 {
            output.push((values[2] << 6) | values[3]);
        }
        index += take;
    }
    Ok(output)
}

#[derive(Clone, Debug)]
pub struct ResolvedMessageMedia {
    pub path: Arc<str>,
    pub media: ResolvedMedia,
}

/// Resolves every file-bearing segment, including nested forwarded messages,
/// gallery items, share images, captions' thumbnails, and rich media.
pub async fn resolve_message_media<R>(
    resolver: &R,
    message: &Message,
) -> HandlerResult<Vec<ResolvedMessageMedia>>
where
    R: MediaResolver + ?Sized,
{
    enum Source<'a> {
        File(&'a File),
        Media(&'a Media),
    }
    fn collect<'a>(message: &'a Message, prefix: &str, output: &mut Vec<(String, Source<'a>)>) {
        for (index, segment) in message.segments.iter().enumerate() {
            let path = format!("{prefix}segments[{index}]");
            match segment {
                MessageSegment::Share {
                    image: Some(file), ..
                } => {
                    output.push((format!("{path}.image"), Source::File(file)));
                }
                MessageSegment::Media { media, .. } => {
                    output.push((path.clone(), Source::Media(media)));
                    if let Some(thumbnail) = &media.thumbnail {
                        output.push((format!("{path}.thumbnail"), Source::File(thumbnail)));
                    }
                }
                MessageSegment::MediaGallery(items) => {
                    for (item_index, item) in items.iter().enumerate() {
                        let item_path = format!("{path}.items[{item_index}]");
                        output.push((item_path.clone(), Source::Media(&item.media)));
                        if let Some(thumbnail) = &item.media.thumbnail {
                            output
                                .push((format!("{item_path}.thumbnail"), Source::File(thumbnail)));
                        }
                    }
                }
                MessageSegment::Emoji(emoji) => {
                    if let Some(file) = &emoji.file {
                        output.push((path, Source::File(file)));
                    }
                }
                MessageSegment::Sticker(sticker) => {
                    if let Some(file) = &sticker.file {
                        output.push((path, Source::File(file)));
                    }
                }
                MessageSegment::ForwardCustomNode { message, .. } => {
                    collect(message, &format!("{path}.message."), output);
                }
                _ => {}
            }
        }
    }

    let mut sources = Vec::new();
    collect(message, "", &mut sources);
    let mut output = Vec::with_capacity(sources.len());
    for (path, source) in sources {
        let media = match source {
            Source::File(file) => resolver.resolve_file(file).await?,
            Source::Media(media) => resolver.resolve(media).await?,
        };
        output.push(ResolvedMessageMedia {
            path: Arc::from(path),
            media,
        });
    }
    Ok(output)
}

#[async_trait]
pub trait MediaHost: Send + Sync + 'static {
    /// Stores resolved bytes and returns a portable file descriptor, usually
    /// containing a URL accepted by adapters that cannot upload local bytes.
    async fn host(&self, media: ResolvedMedia) -> HandlerResult<File>;
}

#[async_trait]
pub trait MediaFetcher: Send + Sync + 'static {
    /// Fetches one external URI under the caller's byte and protocol policy.
    async fn fetch(&self, uri: &str, max_bytes: usize) -> HandlerResult<ResolvedMedia>;
}

#[derive(Clone)]
pub struct PortableMediaResolver<F> {
    local: LocalMediaResolver,
    fetcher: Arc<F>,
}

impl<F> PortableMediaResolver<F> {
    #[must_use]
    pub fn new(max_bytes: usize, fetcher: F) -> Self {
        Self {
            local: LocalMediaResolver::new(max_bytes),
            fetcher: Arc::new(fetcher),
        }
    }
}

#[async_trait]
impl<F> MediaResolver for PortableMediaResolver<F>
where
    F: MediaFetcher,
{
    async fn resolve(&self, media: &Media) -> HandlerResult<ResolvedMedia> {
        self.resolve_file(&media.file).await
    }

    async fn resolve_file(&self, file: &File) -> HandlerResult<ResolvedMedia> {
        if file.path.is_some() || file.base64.is_some() {
            return self.local.resolve_file(file).await;
        }
        let uri = file.uri.as_deref().ok_or_else(|| {
            HandlerError::Api("file has no resolvable path, payload, or URI".into())
        })?;
        self.fetcher.fetch(uri, self.local.max_bytes).await
    }
}

pub async fn resolve_and_host_file<R, H>(resolver: &R, host: &H, file: &File) -> HandlerResult<File>
where
    R: MediaResolver + ?Sized,
    H: MediaHost + ?Sized,
{
    host.host(resolver.resolve_file(file).await?).await
}

/// Async command-value resolver. It remains an explicit cold-path operation;
/// ordinary `Args<T>` extraction stays synchronous and allocation-free.
#[async_trait]
pub trait ResolveCommandValue<S>: Sized + Send + 'static
where
    S: Send + Sync + 'static,
{
    async fn resolve(
        context: &Context<S>,
        field: CommandFieldId,
        command: &CommandMatch,
    ) -> HandlerResult<Self>;
}

#[derive(Clone)]
pub struct Resolve<T, S>
where
    S: Send + Sync + 'static,
{
    context: Context<S>,
    _value: PhantomData<fn() -> T>,
}

impl<T, S> Resolve<T, S>
where
    T: ResolveCommandValue<S>,
    S: Send + Sync + 'static,
{
    pub async fn field(&self, field: CommandFieldId) -> HandlerResult<T> {
        let command = self
            .context
            .command()
            .ok_or_else(|| HandlerError::Parse("resolver requires a command match".into()))?;
        T::resolve(&self.context, field, command).await
    }

    /// Resolves a field through its macro-generated static marker.
    pub async fn get<F>(&self, _field: F) -> HandlerResult<T>
    where
        F: crate::CommandFieldTag,
    {
        let command = self
            .context
            .command()
            .ok_or_else(|| HandlerError::Parse("resolver requires a command match".into()))?;
        let branch = command
            .branch_names()
            .iter()
            .map(AsRef::as_ref)
            .collect::<Vec<_>>();
        let field = command
            .command()
            .field_id(&branch, F::NAME)
            .ok_or_else(|| {
                HandlerError::Parse(format!(
                    "command field `{}` is not active on branch `{}`",
                    F::NAME,
                    branch.join(" "),
                ))
            })?;
        T::resolve(&self.context, field, command).await
    }
}

impl<T, S> Extract<S> for Resolve<T, S>
where
    T: ResolveCommandValue<S>,
    S: Send + Sync + 'static,
{
    fn extract(context: &Context<S>) -> Result<Self, ExtractError> {
        Ok(Self {
            context: context.clone(),
            _value: PhantomData,
        })
    }
}

/// A reusable validation and conversion pattern for command values.
pub trait ValuePattern<T>: Send + Sync + 'static {
    fn parse(&self, value: &crate::CommandValue) -> Result<T, CommandParseError>;
    fn describe(&self) -> Arc<str>;
}

pub struct FnValuePattern<T, F> {
    description: Arc<str>,
    parser: F,
    _value: PhantomData<fn() -> T>,
}

#[must_use]
pub fn value_pattern<T, F>(description: impl Into<Arc<str>>, parser: F) -> FnValuePattern<T, F>
where
    F: Fn(&crate::CommandValue) -> Result<T, CommandParseError> + Send + Sync + 'static,
{
    FnValuePattern {
        description: description.into(),
        parser,
        _value: PhantomData,
    }
}

impl<T, F> ValuePattern<T> for FnValuePattern<T, F>
where
    T: Send + Sync + 'static,
    F: Fn(&crate::CommandValue) -> Result<T, CommandParseError> + Send + Sync + 'static,
{
    fn parse(&self, value: &crate::CommandValue) -> Result<T, CommandParseError> {
        (self.parser)(value)
    }

    fn describe(&self) -> Arc<str> {
        Arc::clone(&self.description)
    }
}

#[derive(Clone, Debug)]
pub struct RegexTextPattern {
    regex: Regex,
    description: Arc<str>,
}

impl RegexTextPattern {
    pub fn new(pattern: &str, description: impl Into<Arc<str>>) -> Result<Self, regex::Error> {
        Ok(Self {
            regex: Regex::new(pattern)?,
            description: description.into(),
        })
    }
}

impl ValuePattern<String> for RegexTextPattern {
    fn parse(&self, value: &crate::CommandValue) -> Result<String, CommandParseError> {
        let Some(text) = value.as_text() else {
            return Err(CommandParseError::UnexpectedValue {
                expected: "text",
                actual: "non-text command value",
            });
        };
        if self.regex.is_match(text) {
            Ok(text.to_owned())
        } else {
            Err(CommandParseError::InvalidValue {
                value: text.to_owned(),
                expected: "matching text",
                reason: self.description.to_string(),
            })
        }
    }

    fn describe(&self) -> Arc<str> {
        Arc::clone(&self.description)
    }
}

#[cfg(test)]
mod resource_limit_tests {
    use super::*;
    use crate::{
        bot::{CommandWorker, GlobalCommandCapacity},
        budget::PriorityQueueLimiter,
        RuntimeConfig, RuntimeMetrics,
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
            "x".repeat(MAX_SHORTCUT_PATTERN_BYTES + 1),
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
