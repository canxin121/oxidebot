//! Public command, locale, normalization, and delivery authoring hooks.

use super::*;

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
