use crate::{
    adapter::InterestPlan,
    handler::{PreparedHandler, RouteSpec},
    session::SessionRegistry,
    BotHandle, Filter, MetricsHandle, ShutdownSignal,
};
use futures_util::FutureExt;
use oxidebot_core::{BotIdentity, EventEnvelope, EventKind, MessageTarget, PlatformId};
use std::{
    collections::HashMap,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::Arc,
    time::Duration,
};

type RouteId = u32;

#[derive(Clone, Copy)]
pub(crate) struct RouterLimits {
    pub(crate) handler_timeout: Option<Duration>,
    pub(crate) max_handler_replies: usize,
}

enum RouteList {
    One(RouteId),
    Many(Vec<RouteId>),
}

impl RouteList {
    fn push(&mut self, id: RouteId) {
        match self {
            Self::One(first) => *self = Self::Many(vec![*first, id]),
            Self::Many(values) => values.push(id),
        }
    }

    fn as_slice(&self) -> &[RouteId] {
        match self {
            Self::One(id) => std::slice::from_ref(id),
            Self::Many(values) => values.as_slice(),
        }
    }
}

struct RouteTables {
    generic: Vec<Vec<RouteId>>,
    commands: HashMap<Arc<str>, RouteList>,
    interactions: HashMap<Arc<str>, RouteList>,
    native: HashMap<Arc<str>, RouteList>,
}

fn insert_exact(table: &mut HashMap<Arc<str>, RouteList>, key: &Arc<str>, id: RouteId) {
    use std::collections::hash_map::Entry;
    match table.entry(key.clone()) {
        Entry::Vacant(entry) => {
            entry.insert(RouteList::One(id));
        }
        Entry::Occupied(mut entry) => entry.get_mut().push(id),
    }
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
            RouteSpec::Command(command) => insert_exact(&mut self.commands, command, id),
            RouteSpec::Interaction(custom_id) => {
                insert_exact(&mut self.interactions, custom_id, id)
            }
            RouteSpec::Native(kind) => insert_exact(&mut self.native, kind, id),
        }
    }

    fn exact<'a>(&'a self, event: &EventEnvelope) -> &'a [RouteId] {
        match event.index.kind {
            EventKind::MessageCreated => event
                .index
                .command
                .as_ref()
                .and_then(|value| self.commands.get(value))
                .map_or(&[], RouteList::as_slice),
            EventKind::Interaction => event
                .index
                .interaction
                .as_ref()
                .and_then(|value| self.interactions.get(value))
                .map_or(&[], RouteList::as_slice),
            EventKind::Native => event
                .index
                .native_type
                .as_ref()
                .and_then(|value| self.native.get(value))
                .map_or(&[], RouteList::as_slice),
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

struct ScopedRouteTables {
    global: RouteTables,
    platforms: HashMap<PlatformId, RouteTables>,
    bots: HashMap<BotIdentity, RouteTables>,
}

impl ScopedRouteTables {
    fn new() -> Self {
        Self {
            global: RouteTables::new(),
            platforms: HashMap::new(),
            bots: HashMap::new(),
        }
    }

    fn insert(&mut self, id: RouteId, spec: &RouteSpec, scope: &crate::handler::RouteScope) {
        if let Some(bot) = &scope.bot {
            self.bots
                .entry(bot.clone())
                .or_insert_with(RouteTables::new)
                .insert(id, spec);
        } else if let Some(platform) = &scope.platform {
            self.platforms
                .entry(platform.clone())
                .or_insert_with(RouteTables::new)
                .insert(id, spec);
        } else {
            self.global.insert(id, spec);
        }
    }

    fn candidates<'a>(
        &'a self,
        event: &EventEnvelope,
        identity: &BotIdentity,
    ) -> [&'a [RouteId]; 6] {
        let empty: &'a [RouteId] = &[];
        let platform = self.platforms.get(&identity.platform);
        let bot = self.bots.get(identity);
        [
            &self.global.generic[event.index.kind as usize],
            self.global.exact(event),
            platform.map_or(empty, |routes| {
                routes.generic[event.index.kind as usize].as_slice()
            }),
            platform.map_or(empty, |routes| routes.exact(event)),
            bot.map_or(empty, |routes| {
                routes.generic[event.index.kind as usize].as_slice()
            }),
            bot.map_or(empty, |routes| routes.exact(event)),
        ]
    }
}

fn next_route_id(lists: &[&[RouteId]; 6], positions: &mut [usize; 6]) -> Option<RouteId> {
    let mut selected: Option<(usize, RouteId)> = None;
    for (index, list) in lists.iter().enumerate() {
        let Some(&route_id) = list.get(positions[index]) else {
            continue;
        };
        if selected.is_none_or(|(_, current)| route_id < current) {
            selected = Some((index, route_id));
        }
    }
    let (index, route_id) = selected?;
    positions[index] = positions[index].saturating_add(1);
    Some(route_id)
}

/// Immutable router compiled into candidate indexes before adapters start.
pub(crate) struct CompiledRouter<S>
where
    S: Send + Sync + 'static,
{
    handlers: Box<[PreparedHandler<S>]>,
    routes: ScopedRouteTables,
    filters: Box<[Arc<dyn Filter<S>>]>,
    state: Arc<S>,
    sessions: SessionRegistry,
    shutdown: ShutdownSignal,
    handler_timeout: Option<Duration>,
    max_handler_replies: usize,
    metrics: MetricsHandle,
    interest: InterestPlan,
}

impl<S> CompiledRouter<S>
where
    S: Send + Sync + 'static,
{
    pub(crate) fn compile(
        handlers: Vec<PreparedHandler<S>>,
        filters: Vec<Arc<dyn Filter<S>>>,
        state: Arc<S>,
        sessions: SessionRegistry,
        shutdown: ShutdownSignal,
        limits: RouterLimits,
        metrics: MetricsHandle,
    ) -> Self {
        let mut routes = ScopedRouteTables::new();
        let mut interest = InterestPlan::default();
        for (index, handler) in handlers.iter().enumerate() {
            let id = u32::try_from(index).expect("handler count must fit in u32");
            routes.insert(id, &handler.spec, &handler.scope);
            interest.add_route(&handler.spec, &handler.scope);
        }
        interest.set_session_interest(sessions.interest());
        Self {
            handlers: handlers.into_boxed_slice(),
            routes,
            filters: filters.into_boxed_slice(),
            state,
            sessions,
            shutdown,
            handler_timeout: limits.handler_timeout,
            max_handler_replies: limits.max_handler_replies,
            metrics,
            interest,
        }
    }

    pub(crate) fn interest_for(&self, identity: BotIdentity) -> InterestPlan {
        self.interest.bind(identity)
    }

    /// Dispatches only the pre-indexed candidates. Generic and exact lists are
    /// merged by RouteId so registration order and `Outcome::stop()` semantics
    /// remain deterministic without allocating a temporary candidate vector.
    pub(crate) async fn dispatch(&self, event: Arc<EventEnvelope>, bot: BotHandle) {
        let identity = bot.identity();
        let candidates = self.routes.candidates(&event, identity);
        let candidate_count = candidates.iter().map(|routes| routes.len()).sum::<usize>();
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

        let mut positions = [0_usize; 6];
        while let Some(route_id) = next_route_id(&candidates, &mut positions) {
            let prepared = &self.handlers[route_id as usize];
            debug_assert!(prepared.scope.matches(identity));
            let handler = &prepared.handler;
            self.metrics.handler_call();
            let future = match catch_unwind(AssertUnwindSafe(|| {
                handler.call(
                    Arc::clone(&event),
                    Arc::clone(&self.state),
                    bot.clone(),
                    self.sessions.clone(),
                    self.shutdown.clone(),
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
            let stop = outcome.stop;
            if outcome.replies.len() > self.max_handler_replies {
                self.metrics.handler_effect_rejection();
                tracing::error!(
                    event_id = %event.id,
                    replies = outcome.replies.len(),
                    limit = self.max_handler_replies,
                    "handler outcome exceeded the deferred-reply limit"
                );
            } else if let Some(conversation) = event.index.conversation.clone() {
                let target = MessageTarget::new(conversation);
                for reply in outcome.replies {
                    if let Err(error) = bot.enqueue_send(target.clone(), reply).await {
                        tracing::warn!(event_id = %event.id, %error, "could not enqueue handler reply");
                    }
                }
            }
            if stop {
                break;
            }
        }
    }
}
