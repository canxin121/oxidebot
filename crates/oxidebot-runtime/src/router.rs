use crate::{
    adapter::InterestPlan,
    handler::{PreparedHandler, RouteSpec},
    session::SessionRegistry,
    BotHandle, Filter, MetricsHandle, Outcome, ShutdownSignal,
};
use futures_util::FutureExt;
use oxidebot_core::event::kernel::{DispatchEnvelope, DispatchKind};
use oxidebot_core::{
    conversation::{ConversationRef, MessageTarget},
    event::{EventType, NoticeEvent, RequestEvent},
    BotIdentity, Event, PlatformId,
};
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

pub(crate) fn reply_target(event: &Event) -> Option<MessageTarget> {
    fn group(id: &str) -> MessageTarget {
        MessageTarget::new(ConversationRef::group(id.to_owned()))
    }

    fn direct(id: &str) -> MessageTarget {
        MessageTarget::new(ConversationRef::direct(id.to_owned()))
    }

    match event {
        Event::MessageEvent(event) => Some(
            event
                .group
                .as_ref()
                .map(|group_ref| group(&group_ref.id))
                .unwrap_or_else(|| direct(&event.sender.id)),
        ),
        Event::NoticeEvent(event) => match event {
            NoticeEvent::GroupMemberIncreaseEvent(event) => Some(group(&event.group.id)),
            NoticeEvent::GroupMemberDecreaseEvent(event) => Some(group(&event.group.id)),
            NoticeEvent::GroupAdminChangeEvent(event) => Some(group(&event.group.id)),
            NoticeEvent::GroupMuteChangeEvent(event) => Some(group(&event.group.id)),
            NoticeEvent::GroupMemberMuteChangeEvent(event) => Some(group(&event.group.id)),
            NoticeEvent::GroupHighlightChangeEvent(event) => Some(group(&event.group.id)),
            NoticeEvent::GroupMemberAliasChangeEvent(event) => Some(group(&event.group.id)),
            NoticeEvent::MessageReactionsEvent(event) => Some(
                event
                    .group
                    .as_ref()
                    .map(|group_ref| group(&group_ref.id))
                    .unwrap_or_else(|| direct(&event.user.id)),
            ),
            NoticeEvent::MessageDeletedEvent(event) => event
                .group
                .as_ref()
                .map(|group_ref| group(&group_ref.id))
                .or_else(|| event.user.as_ref().map(|user| direct(&user.id))),
            NoticeEvent::MessageEditedEvent(event) => Some(
                event
                    .group
                    .as_ref()
                    .map(|group_ref| group(&group_ref.id))
                    .unwrap_or_else(|| direct(&event.user.id)),
            ),
        },
        Event::RequestEvent(RequestEvent::FriendAddEvent(event)) => Some(direct(&event.user.id)),
        Event::RequestEvent(RequestEvent::GroupAddEvent(event)) => Some(group(&event.group.id)),
        Event::RequestEvent(RequestEvent::GroupInviteEvent(event)) => Some(direct(&event.user.id)),
        Event::InteractionEvent(event) => Some(
            event
                .group
                .as_ref()
                .map(|group_ref| group(&group_ref.id))
                .unwrap_or_else(|| direct(&event.user.id)),
        ),
        Event::LifecycleEvent(_) | Event::MetaEvent(_) | Event::AnyEvent(_) => None,
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

    /// Dispatches only the pre-indexed candidates. Event and exact lists are
    /// merged by RouteId so registration order and explicit blocking semantics
    /// remain deterministic without allocating a temporary candidate vector.
    pub(crate) async fn dispatch(&self, event: Arc<DispatchEnvelope>, bot: BotHandle) {
        let identity = bot.identity();
        let candidates = self.routes.candidates(&event, identity);
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
            if outcome.replies.len() > self.max_handler_replies {
                self.metrics.handler_effect_rejection();
                tracing::error!(
                    event_id = %event.id,
                    replies = outcome.replies.len(),
                    limit = self.max_handler_replies,
                    "handler outcome exceeded the deferred-reply limit"
                );
            } else if let Some(target) = reply_target(event.event()) {
                match bot.api() {
                    Ok(api) => {
                        for reply in outcome.replies {
                            match api
                                .send_outgoing_message_with(
                                    target.clone(),
                                    reply,
                                    oxidebot_core::FallbackPolicy::Auto,
                                )
                                .await
                            {
                                Ok(report) => {
                                    if report.degraded() {
                                        tracing::warn!(
                                            event_id = %event.id,
                                            degradations = ?report.degradations,
                                            "handler reply required delivery fallbacks"
                                        );
                                    }
                                }
                                Err(error) => {
                                    tracing::warn!(event_id = %event.id, %error, "could not send handler reply");
                                }
                            }
                        }
                    }
                    Err(error) => {
                        tracing::warn!(event_id = %event.id, %error, "adapter does not expose the OxideBot API");
                    }
                }
            }
            if stop {
                break;
            }
        }
    }
}
