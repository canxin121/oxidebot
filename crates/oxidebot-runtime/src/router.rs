use crate::{
    adapter::InterestPlan,
    handler::{HandlerCall, PreparedHandler, RouteSpec},
    session::SessionRegistry,
    BotHandle, CommandId, Filter, MetricsHandle, Outcome, ShutdownSignal,
};
use futures_util::FutureExt;
use oxidebot_core::event::kernel::{DispatchEnvelope, DispatchKind};
use oxidebot_core::{
    conversation::{ConversationRef, MessageTarget},
    event::{EventType, LifecycleEvent, RequestEvent},
    BotIdentity, Event, PlatformId,
};
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    panic::{catch_unwind, AssertUnwindSafe},
    sync::Arc,
    time::{Duration, Instant},
};

type RouteId = u32;

#[derive(Clone, Copy)]
pub(crate) struct RouterLimits {
    pub(crate) handler_timeout: Option<Duration>,
    pub(crate) max_handler_replies: usize,
}

pub(crate) struct RouterRuntime<S>
where
    S: Send + Sync + 'static,
{
    pub(crate) state: Arc<S>,
    pub(crate) sessions: SessionRegistry,
    pub(crate) shutdown: ShutdownSignal,
    pub(crate) metrics: MetricsHandle,
    pub(crate) authoring: Arc<crate::authoring::AuthoringRuntime<S>>,
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
    events: Vec<Vec<RouteId>>,
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
            events: (0..EventType::COUNT).map(|_| Vec::new()).collect(),
            commands: HashMap::new(),
            interactions: HashMap::new(),
            native: HashMap::new(),
        }
    }

    fn insert(&mut self, id: RouteId, spec: &RouteSpec) {
        match spec {
            RouteSpec::Event(event_type) => self.events[*event_type as usize].push(id),
            RouteSpec::Command(command) => insert_exact(&mut self.commands, command, id),
            RouteSpec::Interaction(custom_id) => {
                insert_exact(&mut self.interactions, custom_id, id)
            }
            RouteSpec::Native(kind) => insert_exact(&mut self.native, kind, id),
        }
    }

    fn exact<'a>(&'a self, event: &DispatchEnvelope) -> &'a [RouteId] {
        match event.index.kind {
            DispatchKind::Message => event
                .index
                .command
                .as_ref()
                .and_then(|value| self.commands.get(value))
                .map_or(&[], RouteList::as_slice),
            DispatchKind::Interaction => event
                .index
                .interaction
                .as_ref()
                .and_then(|value| self.interactions.get(value))
                .map_or(&[], RouteList::as_slice),
            DispatchKind::Native => event
                .index
                .native_type
                .as_ref()
                .and_then(|value| self.native.get(value))
                .map_or(&[], RouteList::as_slice),
            DispatchKind::MessageUpdate
            | DispatchKind::MessageDelete
            | DispatchKind::Reaction
            | DispatchKind::Member
            | DispatchKind::Conversation
            | DispatchKind::File
            | DispatchKind::Payment => &[],
        }
    }

    fn event<'a>(&'a self, envelope: &DispatchEnvelope) -> &'a [RouteId] {
        self.events[envelope.index.event_type as usize].as_slice()
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
        event: &DispatchEnvelope,
        identity: &BotIdentity,
    ) -> [&'a [RouteId]; 6] {
        let empty: &'a [RouteId] = &[];
        let platform = self.platforms.get(&identity.platform);
        let bot = self.bots.get(identity);
        [
            self.global.event(event),
            self.global.exact(event),
            platform.map_or(empty, |routes| routes.event(event)),
            platform.map_or(empty, |routes| routes.exact(event)),
            bot.map_or(empty, |routes| routes.event(event)),
            bot.map_or(empty, |routes| routes.exact(event)),
        ]
    }
}

fn next_route_id<const N: usize>(
    lists: &[&[RouteId]; N],
    positions: &mut [usize; N],
) -> Option<RouteId> {
    let route_id = lists
        .iter()
        .enumerate()
        .filter_map(|(index, list)| list.get(positions[index]).copied())
        .min()?;
    for (index, list) in lists.iter().enumerate() {
        if list.get(positions[index]).copied() == Some(route_id) {
            positions[index] = positions[index].saturating_add(1);
        }
    }
    Some(route_id)
}

pub(crate) fn reply_target(event: &Event) -> Option<MessageTarget> {
    fn target(conversation: &ConversationRef) -> MessageTarget {
        MessageTarget::new(conversation.clone())
    }

    fn direct(id: &str) -> MessageTarget {
        MessageTarget::new(ConversationRef::direct(id.to_owned()))
    }

    match event {
        Event::Message(event) => Some(target(&event.conversation)),
        Event::Lifecycle(event) => match event {
            LifecycleEvent::GroupMemberJoined(event) => Some(target(&event.conversation)),
            LifecycleEvent::GroupMemberLeft(event) => Some(target(&event.conversation)),
            LifecycleEvent::GroupAdminChanged(event) => Some(target(&event.conversation)),
            LifecycleEvent::GroupMuteChanged(event) => Some(target(&event.conversation)),
            LifecycleEvent::GroupMemberMuteChanged(event) => Some(target(&event.conversation)),
            LifecycleEvent::GroupHighlightChanged(event) => Some(target(&event.conversation)),
            LifecycleEvent::GroupMemberAliasChanged(event) => Some(target(&event.conversation)),
            LifecycleEvent::MessageReactionsChanged(event) => Some(
                event
                    .conversation
                    .as_ref()
                    .map(target)
                    .unwrap_or_else(|| direct(&event.user.id)),
            ),
            LifecycleEvent::MessageDeleted(event) => event
                .conversation
                .as_ref()
                .map(target)
                .or_else(|| event.user.as_ref().map(|user| direct(&user.id))),
            LifecycleEvent::MessageEdited(event) => Some(
                event
                    .conversation
                    .as_ref()
                    .map(target)
                    .unwrap_or_else(|| direct(&event.user.id)),
            ),
            _ => None,
        },
        Event::Request(RequestEvent::Friend(event)) => Some(direct(&event.user.id)),
        Event::Request(RequestEvent::GroupJoin(event)) => Some(target(&event.conversation)),
        Event::Request(RequestEvent::GroupInvite(event)) => Some(direct(&event.user.id)),
        Event::Interaction(event) => Some(
            event
                .conversation
                .as_ref()
                .map(target)
                .unwrap_or_else(|| direct(&event.user.id)),
        ),
        Event::Meta(_) | Event::Native(_) => None,
    }
}

/// Immutable dispatch table compiled into candidate indexes before adapters start.
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
    authoring: Arc<crate::authoring::AuthoringRuntime<S>>,
    command_routes: HashMap<CommandId, Vec<RouteId>>,
    all_command_routes: Vec<RouteId>,
    interest: InterestPlan,
}

impl<S> CompiledRouter<S>
where
    S: Send + Sync + 'static,
{
    pub(crate) fn compile(
        handlers: Vec<PreparedHandler<S>>,
        filters: Vec<Arc<dyn Filter<S>>>,
        runtime: RouterRuntime<S>,
        limits: RouterLimits,
    ) -> Self {
        let RouterRuntime {
            state,
            sessions,
            shutdown,
            metrics,
            authoring,
        } = runtime;
        let mut routes = ScopedRouteTables::new();
        let mut command_routes: HashMap<CommandId, Vec<RouteId>> = HashMap::new();
        let mut all_command_routes = Vec::new();
        let mut interest = InterestPlan::default();
        for (index, handler) in handlers.iter().enumerate() {
            let id = u32::try_from(index).expect("handler count must fit in u32");
            routes.insert(id, &handler.spec, &handler.scope);
            interest.add_route(&handler.spec, &handler.scope);
            if let Some(command_id) = handler.command_id {
                command_routes.entry(command_id).or_default().push(id);
                all_command_routes.push(id);
            }
        }
        if !all_command_routes.is_empty()
            && (authoring.registry.dynamic_shortcuts_enabled()
                || authoring.registry.broad_command_matching())
        {
            interest.add_route(
                &RouteSpec::Event(EventType::Message),
                &crate::handler::RouteScope::global(),
            );
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
            authoring,
            command_routes,
            all_command_routes,
            interest,
        }
    }

    pub(crate) fn interest_for(&self, identity: BotIdentity) -> InterestPlan {
        self.interest.bind(identity)
    }

    /// Dispatches only the pre-indexed candidates. Event and exact lists are
    /// merged by RouteId so registration order and explicit blocking semantics
    /// remain deterministic without allocating a temporary candidate vector.
    pub(crate) async fn dispatch(&self, event: Arc<DispatchEnvelope>, bot: BotHandle) {
        let identity = bot.identity();
        let base_candidates = self.routes.candidates(&event, identity);
        let mut dynamic_candidates = Vec::new();
        if matches!(event.index.kind, DispatchKind::Message) {
            if self.authoring.registry.broad_command_matching() {
                dynamic_candidates.extend_from_slice(&self.all_command_routes);
            } else if self.authoring.registry.dynamic_shortcuts_enabled() {
                if let Event::Message(message) = event.event() {
                    let input = message.message.extract_plain_text();
                    for command_id in self
                        .authoring
                        .registry
                        .matching_shortcut_commands(&input, 64)
                    {
                        if let Some(routes) = self.command_routes.get(&command_id) {
                            dynamic_candidates.extend_from_slice(routes);
                        }
                    }
                }
            }
        }
        dynamic_candidates.sort_unstable();
        dynamic_candidates.dedup();
        let candidates = [
            base_candidates[0],
            base_candidates[1],
            base_candidates[2],
            base_candidates[3],
            base_candidates[4],
            base_candidates[5],
            dynamic_candidates.as_slice(),
        ];
        let candidate_count = candidates.iter().map(|routes| routes.len()).sum::<usize>();
        self.metrics.route_candidates(candidate_count);
        if candidate_count == 0 {
            return;
        }

        for filter in &self.filters {
            match catch_unwind(AssertUnwindSafe(|| {
                filter.accepts(event.event(), &self.state)
            })) {
                Ok(true) => {}
                Ok(false) => return,
                Err(_) => {
                    self.metrics.filter_panic();
                    tracing::error!(event_id = %event.id, "global filter panicked");
                    return;
                }
            }
        }

        let responder = match event.event() {
            Event::Interaction(interaction) => interaction
                .response
                .clone()
                .map(|handle| crate::Responder::new(bot.clone(), handle)),
            _ => None,
        };
        if let Some(responder) = &responder {
            responder.start_auto_defer();
        }

        let command_input = self
            .authoring
            .registry
            .broad_command_matching()
            .then(|| Arc::new(tokio::sync::OnceCell::new()));
        let mut positions = [0_usize; 7];
        while let Some(route_id) = next_route_id(&candidates, &mut positions) {
            let prepared = &self.handlers[route_id as usize];
            if !prepared.scope.matches(identity) {
                continue;
            }
            let handler = &prepared.handler;
            self.metrics.handler_call();
            self.metrics.handler_started();
            let handler_started = Instant::now();
            let future = match catch_unwind(AssertUnwindSafe(|| {
                handler.call(HandlerCall {
                    event: Arc::clone(&event),
                    state: Arc::clone(&self.state),
                    bot: bot.clone(),
                    sessions: self.sessions.clone(),
                    shutdown: self.shutdown.clone(),
                    authoring: Arc::clone(&self.authoring),
                    command_input: command_input.clone(),
                    responder: responder.clone(),
                })
            })) {
                Ok(future) => future,
                Err(_) => {
                    self.metrics.handler_finished(handler_started.elapsed());
                    self.metrics.handler_panic();
                    tracing::error!(event_id = %event.id, "handler panicked while creating its future");
                    if prepared.default_block {
                        break;
                    }
                    continue;
                }
            };
            let result = if let Some(timeout) = self.handler_timeout {
                match tokio::time::timeout(timeout, AssertUnwindSafe(future).catch_unwind()).await {
                    Ok(result) => result,
                    Err(_) => {
                        self.metrics.handler_finished(handler_started.elapsed());
                        self.metrics.handler_timeout();
                        tracing::warn!(event_id = %event.id, "handler timed out");
                        if prepared.default_block {
                            break;
                        }
                        continue;
                    }
                }
            } else {
                AssertUnwindSafe(future).catch_unwind().await
            };
            self.metrics.handler_finished(handler_started.elapsed());
            let outcome = match result {
                Ok(Ok(outcome)) => outcome,
                Ok(Err(error)) => {
                    if let Some(message) = error.user_message() {
                        Outcome::new().text(message.to_owned())
                    } else {
                        tracing::warn!(event_id = %event.id, %error, "handler failed");
                        if prepared.default_block {
                            break;
                        }
                        continue;
                    }
                }
                Err(_) => {
                    self.metrics.handler_panic();
                    tracing::error!(event_id = %event.id, "handler panicked");
                    if prepared.default_block {
                        break;
                    }
                    continue;
                }
            }
            .resolve(prepared.default_block);
            let stop = outcome.is_stopped();
            let delivery_pipeline = outcome.delivery_pipeline();
            if outcome.replies.len() > self.max_handler_replies {
                self.metrics.handler_effect_rejection();
                tracing::error!(
                    event_id = %event.id,
                    replies = outcome.replies.len(),
                    limit = self.max_handler_replies,
                    "handler outcome exceeded the deferred-reply limit"
                );
            } else if let Some(responder) = &responder {
                for reply in outcome.replies {
                    let visibility = match reply.options.visibility {
                        oxidebot_core::content::MessageVisibility::Ephemeral => {
                            oxidebot_core::InteractionVisibility::Ephemeral
                        }
                        _ => oxidebot_core::InteractionVisibility::Public,
                    };
                    match AssertUnwindSafe(responder.respond_with(reply, visibility))
                        .catch_unwind()
                        .await
                    {
                        Ok(Ok(_)) => {}
                        Ok(Err(error)) => {
                            self.metrics.delivery_error();
                            tracing::warn!(
                                event_id = %event.id,
                                %error,
                                "could not send automatic interaction response"
                            );
                        }
                        Err(_) => {
                            self.metrics.delivery_panic();
                            tracing::error!(
                                event_id = %event.id,
                                "interaction response panicked"
                            );
                        }
                    }
                }
            } else if let Some(target) = reply_target(event.event()) {
                for (reply_index, mut reply) in outcome.replies.into_iter().enumerate() {
                    if reply.options.idempotency_key.is_none()
                        && bot.capabilities().send_idempotency
                            != crate::IdempotencyGuarantee::Unsupported
                    {
                        let mut hasher = std::collections::hash_map::DefaultHasher::new();
                        event.id.hash(&mut hasher);
                        reply.options.idempotency_key = Some(format!(
                            "oxidebot-event-{:016x}-{reply_index}",
                            hasher.finish()
                        ));
                    }
                    let delivery = if let Some(pipeline) = &delivery_pipeline {
                        AssertUnwindSafe(pipeline.deliver(
                            &bot,
                            target.clone(),
                            reply,
                            oxidebot_core::FallbackPolicy::Auto,
                        ))
                        .catch_unwind()
                        .await
                    } else {
                        AssertUnwindSafe(self.authoring.deliver(
                            None,
                            &bot,
                            target.clone(),
                            reply,
                            oxidebot_core::FallbackPolicy::Auto,
                        ))
                        .catch_unwind()
                        .await
                    };
                    match delivery {
                        Ok(Ok(report)) => {
                            if report.degraded() {
                                self.metrics.delivery_degradation();
                                tracing::warn!(
                                    event_id = %event.id,
                                    degradations = ?report.degradations,
                                    "handler reply required delivery fallbacks"
                                );
                            }
                        }
                        Ok(Err(error)) => {
                            if matches!(error, crate::HandlerError::DeliveryPanicked) {
                                self.metrics.delivery_panic();
                            } else {
                                self.metrics.delivery_error();
                            }
                            tracing::warn!(event_id = %event.id, %error, "could not send handler reply");
                        }
                        Err(_) => {
                            self.metrics.delivery_panic();
                            tracing::error!(
                                event_id = %event.id,
                                "delivery middleware or adapter panicked while sending a handler reply"
                            );
                        }
                    }
                }
            }
            if stop {
                break;
            }
        }
    }
}
