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
    source::{
        message::{File, Message},
        user::User,
    },
};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub enum LifecycleEvent {
    GroupMemberJoined(Box<GroupMemberJoinedEvent>),
    GroupMemberLeft(Box<GroupMemberLeftEvent>),
    GroupAdminChanged(Box<GroupAdminChangedEvent>),
    GroupMuteChanged(Box<GroupMuteChangedEvent>),
    GroupMemberMuteChanged(Box<GroupMemberMuteChangedEvent>),
    GroupHighlightChanged(Box<GroupHighlightChangedEvent>),
    GroupMemberAliasChanged(Box<GroupMemberAliasChangedEvent>),
    MessageReactionsChanged(Box<MessageReactionsChangedEvent>),
    MessageDeleted(Box<MessageDeletedEvent>),
    MessageEdited(Box<MessageEditedEvent>),
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct GroupMemberJoinedEvent {
    pub conversation: ConversationRef,
    pub user: User,
    pub reason: GroupMemberJoinReason,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GroupMemberLeftEvent {
    pub conversation: ConversationRef,
    pub user: User,
    pub reason: GroupMemberLeaveReason,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GroupAdminChangedEvent {
    pub conversation: ConversationRef,
    pub user: User,
    pub state: GroupAdminState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GroupMuteChangedEvent {
    pub conversation: ConversationRef,
    pub operator: Option<User>,
    pub state: MuteState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GroupMemberMuteChangedEvent {
    pub conversation: ConversationRef,
    pub user: User,
    pub operator: Option<User>,
    pub state: MuteState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GroupHighlightChangedEvent {
    pub conversation: ConversationRef,
    pub state: GroupHighlightState,
    pub message: Message,
    pub sender: Option<User>,
    pub operator: Option<User>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GroupMemberAliasChangedEvent {
    pub conversation: ConversationRef,
    pub user: User,
    pub operator: Option<User>,
    pub old_alias: Option<String>,
    pub new_alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessageDeletedEvent {
    pub user: Option<User>,
    pub operator: Option<User>,
    pub conversation: Option<ConversationRef>,
    pub message: Option<Message>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GroupMemberJoinReason {
    Approved {
        operator: Option<Box<User>>,
    },
    Invited {
        inviter: Option<Box<User>>,
        operator: Option<Box<User>>,
    },
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GroupMemberLeaveReason {
    Kicked { operator: Option<User> },
    BotKicked { operator: Option<User> },
    Left,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GroupAdminState {
    Granted,
    Revoked,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GroupHighlightState {
    Enabled,
    Disabled,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MuteState {
    Muted { duration: Option<Duration> },
    Unmuted,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessageEditedEvent {
    pub user: User,
    pub conversation: Option<ConversationRef>,
    pub new_message: Option<Message>,
    pub operator: Option<User>,
    pub old_message: Option<Message>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessageReactionsChangedEvent {
    pub user: User,
    pub conversation: Option<ConversationRef>,
    pub message: Message,
    pub reactions: Vec<String>,
}
