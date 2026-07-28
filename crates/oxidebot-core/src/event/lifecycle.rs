use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::{
    application::{MiniAppEvent, SuggestionRequest, SuggestionSelection},
    collaboration::{ActivityState, CallSession, PinnedMessage, ReactionChange, ReadReceipt},
    commerce::{CheckoutRequest, Payment, ShippingRequest, Subscription},
    content::{Checklist, ChecklistChange, MessageEnvelope, Poll},
    conversation::{
        ConversationMember, ConversationProfile, ConversationRef, JoinRequest, MessageRef,
        PermissionSet, Thread,
    },
    interaction::PlatformNativeData,
    source::{message::File, user::User},
};

#[derive(Clone, Debug, PartialEq)]
pub enum LifecycleEvent {
    MessageCreated(Box<MessageEnvelope>),
    MessageUpdated(Box<MessageEnvelope>),
    MessagesDeleted {
        conversation: Option<ConversationRef>,
        messages: Vec<MessageRef>,
        platform_data: Option<PlatformNativeData>,
    },
    ActivityChanged(ActivityState),
    ReadReceiptUpdated(ReadReceipt),
    CallUpdated(CallSession),
    ReactionsChanged {
        message: MessageRef,
        actor: Option<User>,
        change: ReactionChange,
    },
    MessagePinned(PinnedMessage),
    MessageUnpinned(PinnedMessage),
    ThreadCreated(Thread),
    ThreadUpdated(Thread),
    ThreadClosed(Thread),
    ThreadDeleted(Thread),
    PollUpdated(Poll),
    PollVoteChanged {
        poll_id: String,
        user: Option<User>,
        option_ids: Vec<String>,
        platform_data: Option<PlatformNativeData>,
    },
    ChecklistUpdated(Checklist),
    ChecklistChanged(ChecklistChange),
    ConversationCreated {
        conversation: ConversationRef,
        profile: Option<Box<ConversationProfile>>,
    },
    ConversationUpdated {
        conversation: ConversationRef,
        old_profile: Option<Box<ConversationProfile>>,
        new_profile: Box<ConversationProfile>,
    },
    ConversationArchived(ConversationRef),
    MemberUpdated {
        conversation: ConversationRef,
        old_member: Option<Box<ConversationMember>>,
        new_member: Box<ConversationMember>,
    },
    JoinRequested(JoinRequest),
    PermissionsChanged {
        conversation: ConversationRef,
        user: Option<User>,
        old_permissions: Option<PermissionSet>,
        new_permissions: PermissionSet,
    },
    FileShared {
        conversation: ConversationRef,
        user: Option<User>,
        file: File,
    },
    FileDeleted {
        conversation: Option<ConversationRef>,
        file_id: String,
    },
    SuggestionRequested(SuggestionRequest),
    SuggestionSelected(SuggestionSelection),
    MiniApp(MiniAppEvent),
    ShippingRequested(ShippingRequest),
    CheckoutRequested(CheckoutRequest),
    PaymentUpdated(Payment),
    SubscriptionUpdated(Subscription),
    ScheduledMessageSent(MessageRef),
    ScheduledMessageFailed {
        client_message_id: Option<String>,
        error: String,
    },
    PlatformNative(PlatformNativeData),
}

#[derive(Clone, Debug)]
pub struct EventEnvelope {
    pub id: String,
    pub platform: String,
    pub bot_id: Option<String>,
    pub occurred_at: Option<DateTime<Utc>>,
    pub received_at: DateTime<Utc>,
    pub conversation: Option<ConversationRef>,
    pub delivery_attempt: Option<u32>,
    pub raw: Option<Value>,
    pub metadata: BTreeMap<String, Value>,
    pub event: super::Event,
}

impl EventEnvelope {
    pub fn new(platform: impl Into<String>, event: super::Event) -> Self {
        Self {
            id: String::new(),
            platform: platform.into(),
            bot_id: None,
            occurred_at: None,
            received_at: Utc::now(),
            conversation: None,
            delivery_attempt: None,
            raw: None,
            metadata: BTreeMap::new(),
            event,
        }
    }
}
