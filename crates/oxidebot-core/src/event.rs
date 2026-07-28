//! OxideBot's complete event model.
//!
//! This module is the single event surface for both adapters and handlers. The
//! public values are the canonical event hierarchy. Compact
//! indexes and bounded dispatch records live in the hidden [`kernel`] module;
//! they are implementation details rather than a second event taxonomy.

use std::any::Any;

pub mod any;
#[doc(hidden)]
pub mod kernel;
pub mod lifecycle;
pub mod message;
pub mod meta;
pub mod notice;
pub mod request;

pub use crate::interaction::InteractionEvent;
pub use any::{AnyEvent, AnyEventDataObject, AnyEventDataTrait};
pub use lifecycle::{EventEnvelope, LifecycleEvent};
pub use message::MessageEvent;
pub use meta::MetaEvent;
pub use notice::*;
pub use request::*;

/// The complete OxideBot event hierarchy.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum Event {
    MessageEvent(MessageEvent),
    NoticeEvent(NoticeEvent),
    RequestEvent(RequestEvent),
    InteractionEvent(InteractionEvent),
    LifecycleEvent(LifecycleEvent),
    MetaEvent(MetaEvent),
    AnyEvent(AnyEvent),
}

impl Event {
    /// Returns the stable dense type used by adapter interest gating and the
    /// compiled handler table.
    #[must_use]
    pub const fn event_type(&self) -> EventType {
        match self {
            Self::MessageEvent(_) => EventType::Message,
            Self::NoticeEvent(event) => match event {
                NoticeEvent::GroupMemberIncreseEvent(_) => EventType::NoticeGroupMemberIncrease,
                NoticeEvent::GroupMemberDecreaseEvent(_) => EventType::NoticeGroupMemberDecrease,
                NoticeEvent::GroupAdminChangeEvent(_) => EventType::NoticeGroupAdminChange,
                NoticeEvent::GroupMuteChangeEvent(_) => EventType::NoticeGroupMuteChange,
                NoticeEvent::GroupMemberMuteChangeEvent(_) => {
                    EventType::NoticeGroupMemberMuteChange
                }
                NoticeEvent::GroupHightLightChangeEvent(_) => EventType::NoticeGroupHighlightChange,
                NoticeEvent::GroupMemberAliasChangeEvent(_) => {
                    EventType::NoticeGroupMemberAliasChange
                }
                NoticeEvent::MessageReactionsEvent(_) => EventType::NoticeMessageReactions,
                NoticeEvent::MessageDeletedEvent(_) => EventType::NoticeMessageDeleted,
                NoticeEvent::MessageEditedEvent(_) => EventType::NoticeMessageEdited,
            },
            Self::RequestEvent(event) => match event {
                RequestEvent::FriendAddEvent(_) => EventType::RequestFriendAdd,
                RequestEvent::GroupAddEvent(_) => EventType::RequestGroupAdd,
                RequestEvent::GroupInviteEvent(_) => EventType::RequestGroupInvite,
            },
            Self::InteractionEvent(_) => EventType::Interaction,
            Self::LifecycleEvent(event) => match event {
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
                LifecycleEvent::PlatformNative(_) => EventType::LifecyclePlatformNative,
            },
            Self::MetaEvent(MetaEvent::ConnectEvent) => EventType::MetaConnect,
            Self::MetaEvent(MetaEvent::DisconnectEvent) => EventType::MetaDisconnect,
            Self::AnyEvent(_) => EventType::Any,
        }
    }
}

/// Adapter event source.
pub trait EventTrait: Send + Sync + Any {
    fn get_events(&self) -> Vec<Event>;

    fn get_event_envelopes(&self) -> Vec<EventEnvelope> {
        self.get_events()
            .into_iter()
            .map(|event| EventEnvelope::new(self.server(), event))
            .collect()
    }

    fn server(&self) -> &'static str;
    fn clone_box(&self) -> EventObject;
    fn as_any(&self) -> &dyn Any;
}

pub type EventObject = Box<dyn EventTrait>;

impl Clone for EventObject {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

impl std::fmt::Debug for EventObject {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_list().entries(self.get_events()).finish()
    }
}

/// Dense runtime discriminator derived from the public [`Event`] hierarchy.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum EventType {
    Message = 0,
    NoticeGroupMemberIncrease = 1,
    NoticeGroupMemberDecrease = 2,
    NoticeGroupAdminChange = 3,
    NoticeGroupMuteChange = 4,
    NoticeGroupMemberMuteChange = 5,
    NoticeGroupHighlightChange = 6,
    NoticeGroupMemberAliasChange = 7,
    NoticeMessageReactions = 8,
    NoticeMessageDeleted = 9,
    NoticeMessageEdited = 10,
    RequestFriendAdd = 11,
    RequestGroupAdd = 12,
    RequestGroupInvite = 13,
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
    LifecyclePlatformNative = 49,
    MetaConnect = 50,
    MetaDisconnect = 51,
    Any = 52,
}

impl EventType {
    pub const COUNT: usize = 53;

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
            Self::LifecycleMessageUpdated | Self::NoticeMessageEdited => {
                DispatchKind::MessageUpdate
            }
            Self::LifecycleMessagesDeleted | Self::NoticeMessageDeleted => {
                DispatchKind::MessageDelete
            }
            Self::Interaction
            | Self::RequestFriendAdd
            | Self::RequestGroupAdd
            | Self::RequestGroupInvite
            | Self::LifecycleSuggestionRequested
            | Self::LifecycleSuggestionSelected
            | Self::LifecycleMiniApp
            | Self::LifecycleShippingRequested
            | Self::LifecycleCheckoutRequested => DispatchKind::Interaction,
            Self::LifecycleReactionsChanged | Self::NoticeMessageReactions => {
                DispatchKind::Reaction
            }
            Self::NoticeGroupMemberIncrease
            | Self::NoticeGroupMemberDecrease
            | Self::NoticeGroupAdminChange
            | Self::NoticeGroupMuteChange
            | Self::NoticeGroupMemberMuteChange
            | Self::NoticeGroupMemberAliasChange
            | Self::LifecycleMemberUpdated
            | Self::LifecycleJoinRequested
            | Self::LifecyclePermissionsChanged => DispatchKind::Member,
            Self::NoticeGroupHighlightChange
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
            Self::LifecyclePlatformNative
            | Self::MetaConnect
            | Self::MetaDisconnect
            | Self::Any => DispatchKind::Native,
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

    category_tags!(MessageEvent, MessageEvent, Message => Message);
    category_tags!(InteractionEvent, InteractionEvent, Interaction => Interaction);
    category_tags!(AnyEvent, AnyEvent, Any => Any);
    category_tags!(MetaEvent, MetaEvent,
        Connect => MetaConnect,
        Disconnect => MetaDisconnect,
    );
    macro_rules! notice_tags {
        ($( $name:ident => $variant:ident : $target:ty => $kind:ident ),+ $(,)?) => {$ (
            #[derive(Clone, Copy, Debug, Default)]
            pub struct $name;
            impl EventTag for $name {
                type Event = $target;
                const TYPE: EventType = EventType::$kind;
                fn get(event: &Event) -> Option<&Self::Event> {
                    match event {
                        Event::NoticeEvent(NoticeEvent::$variant(value)) => Some(value),
                        _ => None,
                    }
                }
            }
        )+};
    }

    notice_tags!(
        GroupMemberIncrease => GroupMemberIncreseEvent : GroupMemberIncreseEvent => NoticeGroupMemberIncrease,
        GroupMemberDecrease => GroupMemberDecreaseEvent : GroupMemberDecreaseEvent => NoticeGroupMemberDecrease,
        GroupAdminChange => GroupAdminChangeEvent : GroupAdminChangeEvent => NoticeGroupAdminChange,
        GroupMuteChange => GroupMuteChangeEvent : GroupMuteChangeEvent => NoticeGroupMuteChange,
        GroupMemberMuteChange => GroupMemberMuteChangeEvent : GroupMemberMuteChangeEvent => NoticeGroupMemberMuteChange,
        GroupHighlightChange => GroupHightLightChangeEvent : GroupHightLightChangeEvent => NoticeGroupHighlightChange,
        GroupMemberAliasChange => GroupMemberAliasChangeEvent : GroupMemberAliasChangeEvent => NoticeGroupMemberAliasChange,
        MessageReactions => MessageReactionsEvent : MessageReactionsEvent => NoticeMessageReactions,
        MessageDeleted => MessageDeletedEvent : MessageDeletedEvent => NoticeMessageDeleted,
        MessageEdited => MessageEditedEvent : MessageEditedEvent => NoticeMessageEdited,
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
                        Event::RequestEvent(RequestEvent::$variant(value)) => Some(value),
                        _ => None,
                    }
                }
            }
        )+};
    }

    request_tags!(
        FriendAdd => FriendAddEvent : FriendAddEvent => RequestFriendAdd,
        GroupAdd => GroupAddEvent : GroupAddEvent => RequestGroupAdd,
        GroupInvite => GroupInviteEvent : GroupInviteEvent => RequestGroupInvite,
    );
    category_tags!(LifecycleEvent, LifecycleEvent,
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
        PlatformNative => LifecyclePlatformNative,
    );
}
