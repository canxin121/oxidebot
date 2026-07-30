//! Erased handler representation and runtime invocation pipeline.

use super::*;

// Command selectors are stored once per handler; keeping commands inline avoids
// an allocation on every registration.
#[allow(clippy::large_enum_variant)]
pub(in crate::module) enum Selector {
    Event {
        event_type: EventType,
        kind: DispatchKind,
    },
    Command(Command),
    Interaction(Arc<str>),
    Native(Arc<str>),
}

pub(in crate::module) enum EndpointKind<S>
where
    S: Send + Sync + 'static,
{
    Handler(Arc<dyn Endpoint<S>>),
    Help,
}

pub(in crate::module) struct HandlerDefinition<S>
where
    S: Send + Sync + 'static,
{
    pub(in crate::module) selector: Selector,
    pub(in crate::module) endpoint: EndpointKind<S>,
    pub(in crate::module) guards: Vec<SharedGuard<S>>,
    pub(in crate::module) before: Vec<SharedBefore<S>>,
    pub(in crate::module) after: Vec<SharedAfter<S>>,
    pub(in crate::module) scope: RouteScope,
    pub(in crate::module) default_block: bool,
    pub(in crate::module) branch_filter: Option<(Arc<[Arc<str>]>, bool)>,
}

pub(in crate::module) struct ModuleHandler<S>
where
    S: Send + Sync + 'static,
{
    pub(in crate::module) spec: RouteSpec,
    pub(in crate::module) event_kind: DispatchKind,
    pub(in crate::module) scope: RouteScope,
    pub(in crate::module) endpoint: Arc<dyn Endpoint<S>>,
    pub(in crate::module) guards: Arc<[SharedGuard<S>]>,
    pub(in crate::module) before: Arc<[SharedBefore<S>]>,
    pub(in crate::module) after: Arc<[SharedAfter<S>]>,
    pub(in crate::module) command: Option<Command>,
    pub(in crate::module) default_block: bool,
    pub(in crate::module) branch_filter: Option<(Arc<[Arc<str>]>, bool)>,
}

fn bind_delivery<S>(context: &Context<S>, outcome: Outcome) -> Outcome
where
    S: Send + Sync + 'static,
{
    let pipeline: Arc<dyn crate::authoring::ErasedDeliveryPipeline> =
        Arc::new(crate::authoring::BoundDeliveryPipeline {
            runtime: context.authoring_arc(),
            context: context.clone(),
        });
    outcome.with_delivery_pipeline(pipeline)
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

    fn command_id(&self) -> Option<crate::CommandId> {
        self.command.as_ref().map(Command::id)
    }

    fn call(&self, call: HandlerCall<S>) -> BoxFuture<'static, HandlerResult<Outcome>> {
        let HandlerCall {
            event,
            state,
            bot,
            sessions,
            shutdown,
            authoring,
            command_input,
            responder,
        } = call;
        let endpoint = Arc::clone(&self.endpoint);
        let guards = Arc::clone(&self.guards);
        let before = Arc::clone(&self.before);
        let after = Arc::clone(&self.after);
        let command = self.command.clone();
        let branch_filter = self.branch_filter.clone();

        Box::pin(async move {
            let base_context = Context::new(
                Arc::clone(&event),
                Arc::clone(&state),
                bot.clone(),
                sessions.clone(),
                shutdown.clone(),
                None,
                responder.clone(),
                Arc::clone(&authoring),
            );
            let command = if let Some(command) = command {
                if !authoring.registry.is_enabled(command.id()) {
                    return Ok(Outcome::continue_());
                }
                match match_command_event(
                    &command,
                    event.event(),
                    &base_context,
                    command_input.as_deref(),
                )
                .await
                {
                    Ok(Some(result)) => Some(result),
                    Ok(None) => return Ok(Outcome::continue_()),
                    Err(CommandEventError::Parse(error)) => {
                        let outcome =
                            command_parse_outcome(&base_context, &command, &[], error).await?;
                        return Ok(bind_delivery(&base_context, outcome));
                    }
                    Err(CommandEventError::Handler(error)) => return Err(error),
                }
            } else {
                None
            };
            if let (Some((filter, descendants)), Some(command)) = (&branch_filter, &command) {
                let matches = if *descendants {
                    command.branch_names().len() >= filter.len()
                        && command
                            .branch_names()
                            .iter()
                            .take(filter.len())
                            .zip(filter.iter())
                            .all(|(actual, expected)| actual.eq_ignore_ascii_case(expected))
                } else {
                    let expected = filter.iter().map(AsRef::as_ref).collect::<Vec<_>>();
                    command.is_branch(&expected)
                };
                if !matches {
                    return Ok(Outcome::continue_());
                }
            }

            let mut context = Context::new(
                event, state, bot, sessions, shutdown, command, responder, authoring,
            );

            // Admission runs before interactive completion. An unauthorized or
            // rate-limited user must never be prompted for missing arguments.
            for guard in guards.iter() {
                match guard.check(context.clone()).await? {
                    GuardDecision::Allow => {}
                    GuardDecision::Skip => return Ok(Outcome::continue_()),
                    GuardDecision::Deny(outcome) => {
                        return Ok(bind_delivery(
                            &context,
                            outcome.with_propagation(crate::Propagation::Stop),
                        ));
                    }
                }
            }

            if let Some((input, cursor)) = explicit_completion_input(context.event()) {
                if context.command().is_some() {
                    let outcome = command_completion_outcome(&context, &input, cursor).await?;
                    return Ok(bind_delivery(&context, outcome));
                }
            }
            if context.command().is_some() {
                match prepare_command(context.clone()).await? {
                    CommandPreparation::Ready(ready) => context = ready,
                    CommandPreparation::Respond(outcome) => {
                        return Ok(bind_delivery(&context, outcome));
                    }
                }
            }
            for hook in before.iter() {
                hook.call(context.clone()).await?;
            }

            let mut outcome = endpoint.call(context.clone()).await?;
            for hook in after.iter() {
                outcome = hook.call(context.clone(), outcome).await?;
            }
            Ok(bind_delivery(&context, outcome))
        })
    }
}
