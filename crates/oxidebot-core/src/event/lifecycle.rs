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

/// Normalized platform state-change event.
#[derive(Clone, Debug, PartialEq)]
pub enum LifecycleEvent {
    /// A member joined a group.
    GroupMemberJoined(Box<GroupMemberJoinedEvent>),
    /// A member left a group.
    GroupMemberLeft(Box<GroupMemberLeftEvent>),
    /// A member's administrator state changed.
    GroupAdminChanged(Box<GroupAdminChangedEvent>),
    /// Group-wide mute state changed.
    GroupMuteChanged(Box<GroupMuteChangedEvent>),
    /// One member's mute state changed.
    GroupMemberMuteChanged(Box<GroupMemberMuteChangedEvent>),
    /// Group highlight state changed.
    GroupHighlightChanged(Box<GroupHighlightChangedEvent>),
    /// A member's group alias changed.
    GroupMemberAliasChanged(Box<GroupMemberAliasChangedEvent>),
    /// Reactions represented by a legacy platform event changed.
    MessageReactionsChanged(Box<MessageReactionsChangedEvent>),
    /// A message was deleted.
    MessageDeleted(Box<MessageDeletedEvent>),
    /// A message was edited.
    MessageEdited(Box<MessageEditedEvent>),
    /// A message was created.
    MessageCreated(Box<MessageEnvelope>),
    /// A message envelope was updated.
    MessageUpdated(Box<MessageEnvelope>),
    /// Several messages were deleted.
    MessagesDeleted {
        /// Conversation containing the deleted messages, if known.
        conversation: Option<ConversationRef>,
        /// References of deleted messages.
        messages: Vec<MessageRef>,
        /// Lossless platform-specific event metadata.
        platform_data: Option<PlatformNativeData>,
    },
    /// A transient chat activity changed.
    ActivityChanged(ActivityState),
    /// A user read position changed.
    ReadReceiptUpdated(ReadReceipt),
    /// A call session changed.
    CallUpdated(CallSession),
    /// Reaction aggregates changed.
    ReactionsChanged {
        /// Message whose reactions changed.
        message: MessageRef,
        /// Actor responsible for the change, if known.
        actor: Option<User>,
        /// Reaction delta and current state.
        change: ReactionChange,
    },
    /// A message was pinned.
    MessagePinned(PinnedMessage),
    /// A message was unpinned.
    MessageUnpinned(PinnedMessage),
    /// A thread was created.
    ThreadCreated(Thread),
    /// A thread was updated.
    ThreadUpdated(Thread),
    /// A thread was closed.
    ThreadClosed(Thread),
    /// A thread was deleted.
    ThreadDeleted(Thread),
    /// A poll changed.
    PollUpdated(Poll),
    /// A user's poll vote changed.
    PollVoteChanged {
        /// Platform poll identifier.
        poll_id: String,
        /// Voter, if exposed by the platform.
        user: Option<User>,
        /// Currently selected option identifiers.
        option_ids: Vec<String>,
        /// Lossless platform-specific vote metadata.
        platform_data: Option<PlatformNativeData>,
    },
    /// A checklist was replaced.
    ChecklistUpdated(Checklist),
    /// Checklist task state changed.
    ChecklistChanged(ChecklistChange),
    /// A conversation was created.
    ConversationCreated {
        /// Newly created conversation.
        conversation: ConversationRef,
        /// Initial profile, if exposed.
        profile: Option<Box<ConversationProfile>>,
    },
    /// Conversation profile state changed.
    ConversationUpdated {
        /// Updated conversation.
        conversation: ConversationRef,
        /// Previous profile, if available.
        old_profile: Option<Box<ConversationProfile>>,
        /// Current profile.
        new_profile: Box<ConversationProfile>,
    },
    /// A conversation was archived.
    ConversationArchived(ConversationRef),
    /// A conversation member changed.
    MemberUpdated {
        /// Conversation containing the member.
        conversation: ConversationRef,
        /// Previous member state, if available.
        old_member: Option<Box<ConversationMember>>,
        /// Current member state.
        new_member: Box<ConversationMember>,
    },
    /// A user requested to join a conversation.
    JoinRequested(JoinRequest),
    /// Permissions changed for a user or a conversation default.
    PermissionsChanged {
        /// Conversation whose permissions changed.
        conversation: ConversationRef,
        /// Affected user, or none for default permissions.
        user: Option<User>,
        /// Previous permissions, if available.
        old_permissions: Option<PermissionSet>,
        /// Current permissions.
        new_permissions: PermissionSet,
    },
    /// A file was shared in a conversation.
    FileShared {
        /// Conversation receiving the file.
        conversation: ConversationRef,
        /// User that shared the file, if known.
        user: Option<User>,
        /// Shared file descriptor.
        file: File,
    },
    /// A platform file was deleted.
    FileDeleted {
        /// Conversation that contained the file, if known.
        conversation: Option<ConversationRef>,
        /// Platform file identifier.
        file_id: String,
    },
    /// Dynamic suggestions were requested.
    SuggestionRequested(SuggestionRequest),
    /// A dynamic suggestion was selected.
    SuggestionSelected(SuggestionSelection),
    /// A mini app emitted an event.
    MiniApp(MiniAppEvent),
    /// A shipping quote was requested.
    ShippingRequested(ShippingRequest),
    /// Checkout approval was requested.
    CheckoutRequested(CheckoutRequest),
    /// A payment changed.
    PaymentUpdated(Payment),
    /// A subscription changed.
    SubscriptionUpdated(Subscription),
    /// A scheduled message was sent.
    ScheduledMessageSent(MessageRef),
    /// A scheduled message failed.
    ScheduledMessageFailed {
        /// Caller-supplied message identifier, if provided.
        client_message_id: Option<String>,
        /// Human-readable failure detail.
        error: String,
    },
}

/// Details for a group-member join.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupMemberJoinedEvent {
    /// Group conversation.
    pub conversation: ConversationRef,
    /// Member that joined.
    pub user: User,
    /// Reason reported for the join.
    pub reason: GroupMemberJoinReason,
}

/// Details for a group-member departure.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupMemberLeftEvent {
    /// Group conversation.
    pub conversation: ConversationRef,
    /// Member that departed.
    pub user: User,
    /// Reason reported for the departure.
    pub reason: GroupMemberLeaveReason,
}

/// Details for an administrator-state change.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupAdminChangedEvent {
    /// Group conversation.
    pub conversation: ConversationRef,
    /// Member whose administrator state changed.
    pub user: User,
    /// Current administrator state.
    pub state: GroupAdminState,
}

/// Details for a group-wide mute-state change.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupMuteChangedEvent {
    /// Group conversation.
    pub conversation: ConversationRef,
    /// User that made the change, if known.
    pub operator: Option<User>,
    /// Current mute state.
    pub state: MuteState,
}

/// Details for one group member's mute-state change.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupMemberMuteChangedEvent {
    /// Group conversation.
    pub conversation: ConversationRef,
    /// Member whose state changed.
    pub user: User,
    /// User that made the change, if known.
    pub operator: Option<User>,
    /// Current mute state.
    pub state: MuteState,
}

/// Details for a group highlight-state change.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupHighlightChangedEvent {
    /// Group conversation.
    pub conversation: ConversationRef,
    /// Current highlight state.
    pub state: GroupHighlightState,
    /// Message associated with the highlight.
    pub message: Message,
    /// Message sender, if known.
    pub sender: Option<User>,
    /// User that made the change, if known.
    pub operator: Option<User>,
}

/// Details for a group member alias change.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupMemberAliasChangedEvent {
    /// Group conversation.
    pub conversation: ConversationRef,
    /// Member whose alias changed.
    pub user: User,
    /// User that made the change, if known.
    pub operator: Option<User>,
    /// Previous alias, if known.
    pub old_alias: Option<String>,
    /// Current alias, if known.
    pub new_alias: Option<String>,
}

/// Details for a deleted message.
#[derive(Debug, Clone, PartialEq)]
pub struct MessageDeletedEvent {
    /// Original sender, if known.
    pub user: Option<User>,
    /// User that deleted the message, if known.
    pub operator: Option<User>,
    /// Conversation containing the message, if known.
    pub conversation: Option<ConversationRef>,
    /// Deleted message content, if retained by the platform.
    pub message: Option<Message>,
}

/// Reason a member joined a group.
#[derive(Debug, Clone, PartialEq)]
pub enum GroupMemberJoinReason {
    /// A join request was approved.
    Approved {
        /// User that approved the request, if known.
        operator: Option<Box<User>>,
    },
    /// The user was invited.
    Invited {
        /// User that sent the invitation, if known.
        inviter: Option<Box<User>>,
        /// User that approved or processed the invitation, if known.
        operator: Option<Box<User>>,
    },
    /// Platform did not expose a reason.
    Unknown,
}

/// Reason a member left a group.
#[derive(Debug, Clone, PartialEq)]
pub enum GroupMemberLeaveReason {
    /// A user was removed by an operator.
    Kicked {
        /// User that removed the member, if known.
        operator: Option<User>,
    },
    /// The bot was removed by an operator.
    BotKicked {
        /// User that removed the bot, if known.
        operator: Option<User>,
    },
    /// The member left voluntarily.
    Left,
    /// Platform did not expose a reason.
    Unknown,
}

/// Current administrator state for a member.
#[derive(Debug, Clone, PartialEq)]
pub enum GroupAdminState {
    /// Administrator privileges were granted.
    Granted,
    /// Administrator privileges were revoked.
    Revoked,
    /// Platform did not expose a state.
    Unknown,
}

/// Current highlight state for a group feature.
#[derive(Debug, Clone, PartialEq)]
pub enum GroupHighlightState {
    /// Highlight was enabled.
    Enabled,
    /// Highlight was disabled.
    Disabled,
    /// Platform did not expose a state.
    Unknown,
}

/// Current group or member mute state.
#[derive(Debug, Clone, PartialEq)]
pub enum MuteState {
    /// Muted, optionally until a duration expires.
    Muted {
        /// Mute duration, when supplied by the platform.
        duration: Option<Duration>,
    },
    /// Not muted.
    Unmuted,
    /// Platform did not expose a state.
    Unknown,
}

/// Details for an edited message.
#[derive(Debug, Clone, PartialEq)]
pub struct MessageEditedEvent {
    /// Message author.
    pub user: User,
    /// Conversation containing the message, if known.
    pub conversation: Option<ConversationRef>,
    /// Current message content, if retained.
    pub new_message: Option<Message>,
    /// User that made the edit, if known.
    pub operator: Option<User>,
    /// Previous message content, if retained.
    pub old_message: Option<Message>,
}

/// Details for a legacy platform reaction-change event.
#[derive(Debug, Clone, PartialEq)]
pub struct MessageReactionsChangedEvent {
    /// User associated with the reaction event.
    pub user: User,
    /// Conversation containing the message, if known.
    pub conversation: Option<ConversationRef>,
    /// Message whose reactions changed.
    pub message: Message,
    /// Platform reaction strings.
    pub reactions: Vec<String>,
}
