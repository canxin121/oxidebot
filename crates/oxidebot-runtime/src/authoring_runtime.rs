//! Runtime storage and execution of authoring hooks and delivery middleware.

use super::*;

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
