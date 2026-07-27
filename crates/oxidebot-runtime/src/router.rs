use crate::{
    adapter::InterestPlan, handler::RouteSpec, session::SessionRegistry, BotHandle, ErasedHandler,
    Filter, MetricsHandle, ShutdownSignal,
};
use futures_util::FutureExt;
use oxidebot_core::{EventEnvelope, EventKind, MessageTarget};
use std::{
    collections::HashMap,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::Arc,
    time::Duration,
};
use tokio_util::sync::CancellationToken;

type RouteId = u32;

struct RouteTables {
    generic: Vec<Vec<RouteId>>,
    commands: HashMap<Arc<str>, Vec<RouteId>>,
    interactions: HashMap<Arc<str>, Vec<RouteId>>,
    native: HashMap<Arc<str>, Vec<RouteId>>,
}

impl RouteTables {
    fn new() -> Self {
        Self {
            generic: (0..EventKind::BIT_COUNT).map(|_| Vec::new()).collect(),
            commands: HashMap::new(),
            interactions: HashMap::new(),
            native: HashMap::new(),
        }
    }

    fn insert(&mut self, id: RouteId, spec: &RouteSpec) {
        match spec {
            RouteSpec::Generic(kind) => self.generic[*kind as usize].push(id),
            RouteSpec::Command(command) => {
                self.commands.entry(command.clone()).or_default().push(id);
            }
            RouteSpec::Interaction(custom_id) => {
                self.interactions
                    .entry(custom_id.clone())
                    .or_default()
                    .push(id);
            }
            RouteSpec::Native(kind) => {
                self.native.entry(kind.clone()).or_default().push(id);
            }
        }
    }

    fn exact<'a>(&'a self, event: &EventEnvelope) -> &'a [RouteId] {
        match event.index.kind {
            EventKind::MessageCreated => event
                .index
                .command
                .as_ref()
                .and_then(|value| self.commands.get(value))
                .map_or(&[], Vec::as_slice),
            EventKind::Interaction => event
                .index
                .interaction
                .as_ref()
                .and_then(|value| self.interactions.get(value))
                .map_or(&[], Vec::as_slice),
            EventKind::Native => event
                .index
                .native_type
                .as_ref()
                .and_then(|value| self.native.get(value))
                .map_or(&[], Vec::as_slice),
            EventKind::MessageUpdated
            | EventKind::MessagesDeleted
            | EventKind::ReactionChanged
            | EventKind::MemberChanged
            | EventKind::ConversationChanged
            | EventKind::FileChanged
            | EventKind::PaymentChanged => &[],
        }
    }
}

/// Immutable router compiled into candidate indexes before adapters start.
pub(crate) struct CompiledRouter<S>
where
    S: Send + Sync + 'static,
{
    handlers: Box<[Arc<dyn ErasedHandler<S>>]>,
    routes: RouteTables,
    filters: Box<[Arc<dyn Filter<S>>]>,
    state: Arc<S>,
    sessions: SessionRegistry,
    cancellation: CancellationToken,
    handler_timeout: Option<Duration>,
    metrics: MetricsHandle,
    interest: InterestPlan,
}

impl<S> CompiledRouter<S>
where
    S: Send + Sync + 'static,
{
    pub(crate) fn compile(
        handlers: Vec<Arc<dyn ErasedHandler<S>>>,
        filters: Vec<Arc<dyn Filter<S>>>,
        state: Arc<S>,
        sessions: SessionRegistry,
        cancellation: CancellationToken,
        handler_timeout: Option<Duration>,
        metrics: MetricsHandle,
    ) -> Self {
        let mut routes = RouteTables::new();
        let mut interest = InterestPlan::default();
        for (index, handler) in handlers.iter().enumerate() {
            let id = u32::try_from(index).expect("handler count must fit in u32");
            let spec = handler.route_spec();
            routes.insert(id, &spec);
            interest.add_route(&spec);
        }
        interest.set_session_interest(sessions.interest());
        Self {
            handlers: handlers.into_boxed_slice(),
            routes,
            filters: filters.into_boxed_slice(),
            state,
            sessions,
            cancellation,
            handler_timeout,
            metrics,
            interest,
        }
    }

    pub(crate) fn interest(&self) -> InterestPlan {
        self.interest.clone()
    }

    /// Dispatches only the pre-indexed candidates. Generic and exact lists are
    /// merged by RouteId so registration order and `Outcome::stop()` semantics
    /// remain deterministic without allocating a temporary candidate vector.
    pub(crate) async fn dispatch(&self, event: Arc<EventEnvelope>, bot: BotHandle) {
        let generic = &self.routes.generic[event.index.kind as usize];
        let exact = self.routes.exact(&event);
        let candidate_count = generic.len().saturating_add(exact.len());
        self.metrics.route_candidates(candidate_count);
        if candidate_count == 0 {
            return;
        }

        for filter in &self.filters {
            match catch_unwind(AssertUnwindSafe(|| filter.accepts(&event, &self.state))) {
                Ok(true) => {}
                Ok(false) => return,
                Err(_) => {
                    self.metrics.filter_panic();
                    tracing::error!(event_id = %event.id, "global filter panicked");
                    return;
                }
            }
        }

        let mut generic_index = 0;
        let mut exact_index = 0;
        while generic_index < generic.len() || exact_index < exact.len() {
            let route_id = match (generic.get(generic_index), exact.get(exact_index)) {
                (Some(left), Some(right)) if left <= right => {
                    generic_index += 1;
                    *left
                }
                (Some(_), Some(right)) => {
                    exact_index += 1;
                    *right
                }
                (Some(left), None) => {
                    generic_index += 1;
                    *left
                }
                (None, Some(right)) => {
                    exact_index += 1;
                    *right
                }
                (None, None) => break,
            };

            let handler = &self.handlers[route_id as usize];
            self.metrics.handler_call();
            let future = match catch_unwind(AssertUnwindSafe(|| {
                handler.call(
                    Arc::clone(&event),
                    Arc::clone(&self.state),
                    bot.clone(),
                    self.sessions.clone(),
                    ShutdownSignal::new(self.cancellation.child_token()),
                )
            })) {
                Ok(future) => future,
                Err(_) => {
                    self.metrics.handler_panic();
                    tracing::error!(event_id = %event.id, "handler panicked while creating its future");
                    continue;
                }
            };
            let result = if let Some(timeout) = self.handler_timeout {
                match tokio::time::timeout(timeout, AssertUnwindSafe(future).catch_unwind()).await {
                    Ok(result) => result,
                    Err(_) => {
                        self.metrics.handler_timeout();
                        tracing::warn!(event_id = %event.id, "handler timed out");
                        continue;
                    }
                }
            } else {
                AssertUnwindSafe(future).catch_unwind().await
            };
            let outcome = match result {
                Ok(Ok(outcome)) => outcome,
                Ok(Err(error)) => {
                    tracing::warn!(event_id = %event.id, %error, "handler failed");
                    continue;
                }
                Err(_) => {
                    self.metrics.handler_panic();
                    tracing::error!(event_id = %event.id, "handler panicked");
                    continue;
                }
            };
            if let Some(conversation) = event.index.conversation.clone() {
                let target = MessageTarget::new(conversation);
                for reply in outcome.replies {
                    if let Err(error) = bot.enqueue_send(target.clone(), reply).await {
                        tracing::warn!(event_id = %event.id, %error, "could not enqueue handler reply");
                    }
                }
            }
            if outcome.stop {
                break;
            }
        }
    }
}
