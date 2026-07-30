use crate::handler::{RouteScope, RouteSpec};
use oxidebot_core::event::kernel::{DispatchIndex, DispatchKind};
use oxidebot_core::event::EventTypeSet;
use oxidebot_core::{BotIdentity, PlatformId};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

#[derive(Clone, Debug, Default)]
struct InterestSet {
    event_types: EventTypeSet,
    commands: HashSet<Arc<str>>,
    interactions: HashSet<Arc<str>>,
    native_types: HashSet<Arc<str>>,
}

impl InterestSet {
    fn accepts(&self, index: &DispatchIndex) -> bool {
        if self.event_types.contains(index.event_type) {
            return true;
        }
        match index.kind {
            DispatchKind::Message => index
                .command
                .as_ref()
                .is_some_and(|value| self.commands.contains(value)),
            DispatchKind::Interaction => index
                .interaction
                .as_ref()
                .is_some_and(|value| self.interactions.contains(value)),
            DispatchKind::Native => index
                .native_type
                .as_ref()
                .is_some_and(|value| self.native_types.contains(value)),
            DispatchKind::MessageUpdate
            | DispatchKind::MessageDelete
            | DispatchKind::Reaction
            | DispatchKind::Member
            | DispatchKind::Conversation
            | DispatchKind::File
            | DispatchKind::Payment => false,
        }
    }

    fn add_route(&mut self, route: &RouteSpec) {
        match route {
            RouteSpec::Event(event_type) => self.event_types.insert(*event_type),
            RouteSpec::Command(value) => {
                self.commands.insert(value.clone());
            }
            RouteSpec::Interaction(value) => {
                self.interactions.insert(value.clone());
            }
            RouteSpec::Native(value) => {
                self.native_types.insert(value.clone());
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
struct StaticInterestPlan {
    global: InterestSet,
    platforms: HashMap<PlatformId, InterestSet>,
    bots: HashMap<BotIdentity, InterestSet>,
}

/// Immutable route interest compiled before adapters start.
///
/// Static route sets are shared by `Arc`. Each adapter binds only its identity,
/// so platform- or bot-scoped routes do not cause unrelated transports to fully
/// decode matching traffic.
#[derive(Clone, Debug, Default)]
pub struct InterestPlan {
    static_plan: Arc<StaticInterestPlan>,
    identity: Option<BotIdentity>,
    session_interest: Option<crate::session::SessionInterest>,
}

impl InterestPlan {
    /// Returns whether an index can reach a route or exact active session.
    #[must_use]
    pub fn accepts(&self, index: &DispatchIndex) -> bool {
        if self.static_plan.global.accepts(index) {
            return true;
        }
        if let Some(identity) = &self.identity {
            if self
                .static_plan
                .platforms
                .get(&identity.platform)
                .is_some_and(|interest| interest.accepts(index))
                || self
                    .static_plan
                    .bots
                    .get(identity)
                    .is_some_and(|interest| interest.accepts(index))
            {
                return true;
            }
        }
        self.session_interest
            .as_ref()
            .is_some_and(|interest| interest.accepts(index))
    }

    pub(crate) fn add_route(&mut self, route: &RouteSpec, scope: &RouteScope) {
        let static_plan = Arc::make_mut(&mut self.static_plan);
        if let Some(bot) = &scope.bot {
            static_plan
                .bots
                .entry(bot.clone())
                .or_default()
                .add_route(route);
        } else if let Some(platform) = &scope.platform {
            static_plan
                .platforms
                .entry(platform.clone())
                .or_default()
                .add_route(route);
        } else {
            static_plan.global.add_route(route);
        }
    }

    pub(crate) fn bind(&self, identity: BotIdentity) -> Self {
        Self {
            static_plan: Arc::clone(&self.static_plan),
            identity: Some(identity),
            session_interest: self.session_interest.clone(),
        }
    }

    pub(crate) fn set_session_interest(&mut self, value: crate::session::SessionInterest) {
        self.session_interest = Some(value);
    }
}
