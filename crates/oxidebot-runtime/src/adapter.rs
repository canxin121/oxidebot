use crate::{
    budget::{HierarchicalLease, QueueAcquireError, QueueLease, QueueLimiter},
    handler::{RouteScope, RouteSpec},
    AdapterError, BotDescriptor, BotServices, DecodeError, MetricsHandle, QueueBudget,
    ShutdownSignal,
};
use async_trait::async_trait;
use oxidebot_core::event::kernel::{DispatchBatch, DispatchDraft, DispatchIndex, DispatchKind};
use oxidebot_core::event::{EventType, EventTypeSet, MessageEvent};
use oxidebot_core::{
    source::{message::Message, user::User},
    BotIdentity, BotSlot, CompactId, ConversationKey, ConversationRef, Event, EventId, PlatformId,
    RetainedSize, UserKey,
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::SystemTime,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Routing metadata extracted before a platform frame is fully decoded.
#[derive(Clone, Debug, Default)]
pub struct FrameIndex {
    pub events: Vec<DispatchIndex>,
    /// Conservative retained-byte upper bound for the fully decoded batch.
    pub estimated_bytes: usize,
}

impl FrameIndex {
    #[must_use]
    pub fn one(event: DispatchIndex, estimated_bytes: usize) -> Self {
        Self {
            events: vec![event],
            estimated_bytes,
        }
    }
}

/// Two-stage inbound platform frame.
pub trait InboundFrame: Send + 'static {
    /// Extracts only fields needed by interest gating and routing.
    fn index(
        &self,
        bot: BotSlot,
        platform: &PlatformId,
    ) -> std::result::Result<FrameIndex, DecodeError>;

    /// Fully decodes the frame exactly once after bounded admission.
    fn decode(
        self,
        bot: BotSlot,
        platform: &PlatformId,
    ) -> std::result::Result<DispatchBatch, DecodeError>;

    /// Decodes while receiving the already validated routing index.
    ///
    /// The default preserves the simple adapter API. High-throughput adapters
    /// can override this method to reuse command IDs, conversation keys, JSON
    /// offsets, or other work produced during [`Self::index`] instead of
    /// parsing the raw frame twice.
    fn decode_indexed(
        self,
        bot: BotSlot,
        platform: &PlatformId,
        _index: &FrameIndex,
    ) -> std::result::Result<DispatchBatch, DecodeError>
    where
        Self: Sized,
    {
        self.decode(bot, platform)
    }
}

/// A single already-decoded canonical OxideBot event.
///
/// This is the normal inbound boundary for adapters, importers, webhook
/// handlers, and tests which have already mapped their wire payload into the
/// public [`Event`] hierarchy.  The runtime derives the routing index,
/// conversation partition, command key, interaction key, and native event key
/// from that event; adapter authors do not need to use the hidden dispatch
/// kernel types.
///
/// [`InboundFrame`] remains the escape hatch for high-throughput transports
/// that can cheaply index a wire payload before decoding it.  Use
/// [`EventFrame::retained_bytes`] only when an event can retain more than the
/// conservative default, such as an importer attaching a large native value.
pub struct EventFrame {
    id: EventId,
    event: Event,
    occurred_at: Option<SystemTime>,
    retained_bytes: usize,
}

impl EventFrame {
    /// Conservative per-event default that keeps ordinary adapter code free
    /// from manual allocation accounting.  A frame whose actual retained size
    /// exceeds this bound is rejected with a precise error and can be retried
    /// with [`Self::retained_bytes`].
    pub const DEFAULT_RETAINED_BYTES: usize = 64 * 1024;

    /// Creates a canonical frame with receive-time as its occurrence time.
    #[must_use]
    pub fn new(id: EventId, event: Event) -> Self {
        Self {
            id,
            event,
            occurred_at: Some(SystemTime::now()),
            retained_bytes: Self::DEFAULT_RETAINED_BYTES,
        }
    }

    /// Replaces the adapter-reported occurrence time.
    #[must_use]
    pub fn occurred_at(mut self, occurred_at: SystemTime) -> Self {
        self.occurred_at = Some(occurred_at);
        self
    }

    /// Omits an occurrence time when the transport does not provide one.
    #[must_use]
    pub fn without_occurrence_time(mut self) -> Self {
        self.occurred_at = None;
        self
    }

    /// Sets a conservative retained-byte charge for this event.
    ///
    /// The value includes all event-owned payload data but not the runtime's
    /// small dispatch-envelope overhead. It must be non-zero.
    #[must_use]
    pub fn retained_bytes(mut self, retained_bytes: usize) -> Self {
        self.retained_bytes = retained_bytes.max(std::mem::size_of::<Event>());
        self
    }

    fn dispatch_index(&self, bot: BotSlot, platform: &PlatformId) -> DispatchIndex {
        let mut index = DispatchIndex::event(bot, platform.clone(), self.event.event_type());
        match &self.event {
            Event::Message(event) => {
                index.conversation = Some(conversation_key(bot, &event.conversation));
                index.actor = Some(UserKey::new(bot, CompactId::from(event.sender.id.clone())));
                index.command = command_key(&event.message);
            }
            Event::Interaction(event) => {
                index.conversation = event
                    .conversation
                    .as_ref()
                    .map(|conversation| conversation_key(bot, conversation));
                index.actor = Some(UserKey::new(bot, CompactId::from(event.user.id.clone())));
                index.interaction = event.action_id.as_deref().map(Arc::from);
            }
            Event::Request(event) => {
                let (user, conversation) = match event {
                    oxidebot_core::event::RequestEvent::Friend(event) => (&event.user, None),
                    oxidebot_core::event::RequestEvent::GroupJoin(event) => {
                        (&event.user, Some(&event.conversation))
                    }
                    oxidebot_core::event::RequestEvent::GroupInvite(event) => {
                        (&event.user, Some(&event.conversation))
                    }
                };
                index.actor = Some(UserKey::new(bot, CompactId::from(user.id.clone())));
                index.conversation = conversation.map(|value| conversation_key(bot, value));
            }
            Event::Native(event) => index.native_type = Some(Arc::from(event.kind.as_str())),
            Event::Notice(event) => {
                use oxidebot_core::event::NoticeEvent;
                let (conversation, user) = match event {
                    NoticeEvent::GroupMemberJoined(event) => {
                        (Some(&event.conversation), Some(&event.user))
                    }
                    NoticeEvent::GroupMemberLeft(event) => {
                        (Some(&event.conversation), Some(&event.user))
                    }
                    NoticeEvent::GroupAdminChanged(event) => {
                        (Some(&event.conversation), Some(&event.user))
                    }
                    NoticeEvent::GroupMuteChanged(event) => {
                        (Some(&event.conversation), event.operator.as_ref())
                    }
                    NoticeEvent::GroupMemberMuteChanged(event) => {
                        (Some(&event.conversation), Some(&event.user))
                    }
                    NoticeEvent::GroupHighlightChanged(event) => (
                        Some(&event.conversation),
                        event.sender.as_ref().or(event.operator.as_ref()),
                    ),
                    NoticeEvent::GroupMemberAliasChanged(event) => {
                        (Some(&event.conversation), Some(&event.user))
                    }
                    NoticeEvent::MessageReactionsChanged(event) => {
                        (event.conversation.as_ref(), Some(&event.user))
                    }
                    NoticeEvent::MessageDeleted(event) => (
                        event.conversation.as_ref(),
                        event.user.as_ref().or(event.operator.as_ref()),
                    ),
                    NoticeEvent::MessageEdited(event) => {
                        (event.conversation.as_ref(), Some(&event.user))
                    }
                };
                index.conversation = conversation.map(|value| conversation_key(bot, value));
                index.actor =
                    user.map(|value| UserKey::new(bot, CompactId::from(value.id.clone())));
            }
            Event::Lifecycle(_) | Event::Meta(_) => {}
        }
        index
    }
}

fn conversation_key(bot: BotSlot, conversation: &ConversationRef) -> ConversationKey {
    if let Some(parent) = &conversation.parent {
        ConversationKey::new(bot, CompactId::from(parent.id.clone()))
            .in_subspace(CompactId::from(conversation.id.clone()))
    } else {
        ConversationKey::new(bot, CompactId::from(conversation.id.clone()))
    }
}

fn command_key(message: &Message) -> Option<Arc<str>> {
    message
        .get_raw_text()
        .strip_prefix('/')
        .and_then(|value| value.split_whitespace().next())
        .and_then(|value| value.split('@').next())
        .filter(|value| !value.is_empty())
        .map(Arc::from)
}

impl InboundFrame for EventFrame {
    fn index(
        &self,
        bot: BotSlot,
        platform: &PlatformId,
    ) -> std::result::Result<FrameIndex, DecodeError> {
        self.id
            .validate()
            .map_err(|error| DecodeError::new(format!("invalid event id: {error}")))?;
        Ok(FrameIndex::one(
            self.dispatch_index(bot, platform),
            self.retained_bytes.saturating_add(2_048),
        ))
    }

    fn decode(
        self,
        bot: BotSlot,
        platform: &PlatformId,
    ) -> std::result::Result<DispatchBatch, DecodeError> {
        let indexed = FrameIndex::one(self.dispatch_index(bot, platform), 0);
        self.decode_indexed(bot, platform, &indexed)
    }

    fn decode_indexed(
        self,
        _bot: BotSlot,
        _platform: &PlatformId,
        indexed: &FrameIndex,
    ) -> std::result::Result<DispatchBatch, DecodeError> {
        let index = indexed
            .events
            .first()
            .cloned()
            .ok_or_else(|| DecodeError::new("event frame index is empty"))?;
        let mut draft = DispatchDraft::new(self.id, index, self.event, self.retained_bytes);
        draft.occurred_at = self.occurred_at;
        Ok(DispatchBatch::new([draft]))
    }
}

/// A bounded batch of already-decoded canonical events from one transport
/// delivery. This preserves one admission decision while still letting the
/// runtime discard uninterested individual events after validation.
#[derive(Default)]
pub struct EventBatchFrame {
    events: Vec<EventFrame>,
}

impl EventBatchFrame {
    #[must_use]
    pub fn new(events: impl IntoIterator<Item = EventFrame>) -> Self {
        Self {
            events: events.into_iter().collect(),
        }
    }

    #[must_use]
    pub fn push(mut self, event: EventFrame) -> Self {
        self.events.push(event);
        self
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

impl InboundFrame for EventBatchFrame {
    fn index(
        &self,
        bot: BotSlot,
        platform: &PlatformId,
    ) -> std::result::Result<FrameIndex, DecodeError> {
        let mut events = Vec::with_capacity(self.events.len());
        let mut estimated_bytes = 0_usize;
        for frame in &self.events {
            let index = frame.index(bot, platform)?;
            events.extend(index.events);
            estimated_bytes = estimated_bytes.saturating_add(index.estimated_bytes);
        }
        Ok(FrameIndex {
            events,
            estimated_bytes,
        })
    }

    fn decode(
        self,
        bot: BotSlot,
        platform: &PlatformId,
    ) -> std::result::Result<DispatchBatch, DecodeError> {
        let indexed = self.index(bot, platform)?;
        self.decode_indexed(bot, platform, &indexed)
    }

    fn decode_indexed(
        self,
        _bot: BotSlot,
        _platform: &PlatformId,
        indexed: &FrameIndex,
    ) -> std::result::Result<DispatchBatch, DecodeError> {
        if indexed.events.len() != self.events.len() {
            return Err(DecodeError::new(
                "canonical event batch index does not match its event count",
            ));
        }
        let events = self
            .events
            .into_iter()
            .zip(indexed.events.iter().cloned())
            .map(|(frame, index)| {
                let mut draft =
                    DispatchDraft::new(frame.id, index, frame.event, frame.retained_bytes);
                draft.occurred_at = frame.occurred_at;
                draft
            });
        Ok(DispatchBatch::new(events))
    }
}

/// Canonical message frame for simple adapters, importers, console transports,
/// and tests. High-throughput adapters may still implement [`InboundFrame`]
/// directly to reuse offsets from their wire format.
pub struct MessageFrame {
    id: EventId,
    conversation: ConversationRef,
    actor: CompactId,
    sender: User,
    message: Message,
    occurred_at: Option<SystemTime>,
}

impl MessageFrame {
    #[must_use]
    pub fn new(
        id: EventId,
        conversation: ConversationRef,
        actor: impl Into<CompactId>,
        message: Message,
    ) -> Self {
        Self {
            id,
            conversation,
            actor: actor.into(),
            sender: User::default(),
            message,
            occurred_at: Some(SystemTime::now()),
        }
        .with_default_sender()
    }

    #[must_use]
    pub fn text(
        id: EventId,
        conversation: ConversationRef,
        actor: impl Into<CompactId>,
        message_id: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        let mut message = Message::text(text);
        message.id = message_id.into();
        Self::new(id, conversation, actor, message)
    }

    /// Preserves adapter-provided sender profile data while keeping routing
    /// identity consistent with `sender.id`.
    #[must_use]
    pub fn sender(mut self, sender: User) -> Self {
        self.actor = CompactId::from(sender.id.clone());
        self.sender = sender;
        self
    }

    #[must_use]
    pub fn occurred_at(mut self, occurred_at: SystemTime) -> Self {
        self.occurred_at = Some(occurred_at);
        self
    }

    fn with_default_sender(mut self) -> Self {
        self.sender.id = self.actor.to_string();
        self
    }

    fn dispatch_index(&self, bot: BotSlot, platform: &PlatformId) -> DispatchIndex {
        let mut index = DispatchIndex::event(bot, platform.clone(), EventType::Message);
        let conversation = if let Some(parent) = &self.conversation.parent {
            ConversationKey::new(bot, CompactId::from(parent.id.clone()))
                .in_subspace(CompactId::from(self.conversation.id.clone()))
        } else {
            ConversationKey::new(bot, CompactId::from(self.conversation.id.clone()))
        };
        index.conversation = Some(conversation);
        index.actor = Some(UserKey::new(bot, self.actor.clone()));
        index.command = self
            .message
            .get_raw_text()
            .strip_prefix('/')
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.split('@').next())
            .filter(|value| !value.is_empty())
            .map(Arc::from);
        index
    }

    fn retained_bytes(&self) -> usize {
        self.id
            .estimated_bytes()
            .saturating_add(conversation_retained_bytes(&self.conversation))
            .saturating_add(self.actor.estimated_bytes())
            .saturating_add(user_retained_bytes(&self.sender))
            .saturating_add(self.message.estimated_bytes())
            .saturating_add(2_048)
    }
}

fn optional_string_bytes(value: &Option<String>) -> usize {
    value
        .as_ref()
        .map_or(0, |value| value.capacity().saturating_add(24))
}

fn user_retained_bytes(user: &User) -> usize {
    let mut bytes = user.id.capacity().saturating_add(64);
    if let Some(profile) = &user.profile {
        bytes = bytes
            .saturating_add(optional_string_bytes(&profile.display_name))
            .saturating_add(optional_string_bytes(&profile.avatar))
            .saturating_add(optional_string_bytes(&profile.email))
            .saturating_add(optional_string_bytes(&profile.phone))
            .saturating_add(optional_string_bytes(&profile.signature))
            .saturating_add(optional_string_bytes(&profile.level))
            .saturating_add(128);
    }
    bytes
}

fn conversation_retained_bytes(conversation: &ConversationRef) -> usize {
    conversation
        .id
        .capacity()
        .saturating_add(
            conversation
                .parent
                .as_deref()
                .map_or(0, conversation_retained_bytes),
        )
        .saturating_add(conversation.platform_data.as_ref().map_or(0, |data| {
            data.platform.capacity() + data.data.to_string().len()
        }))
        .saturating_add(128)
}

impl InboundFrame for MessageFrame {
    fn index(
        &self,
        bot: BotSlot,
        platform: &PlatformId,
    ) -> std::result::Result<FrameIndex, DecodeError> {
        self.id
            .validate()
            .map_err(|error| DecodeError::new(format!("invalid message event id: {error}")))?;
        CompactId::from(self.conversation.id.clone())
            .validate()
            .map_err(|error| DecodeError::new(format!("invalid conversation id: {error}")))?;
        self.actor
            .validate()
            .map_err(|error| DecodeError::new(format!("invalid actor id: {error}")))?;
        if let Some(parent) = &self.conversation.parent {
            CompactId::from(parent.id.clone())
                .validate()
                .map_err(|error| {
                    DecodeError::new(format!("invalid parent conversation id: {error}"))
                })?;
        }
        if self.message.id.is_empty() {
            return Err(DecodeError::new("message id must not be empty"));
        }
        Ok(FrameIndex::one(
            self.dispatch_index(bot, platform),
            self.retained_bytes().saturating_add(2_048),
        ))
    }

    fn decode(
        self,
        bot: BotSlot,
        platform: &PlatformId,
    ) -> std::result::Result<DispatchBatch, DecodeError> {
        let indexed = FrameIndex::one(self.dispatch_index(bot, platform), 0);
        self.decode_indexed(bot, platform, &indexed)
    }

    fn decode_indexed(
        self,
        _bot: BotSlot,
        _platform: &PlatformId,
        indexed: &FrameIndex,
    ) -> std::result::Result<DispatchBatch, DecodeError> {
        let index = indexed
            .events
            .first()
            .cloned()
            .ok_or_else(|| DecodeError::new("message frame index is empty"))?;
        index
            .conversation
            .as_ref()
            .ok_or_else(|| DecodeError::new("message frame has no conversation"))?;
        let retained = self.retained_bytes();
        let event = Event::Message(MessageEvent {
            id: self.id.as_str().to_owned(),
            time: None,
            sender: self.sender,
            conversation: self.conversation,
            message: self.message,
        });
        let mut draft = DispatchDraft::new(self.id, index, event, retained);
        draft.occurred_at = self.occurred_at;
        Ok(DispatchBatch::new([draft]))
    }
}

/// Builder for adapters that receive a canonical message but want named setup
/// methods rather than implementing the two-stage frame contract manually.
pub struct MessageFrameBuilder {
    frame: MessageFrame,
}

impl MessageFrameBuilder {
    #[must_use]
    pub fn new(
        id: EventId,
        conversation: ConversationRef,
        actor: impl Into<CompactId>,
        message: Message,
    ) -> Self {
        Self {
            frame: MessageFrame::new(id, conversation, actor, message),
        }
    }

    #[must_use]
    pub fn sender(mut self, sender: User) -> Self {
        self.frame = self.frame.sender(sender);
        self
    }

    #[must_use]
    pub fn occurred_at(mut self, occurred_at: SystemTime) -> Self {
        self.frame = self.frame.occurred_at(occurred_at);
        self
    }

    #[must_use]
    pub fn build(self) -> MessageFrame {
        self.frame
    }
}

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

/// Result of submitting one platform frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Submission {
    Ignored,
    Accepted { events: usize },
}

pub(crate) struct IngressBatch {
    pub(crate) batch: DispatchBatch,
    pub(crate) lease: HierarchicalLease,
}

#[derive(Clone)]
pub(crate) struct EventSink {
    sender: mpsc::Sender<IngressBatch>,
    limiter: QueueLimiter,
}

impl EventSink {
    pub(crate) fn channel(budget: QueueBudget) -> (Self, mpsc::Receiver<IngressBatch>) {
        let (sender, receiver) = mpsc::channel(budget.max_items);
        (
            Self {
                sender,
                limiter: QueueLimiter::new(budget),
            },
            receiver,
        )
    }

    async fn reserve(
        &self,
        estimated_bytes: usize,
    ) -> std::result::Result<(mpsc::OwnedPermit<IngressBatch>, QueueLease), AdapterError> {
        let lease = self
            .limiter
            .acquire(estimated_bytes)
            .await
            .map_err(map_ingress_error)?;
        let permit = match self.sender.clone().reserve_owned().await {
            Ok(permit) => permit,
            Err(_) => return Err(AdapterError::new("runtime ingress queue is closed")),
        };
        Ok((permit, lease))
    }
}

fn map_ingress_error(error: QueueAcquireError) -> AdapterError {
    match error {
        QueueAcquireError::Closed => AdapterError::new("runtime ingress budget is closed"),
        QueueAcquireError::Full => AdapterError::new("runtime ingress budget is full"),
        QueueAcquireError::TooLarge => {
            AdapterError::new("platform frame exceeds the ingress byte budget")
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct AdapterLimits {
    pub(crate) ingress: QueueBudget,
    pub(crate) max_frame_bytes: usize,
    pub(crate) max_frame_events: usize,
    pub(crate) max_event_bytes: usize,
}

/// Runtime facilities provided to one adapter runner.
#[derive(Clone)]
pub struct AdapterContext {
    slot: BotSlot,
    platform: PlatformId,
    sink: EventSink,
    ingress_limiter: QueueLimiter,
    interest: InterestPlan,
    cancellation: CancellationToken,
    shutdown: ShutdownSignal,
    metrics: MetricsHandle,
    max_frame_bytes: usize,
    max_frame_events: usize,
    max_event_bytes: usize,
}

impl AdapterContext {
    pub(crate) fn new(
        slot: BotSlot,
        platform: PlatformId,
        sink: EventSink,
        limits: AdapterLimits,
        interest: InterestPlan,
        cancellation: CancellationToken,
        metrics: MetricsHandle,
    ) -> Self {
        let shutdown = ShutdownSignal::new(cancellation.clone());
        Self {
            slot,
            platform,
            sink,
            ingress_limiter: QueueLimiter::new(limits.ingress),
            interest,
            cancellation,
            shutdown,
            metrics,
            max_frame_bytes: limits.max_frame_bytes,
            max_frame_events: limits.max_frame_events,
            max_event_bytes: limits.max_event_bytes,
        }
    }

    #[must_use]
    pub const fn bot_slot(&self) -> BotSlot {
        self.slot
    }

    #[must_use]
    pub fn interest(&self) -> &InterestPlan {
        &self.interest
    }

    #[must_use]
    pub fn shutdown(&self) -> &ShutdownSignal {
        &self.shutdown
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    /// Submits one already-normalized standard event.
    ///
    /// This is the preferred adapter API once a wire payload has been mapped
    /// to [`Event`]. The runtime derives routing metadata and admission
    /// accounting; use [`Self::submit`] only when a transport can perform a
    /// cheaper two-stage index/decode operation on its raw frame.
    pub async fn submit_event(
        &self,
        id: EventId,
        event: Event,
    ) -> std::result::Result<Submission, AdapterError> {
        self.submit(EventFrame::new(id, event)).await
    }

    /// Submits a canonical event with adapter-provided occurrence time and
    /// retained-byte accounting.
    pub async fn submit_event_frame(
        &self,
        frame: EventFrame,
    ) -> std::result::Result<Submission, AdapterError> {
        self.submit(frame).await
    }

    /// Submits several canonical events produced by the same transport
    /// delivery. An empty batch is rejected, matching every other inbound
    /// frame contract.
    pub async fn submit_events(
        &self,
        events: impl IntoIterator<Item = EventFrame>,
    ) -> std::result::Result<Submission, AdapterError> {
        self.submit(EventBatchFrame::new(events)).await
    }

    /// Submits one already-canonical portable message without requiring a
    /// simple adapter to implement the two-stage [`InboundFrame`] contract.
    pub async fn submit_message(
        &self,
        id: EventId,
        conversation: ConversationRef,
        actor: impl Into<CompactId>,
        message: Message,
    ) -> std::result::Result<Submission, AdapterError> {
        self.submit(MessageFrame::new(id, conversation, actor, message))
            .await
    }

    /// Convenience form of [`AdapterContext::submit_message`] for transports
    /// whose input unit is a text line.
    pub async fn submit_text(
        &self,
        id: EventId,
        conversation: ConversationRef,
        actor: impl Into<CompactId>,
        message_id: impl Into<String>,
        text: impl Into<String>,
    ) -> std::result::Result<Submission, AdapterError> {
        self.submit(MessageFrame::text(
            id,
            conversation,
            actor,
            message_id,
            text,
        ))
        .await
    }

    async fn reserve_ingress(
        &self,
        estimated_bytes: usize,
    ) -> std::result::Result<(mpsc::OwnedPermit<IngressBatch>, HierarchicalLease), AdapterError>
    {
        // Local-first ordering prevents one bot from reserving global ingress
        // while it is already above its own item/byte share.
        let local = self
            .ingress_limiter
            .acquire(estimated_bytes)
            .await
            .map_err(map_ingress_error)?;
        let (permit, global) = self.sink.reserve(estimated_bytes).await?;
        Ok((permit, HierarchicalLease::new(local, global)))
    }

    /// Applies interest gating, bounded admission, one full decode, and strict
    /// event/index validation before publishing the batch.
    pub async fn submit<F>(&self, frame: F) -> std::result::Result<Submission, AdapterError>
    where
        F: InboundFrame,
    {
        self.metrics.ingress_frame();
        let index = frame.index(self.slot, &self.platform)?;
        if index.events.is_empty() {
            return Err(AdapterError::new(
                "platform frame produced an empty pre-decode index",
            ));
        }
        if index.events.len() > self.max_frame_events {
            return Err(AdapterError::new(
                "platform frame index exceeds the configured event-count limit",
            ));
        }
        if index.estimated_bytes > self.max_frame_bytes {
            return Err(AdapterError::new(
                "platform frame exceeds the configured per-frame byte limit",
            ));
        }
        if let Err(error) = validate_indexes(&index.events, self.slot, &self.platform) {
            self.metrics.validation_error();
            return Err(error);
        }
        if index
            .events
            .iter()
            .all(|event| !self.interest.accepts(event))
        {
            self.metrics.ignored_frame();
            return Ok(Submission::Ignored);
        }

        let (permit, lease) = tokio::select! {
            _ = self.cancellation.cancelled() => {
                return Err(AdapterError::cancelled("adapter submission was cancelled"));
            }
            admission = self.reserve_ingress(index.estimated_bytes) => {
                match admission {
                    Ok(value) => value,
                    Err(_) if self.cancellation.is_cancelled() => {
                        return Err(AdapterError::cancelled("adapter submission was cancelled"));
                    }
                    Err(error) => return Err(error),
                }
            },
        };
        let mut batch = frame.decode_indexed(self.slot, &self.platform, &index)?;
        if batch.events.len() > self.max_frame_events {
            self.metrics.validation_error();
            return Err(AdapterError::new(
                "decoded frame exceeds the configured event-count limit",
            ));
        }
        if let Err(error) = validate_batch(&batch, &index.events, self.slot, &self.platform) {
            self.metrics.validation_error();
            return Err(error);
        }
        let retained_bytes = batch.retained_bytes();
        if retained_bytes > index.estimated_bytes {
            return Err(AdapterError::new(
                "decoded frame exceeds its pre-decode retained-byte estimate",
            ));
        }
        if retained_bytes > self.max_frame_bytes {
            return Err(AdapterError::new(
                "decoded frame exceeds the configured per-frame byte limit",
            ));
        }
        let decoded_events = batch.events.len();
        self.metrics.decoded_events(decoded_events);

        // A frame may contain several events while only a subset is
        // subscribed. Validate the complete decode, then retain only events that
        // can still reach a route or an exact active session.
        batch
            .events
            .retain(|event| self.interest.accepts(&event.index));
        if batch.events.is_empty() {
            self.metrics.ignored_frame();
            return Ok(Submission::Ignored);
        }
        if batch
            .events
            .iter()
            .any(|event| event.estimated_bytes() > self.max_event_bytes)
        {
            self.metrics.validation_error();
            return Err(AdapterError::new(
                "interested event exceeds the configured per-event byte limit",
            ));
        }
        if self.cancellation.is_cancelled() {
            return Err(AdapterError::cancelled("adapter submission was cancelled"));
        }
        let events = batch.events.len();
        permit.send(IngressBatch { batch, lease });
        Ok(Submission::Accepted { events })
    }
}

fn validate_indexes(
    indexes: &[DispatchIndex],
    slot: BotSlot,
    platform: &PlatformId,
) -> std::result::Result<(), AdapterError> {
    for index in indexes {
        index.validate_for(slot, platform)?;
    }
    Ok(())
}

fn validate_batch(
    batch: &DispatchBatch,
    indexed_events: &[DispatchIndex],
    slot: BotSlot,
    platform: &PlatformId,
) -> std::result::Result<(), AdapterError> {
    if batch.events.is_empty() {
        return Err(AdapterError::new(
            "an interested frame decoded to an empty event batch",
        ));
    }
    if batch.events.len() != indexed_events.len() {
        return Err(AdapterError::new(
            "decoded event count does not match the pre-decode frame index",
        ));
    }
    for (event, indexed) in batch.events.iter().zip(indexed_events) {
        event.index.validate_for(slot, platform)?;
        if &event.index != indexed {
            return Err(AdapterError::new(
                "decoded event routing index differs from the pre-decode frame index",
            ));
        }
        event.validate_for(slot, platform)?;
    }
    Ok(())
}

/// Whether normal completion is meaningful for an adapter.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AdapterMode {
    /// Long-lived network transport. Returning before cancellation is fatal.
    #[default]
    Persistent,
    /// Finite replay/import adapter that may finish successfully.
    Finite,
}

/// One platform transport and its outbound services.
#[async_trait]
pub trait Adapter: Send + 'static {
    fn descriptor(&self) -> BotDescriptor;
    fn services(&self) -> BotServices;
    fn mode(&self) -> AdapterMode {
        AdapterMode::Persistent
    }
    async fn run(self: Box<Self>, context: AdapterContext)
        -> std::result::Result<(), AdapterError>;
}

#[cfg(test)]
mod retained_size_tests {
    use super::*;
    use oxidebot_core::source::user::UserProfile;

    #[test]
    fn message_frame_charges_owned_profile_strings() {
        let id = EventId::new("retained-profile").expect("static event id");
        let baseline = MessageFrame::text(
            id.clone(),
            ConversationRef::direct("room"),
            "user",
            "1",
            "hello",
        );
        let baseline_bytes = baseline.retained_bytes();
        let frame = MessageFrame::text(id, ConversationRef::direct("room"), "user", "1", "hello")
            .sender(User {
                id: "user".into(),
                profile: Some(UserProfile {
                    display_name: Some("x".repeat(1024 * 1024)),
                    ..UserProfile::default()
                }),
            });

        assert!(frame.retained_bytes() >= baseline_bytes.saturating_add(1024 * 1024));
    }

    #[test]
    fn canonical_event_frame_derives_message_routing_metadata() {
        let event = Event::Message(MessageEvent {
            id: "message-1".into(),
            time: None,
            sender: User {
                id: "alice".into(),
                ..User::default()
            },
            conversation: ConversationRef::group("room"),
            message: Message::text("/ping one"),
        });
        let frame = EventFrame::new(EventId::new("event-1").expect("static id"), event);
        let platform = PlatformId::new("test").expect("static platform");
        let index = frame
            .index(BotSlot(0), &platform)
            .expect("canonical event indexes");
        let event = index.events.first().expect("one event index");

        assert_eq!(event.event_type, EventType::Message);
        assert_eq!(event.command.as_deref(), Some("ping"));
        assert_eq!(
            event.conversation.as_ref().map(|key| &key.id),
            Some(&CompactId::from("room"))
        );
        assert_eq!(
            event.actor.as_ref().map(|key| &key.id),
            Some(&CompactId::from("alice"))
        );
    }

    #[test]
    fn canonical_event_frame_derives_interaction_and_native_keys() {
        let platform = PlatformId::new("test").expect("static platform");
        let interaction = Event::Interaction(oxidebot_core::interaction::InteractionEvent {
            id: "interaction-1".into(),
            kind: oxidebot_core::interaction::InteractionKind::Button,
            action_id: Some("approve".into()),
            values: Vec::new(),
            user: User {
                id: "alice".into(),
                ..User::default()
            },
            conversation: None,
            message: None,
            context_id: None,
            response: None,
            fields: Default::default(),
            command: None,
            locale: None,
            permissions: Default::default(),
            data: serde_json::Value::Null,
        });
        let interaction = EventFrame::new(
            EventId::new("event-interaction").expect("static id"),
            interaction,
        )
        .index(BotSlot(0), &platform)
        .expect("interaction indexes");
        assert_eq!(
            interaction.events[0].interaction.as_deref(),
            Some("approve")
        );
    }
}
