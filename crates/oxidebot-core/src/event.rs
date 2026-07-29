//! OxideBot's complete event model.
//!
//! This module is the single event surface for both adapters and handlers. The
//! public values are the canonical event hierarchy. Compact
//! indexes and bounded dispatch records live in the hidden [`kernel`] module;
//! they are implementation details rather than a second event taxonomy.

#[doc(hidden)]
pub mod kernel;
pub mod lifecycle;
pub mod message;
pub mod meta;
pub mod native;
pub mod request;

pub use crate::interaction::InteractionEvent;
pub use lifecycle::*;
pub use message::MessageEvent;
pub use meta::MetaEvent;
pub use native::{NativeEvent, NativeEventData, NativeEventPayload};
pub use request::*;

/// The complete OxideBot event hierarchy.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum Event {
    Message(MessageEvent),
    Request(RequestEvent),
    Interaction(InteractionEvent),
    Lifecycle(LifecycleEvent),
    Meta(MetaEvent),
    Native(NativeEvent),
}

impl Event {
    /// Returns the stable dense type used by adapter interest gating and the
    /// compiled handler table.
    #[must_use]
    pub const fn event_type(&self) -> EventType {
        match self {
            Self::Message(_) => EventType::Message,
            Self::Request(event) => match event {
                RequestEvent::Friend(_) => EventType::FriendRequested,
                RequestEvent::GroupJoin(_) => EventType::GroupJoinRequested,
                RequestEvent::GroupInvite(_) => EventType::GroupInvited,
            },
            Self::Interaction(_) => EventType::Interaction,
            Self::Lifecycle(event) => match event {
                LifecycleEvent::GroupMemberJoined(_) => EventType::GroupMemberJoined,
                LifecycleEvent::GroupMemberLeft(_) => EventType::GroupMemberLeft,
                LifecycleEvent::GroupAdminChanged(_) => EventType::GroupAdminChanged,
                LifecycleEvent::GroupMuteChanged(_) => EventType::GroupMuteChanged,
                LifecycleEvent::GroupMemberMuteChanged(_) => EventType::GroupMemberMuteChanged,
                LifecycleEvent::GroupHighlightChanged(_) => EventType::GroupHighlightChanged,
                LifecycleEvent::GroupMemberAliasChanged(_) => EventType::GroupMemberAliasChanged,
                LifecycleEvent::MessageReactionsChanged(_) => EventType::MessageReactionsChanged,
                LifecycleEvent::MessageDeleted(_) => EventType::MessageDeleted,
                LifecycleEvent::MessageEdited(_) => EventType::MessageEdited,
                LifecycleEvent::MessageCreated(_) => EventType::LifecycleMessageCreated,
                LifecycleEvent::MessageUpdated(_) => EventType::LifecycleMessageUpdated,
                LifecycleEvent::MessagesDeleted { .. } => EventType::LifecycleMessagesDeleted,
                LifecycleEvent::ActivityChanged(_) => EventType::LifecycleActivityChanged,
                LifecycleEvent::ReadReceiptUpdated(_) => EventType::LifecycleReadReceiptUpdated,
                LifecycleEvent::CallUpdated(_) => EventType::LifecycleCallUpdated,
                LifecycleEvent::ReactionsChanged { .. } => EventType::LifecycleReactionsChanged,
                LifecycleEvent::MessagePinned(_) => EventType::LifecycleMessagePinned,
                LifecycleEvent::MessageUnpinned(_) => EventType::LifecycleMessageUnpinned,
                LifecycleEvent::ThreadCreated(_) => EventType::LifecycleThreadCreated,
                LifecycleEvent::ThreadUpdated(_) => EventType::LifecycleThreadUpdated,
                LifecycleEvent::ThreadClosed(_) => EventType::LifecycleThreadClosed,
                LifecycleEvent::ThreadDeleted(_) => EventType::LifecycleThreadDeleted,
                LifecycleEvent::PollUpdated(_) => EventType::LifecyclePollUpdated,
                LifecycleEvent::PollVoteChanged { .. } => EventType::LifecyclePollVoteChanged,
                LifecycleEvent::ChecklistUpdated(_) => EventType::LifecycleChecklistUpdated,
                LifecycleEvent::ChecklistChanged(_) => EventType::LifecycleChecklistChanged,
                LifecycleEvent::ConversationCreated { .. } => {
                    EventType::LifecycleConversationCreated
                }
                LifecycleEvent::ConversationUpdated { .. } => {
                    EventType::LifecycleConversationUpdated
                }
                LifecycleEvent::ConversationArchived(_) => EventType::LifecycleConversationArchived,
                LifecycleEvent::MemberUpdated { .. } => EventType::LifecycleMemberUpdated,
                LifecycleEvent::JoinRequested(_) => EventType::LifecycleJoinRequested,
                LifecycleEvent::PermissionsChanged { .. } => EventType::LifecyclePermissionsChanged,
                LifecycleEvent::FileShared { .. } => EventType::LifecycleFileShared,
                LifecycleEvent::FileDeleted { .. } => EventType::LifecycleFileDeleted,
                LifecycleEvent::SuggestionRequested(_) => EventType::LifecycleSuggestionRequested,
                LifecycleEvent::SuggestionSelected(_) => EventType::LifecycleSuggestionSelected,
                LifecycleEvent::MiniApp(_) => EventType::LifecycleMiniApp,
                LifecycleEvent::ShippingRequested(_) => EventType::LifecycleShippingRequested,
                LifecycleEvent::CheckoutRequested(_) => EventType::LifecycleCheckoutRequested,
                LifecycleEvent::PaymentUpdated(_) => EventType::LifecyclePaymentUpdated,
                LifecycleEvent::SubscriptionUpdated(_) => EventType::LifecycleSubscriptionUpdated,
                LifecycleEvent::ScheduledMessageSent(_) => EventType::LifecycleScheduledMessageSent,
                LifecycleEvent::ScheduledMessageFailed { .. } => {
                    EventType::LifecycleScheduledMessageFailed
                }
            },
            Self::Meta(MetaEvent::Connected) => EventType::MetaConnected,
            Self::Meta(MetaEvent::Disconnected) => EventType::MetaDisconnected,
            Self::Native(_) => EventType::Native,
        }
    }
}

/// Dense runtime discriminator derived from the public [`Event`] hierarchy.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum EventType {
    Message = 0,
    GroupMemberJoined = 1,
    GroupMemberLeft = 2,
    GroupAdminChanged = 3,
    GroupMuteChanged = 4,
    GroupMemberMuteChanged = 5,
    GroupHighlightChanged = 6,
    GroupMemberAliasChanged = 7,
    MessageReactionsChanged = 8,
    MessageDeleted = 9,
    MessageEdited = 10,
    FriendRequested = 11,
    GroupJoinRequested = 12,
    GroupInvited = 13,
    Interaction = 14,
    LifecycleMessageCreated = 15,
    LifecycleMessageUpdated = 16,
    LifecycleMessagesDeleted = 17,
    LifecycleActivityChanged = 18,
    LifecycleReadReceiptUpdated = 19,
    LifecycleCallUpdated = 20,
    LifecycleReactionsChanged = 21,
    LifecycleMessagePinned = 22,
    LifecycleMessageUnpinned = 23,
    LifecycleThreadCreated = 24,
    LifecycleThreadUpdated = 25,
    LifecycleThreadClosed = 26,
    LifecycleThreadDeleted = 27,
    LifecyclePollUpdated = 28,
    LifecyclePollVoteChanged = 29,
    LifecycleChecklistUpdated = 30,
    LifecycleChecklistChanged = 31,
    LifecycleConversationCreated = 32,
    LifecycleConversationUpdated = 33,
    LifecycleConversationArchived = 34,
    LifecycleMemberUpdated = 35,
    LifecycleJoinRequested = 36,
    LifecyclePermissionsChanged = 37,
    LifecycleFileShared = 38,
    LifecycleFileDeleted = 39,
    LifecycleSuggestionRequested = 40,
    LifecycleSuggestionSelected = 41,
    LifecycleMiniApp = 42,
    LifecycleShippingRequested = 43,
    LifecycleCheckoutRequested = 44,
    LifecyclePaymentUpdated = 45,
    LifecycleSubscriptionUpdated = 46,
    LifecycleScheduledMessageSent = 47,
    LifecycleScheduledMessageFailed = 48,
    MetaConnected = 49,
    MetaDisconnected = 50,
    Native = 51,
}

impl EventType {
    pub const COUNT: usize = 52;

    #[must_use]
    pub const fn bit(self) -> u64 {
        1_u64 << (self as u8)
    }

    #[doc(hidden)]
    #[must_use]
    pub const fn dispatch_kind(self) -> kernel::DispatchKind {
        use kernel::DispatchKind;
        match self {
            Self::Message | Self::LifecycleMessageCreated => DispatchKind::Message,
            Self::LifecycleMessageUpdated | Self::MessageEdited => DispatchKind::MessageUpdate,
            Self::LifecycleMessagesDeleted | Self::MessageDeleted => DispatchKind::MessageDelete,
            Self::Interaction
            | Self::FriendRequested
            | Self::GroupJoinRequested
            | Self::GroupInvited
            | Self::LifecycleSuggestionRequested
            | Self::LifecycleSuggestionSelected
            | Self::LifecycleMiniApp
            | Self::LifecycleShippingRequested
            | Self::LifecycleCheckoutRequested => DispatchKind::Interaction,
            Self::LifecycleReactionsChanged | Self::MessageReactionsChanged => {
                DispatchKind::Reaction
            }
            Self::GroupMemberJoined
            | Self::GroupMemberLeft
            | Self::GroupAdminChanged
            | Self::GroupMuteChanged
            | Self::GroupMemberMuteChanged
            | Self::GroupMemberAliasChanged
            | Self::LifecycleMemberUpdated
            | Self::LifecycleJoinRequested
            | Self::LifecyclePermissionsChanged => DispatchKind::Member,
            Self::GroupHighlightChanged
            | Self::LifecycleActivityChanged
            | Self::LifecycleReadReceiptUpdated
            | Self::LifecycleCallUpdated
            | Self::LifecycleMessagePinned
            | Self::LifecycleMessageUnpinned
            | Self::LifecycleThreadCreated
            | Self::LifecycleThreadUpdated
            | Self::LifecycleThreadClosed
            | Self::LifecycleThreadDeleted
            | Self::LifecyclePollUpdated
            | Self::LifecyclePollVoteChanged
            | Self::LifecycleChecklistUpdated
            | Self::LifecycleChecklistChanged
            | Self::LifecycleConversationCreated
            | Self::LifecycleConversationUpdated
            | Self::LifecycleConversationArchived
            | Self::LifecycleScheduledMessageSent
            | Self::LifecycleScheduledMessageFailed => DispatchKind::Conversation,
            Self::LifecycleFileShared | Self::LifecycleFileDeleted => DispatchKind::File,
            Self::LifecyclePaymentUpdated | Self::LifecycleSubscriptionUpdated => {
                DispatchKind::Payment
            }
            Self::MetaConnected | Self::MetaDisconnected | Self::Native => DispatchKind::Native,
        }
    }
}

/// Compact runtime interest set derived from [`Event`].
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EventTypeSet(u64);

impl EventTypeSet {
    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    #[must_use]
    pub const fn one(event_type: EventType) -> Self {
        Self(event_type.bit())
    }

    pub fn insert(&mut self, event_type: EventType) {
        self.0 |= event_type.bit();
    }

    #[must_use]
    pub const fn contains(self, event_type: EventType) -> bool {
        self.0 & event_type.bit() != 0
    }

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// Compile-time selector implemented by the marker types in [`tags`].
#[doc(hidden)]
pub trait EventTag: Send + Sync + 'static {
    type Event: Send + Sync + 'static;
    const TYPE: EventType;
    fn get(event: &Event) -> Option<&Self::Event>;
}

/// Marker types for compile-time event routing.
pub mod tags {
    use super::*;

    macro_rules! category_tags {
        ($variant:ident, $target:ty, $( $name:ident => $kind:ident ),+ $(,)?) => {$ (
            #[derive(Clone, Copy, Debug, Default)]
            pub struct $name;
            impl EventTag for $name {
                type Event = $target;
                const TYPE: EventType = EventType::$kind;
                fn get(event: &Event) -> Option<&Self::Event> {
                    if event.event_type() != Self::TYPE {
                        return None;
                    }
                    match event {
                        Event::$variant(value) => Some(value),
                        _ => None,
                    }
                }
            }
        )+};
    }

    category_tags!(Message, MessageEvent, Message => Message);
    category_tags!(Interaction, InteractionEvent, Interaction => Interaction);
    category_tags!(Native, NativeEvent, Native => Native);
    category_tags!(Meta, MetaEvent,
        Connected => MetaConnected,
        Disconnected => MetaDisconnected,
    );
    macro_rules! lifecycle_payload_tags {
        ($( $name:ident => $variant:ident : $target:ty => $kind:ident ),+ $(,)?) => {$ (
            #[derive(Clone, Copy, Debug, Default)]
            pub struct $name;
            impl EventTag for $name {
                type Event = $target;
                const TYPE: EventType = EventType::$kind;
                fn get(event: &Event) -> Option<&Self::Event> {
                    match event {
                        Event::Lifecycle(LifecycleEvent::$variant(value)) => Some(value),
                        _ => None,
                    }
                }
            }
        )+};
    }

    lifecycle_payload_tags!(
        GroupMemberJoined => GroupMemberJoined : GroupMemberJoinedEvent => GroupMemberJoined,
        GroupMemberLeft => GroupMemberLeft : GroupMemberLeftEvent => GroupMemberLeft,
        GroupAdminChanged => GroupAdminChanged : GroupAdminChangedEvent => GroupAdminChanged,
        GroupMuteChanged => GroupMuteChanged : GroupMuteChangedEvent => GroupMuteChanged,
        GroupMemberMuteChanged => GroupMemberMuteChanged : GroupMemberMuteChangedEvent => GroupMemberMuteChanged,
        GroupHighlightChanged => GroupHighlightChanged : GroupHighlightChangedEvent => GroupHighlightChanged,
        GroupMemberAliasChanged => GroupMemberAliasChanged : GroupMemberAliasChangedEvent => GroupMemberAliasChanged,
        MessageReactionsChanged => MessageReactionsChanged : MessageReactionsChangedEvent => MessageReactionsChanged,
        MessageDeleted => MessageDeleted : MessageDeletedEvent => MessageDeleted,
        MessageEdited => MessageEdited : MessageEditedEvent => MessageEdited,
    );

    macro_rules! request_tags {
        ($( $name:ident => $variant:ident : $target:ty => $kind:ident ),+ $(,)?) => {$ (
            #[derive(Clone, Copy, Debug, Default)]
            pub struct $name;
            impl EventTag for $name {
                type Event = $target;
                const TYPE: EventType = EventType::$kind;
                fn get(event: &Event) -> Option<&Self::Event> {
                    match event {
                        Event::Request(RequestEvent::$variant(value)) => Some(value),
                        _ => None,
                    }
                }
            }
        )+};
    }

    request_tags!(
        FriendRequested => Friend : FriendRequest => FriendRequested,
        GroupJoinRequested => GroupJoin : GroupJoinRequest => GroupJoinRequested,
        GroupInvited => GroupInvite : GroupInviteRequest => GroupInvited,
    );
    category_tags!(Lifecycle, LifecycleEvent,
        MessageCreated => LifecycleMessageCreated,
        MessageUpdated => LifecycleMessageUpdated,
        MessagesDeleted => LifecycleMessagesDeleted,
        ActivityChanged => LifecycleActivityChanged,
        ReadReceiptUpdated => LifecycleReadReceiptUpdated,
        CallUpdated => LifecycleCallUpdated,
        ReactionsChanged => LifecycleReactionsChanged,
        MessagePinned => LifecycleMessagePinned,
        MessageUnpinned => LifecycleMessageUnpinned,
        ThreadCreated => LifecycleThreadCreated,
        ThreadUpdated => LifecycleThreadUpdated,
        ThreadClosed => LifecycleThreadClosed,
        ThreadDeleted => LifecycleThreadDeleted,
        PollUpdated => LifecyclePollUpdated,
        PollVoteChanged => LifecyclePollVoteChanged,
        ChecklistUpdated => LifecycleChecklistUpdated,
        ChecklistChanged => LifecycleChecklistChanged,
        ConversationCreated => LifecycleConversationCreated,
        ConversationUpdated => LifecycleConversationUpdated,
        ConversationArchived => LifecycleConversationArchived,
        MemberUpdated => LifecycleMemberUpdated,
        JoinRequested => LifecycleJoinRequested,
        PermissionsChanged => LifecyclePermissionsChanged,
        FileShared => LifecycleFileShared,
        FileDeleted => LifecycleFileDeleted,
        SuggestionRequested => LifecycleSuggestionRequested,
        SuggestionSelected => LifecycleSuggestionSelected,
        MiniApp => LifecycleMiniApp,
        ShippingRequested => LifecycleShippingRequested,
        CheckoutRequested => LifecycleCheckoutRequested,
        PaymentUpdated => LifecyclePaymentUpdated,
        SubscriptionUpdated => LifecycleSubscriptionUpdated,
        ScheduledMessageSent => LifecycleScheduledMessageSent,
        ScheduledMessageFailed => LifecycleScheduledMessageFailed,
    );
}
