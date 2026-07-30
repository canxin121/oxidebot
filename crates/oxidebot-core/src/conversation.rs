//! Cross-platform conversation, thread, membership, permission, and paging models.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};

use crate::{
    interaction::PlatformNativeData,
    source::{message::File, user::User},
    ConversationId, MessageId, RoleId, UserId,
};

/// Portable kind of a conversation address.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConversationKind {
    /// Direct one-to-one conversation.
    Direct,
    /// Group conversation.
    Group,
    /// Broadcast channel.
    Channel,
    /// Thread nested in a conversation.
    Thread,
    /// Topic nested in a conversation.
    Topic,
    /// Forum-style conversation.
    Forum,
    /// Unknown portable kind.
    #[default]
    Unknown,
    /// Platform-native conversation kind.
    PlatformNative(String),
}

/// A platform conversation address. `parent` represents nesting such as a
/// thread inside a channel or a topic inside a forum.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConversationRef {
    /// Platform-local conversation identifier.
    pub id: ConversationId,
    /// Portable conversation kind.
    pub kind: ConversationKind,
    /// Parent conversation for threads or topics.
    pub parent: Option<Box<ConversationRef>>,
    /// Lossless platform-specific address metadata.
    pub platform_data: Option<PlatformNativeData>,
}

impl ConversationRef {
    /// Creates a root conversation address.
    pub fn new(id: impl Into<ConversationId>, kind: ConversationKind) -> Self {
        Self {
            id: id.into(),
            kind,
            parent: None,
            platform_data: None,
        }
    }

    /// Creates a direct conversation address.
    pub fn direct(id: impl Into<ConversationId>) -> Self {
        Self::new(id, ConversationKind::Direct)
    }

    /// Creates a direct conversation addressed by a platform user ID without
    /// losing whether that platform ID was numeric or textual.
    #[must_use]
    pub fn direct_user(user_id: impl Into<UserId>) -> Self {
        let user_id = user_id.into();
        Self::direct(ConversationId::from(user_id.as_compact().clone()))
    }

    /// Creates a group conversation address.
    pub fn group(id: impl Into<ConversationId>) -> Self {
        Self::new(id, ConversationKind::Group)
    }

    /// Nests this address under a parent conversation.
    pub fn child_of(mut self, parent: ConversationRef) -> Self {
        self.parent = Some(Box::new(parent));
        self
    }

    /// Attaches lossless platform-specific address metadata.
    pub fn platform_data(mut self, platform_data: PlatformNativeData) -> Self {
        self.platform_data = Some(platform_data);
        self
    }
}

/// Reference to one platform message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MessageRef {
    /// Platform-local message identifier.
    pub id: MessageId,
    /// Conversation containing the message, if known.
    pub conversation: Option<ConversationRef>,
    /// Lossless platform-specific message-reference metadata.
    pub platform_data: Option<PlatformNativeData>,
}

impl MessageRef {
    /// Creates an unscoped message reference.
    pub fn new(id: impl Into<MessageId>) -> Self {
        Self {
            id: id.into(),
            conversation: None,
            platform_data: None,
        }
    }

    /// Associates the reference with its containing conversation.
    pub fn in_conversation(mut self, conversation: ConversationRef) -> Self {
        self.conversation = Some(conversation);
        self
    }
}

/// Destination for a portable outbound message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MessageTarget {
    /// Conversation receiving the message.
    pub conversation: ConversationRef,
    /// Optional recipients for platforms that address multiple users inside a
    /// conversation or support a fan-out request.
    pub recipients: Vec<UserId>,
    /// Lossless platform-specific targeting metadata.
    pub platform_data: Option<PlatformNativeData>,
}

impl MessageTarget {
    /// Creates a target for the whole conversation.
    pub fn new(conversation: ConversationRef) -> Self {
        Self {
            conversation,
            recipients: Vec::new(),
            platform_data: None,
        }
    }

    /// Replaces explicit recipient identifiers.
    pub fn recipients(mut self, recipients: impl IntoIterator<Item = impl Into<UserId>>) -> Self {
        self.recipients = recipients.into_iter().map(Into::into).collect();
        self
    }
}

/// Cursor and limit used for a paged platform query.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageRequest {
    /// Opaque continuation cursor.
    pub cursor: Option<String>,
    /// Maximum items requested.
    pub limit: Option<u32>,
}

/// One page of platform results.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Page<T> {
    /// Items in this page.
    pub items: Vec<T>,
    /// Cursor for the following page.
    pub next_cursor: Option<String>,
    /// Cursor for the preceding page.
    pub previous_cursor: Option<String>,
    /// Total item count, if exposed.
    pub total: Option<u64>,
}

impl<T> Default for Page<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            next_cursor: None,
            previous_cursor: None,
            total: None,
        }
    }
}

/// Permission that may be allowed or denied in a conversation.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ConversationPermission {
    /// Send text messages.
    SendMessages,
    /// Send media attachments.
    SendMedia,
    /// Create polls.
    SendPolls,
    /// Send reactions.
    SendReactions,
    /// Add conversation members.
    AddMembers,
    /// Remove conversation members.
    RemoveMembers,
    /// Ban conversation members.
    BanMembers,
    /// Pin messages.
    PinMessages,
    /// Edit or delete messages.
    ManageMessages,
    /// Create and manage threads.
    ManageThreads,
    /// Edit conversation profile.
    ManageProfile,
    /// Create and manage invite links.
    ManageInvites,
    /// Create and manage roles.
    ManageRoles,
    /// Create and manage member tags.
    ManageTags,
    /// Edit the caller's own tag.
    EditOwnTag,
    /// Create and manage calls.
    ManageCalls,
    /// Unknown portable permission.
    #[default]
    Unknown,
    /// Platform-native permission.
    PlatformNative(String),
}

/// Explicit allow and deny permission sets.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionSet {
    /// Explicitly allowed permissions.
    pub allow: BTreeSet<ConversationPermission>,
    /// Explicitly denied permissions.
    pub deny: BTreeSet<ConversationPermission>,
}

/// Role assigned to a conversation member.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleRef {
    /// Platform role identifier.
    pub id: RoleId,
    /// Optional user-visible role name.
    pub name: Option<String>,
    /// Permissions granted by the role.
    pub permissions: PermissionSet,
}

/// One member of a conversation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConversationMember {
    /// Member user identity.
    pub user: User,
    /// Roles assigned to the member.
    pub roles: Vec<RoleRef>,
    /// Explicit member permissions.
    pub permissions: PermissionSet,
    /// Join time, if known.
    pub joined_at: Option<DateTime<Utc>>,
    /// Optional member nickname.
    pub nickname: Option<String>,
    /// A short member label or tag, distinct from an administrator title.
    pub tag: Option<String>,
    /// Lossless platform-specific member metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// Mutable profile metadata for a conversation.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ConversationProfile {
    /// User-visible display name.
    pub name: Option<String>,
    /// Public username or handle.
    pub username: Option<String>,
    /// User-visible description.
    pub description: Option<String>,
    /// Optional avatar.
    pub avatar: Option<File>,
    /// Current member count, if exposed.
    pub member_count: Option<u64>,
    /// Conversation kind, if exposed.
    pub kind: Option<ConversationKind>,
    /// Parent conversation for nested spaces.
    pub parent: Option<ConversationRef>,
    /// Default member permissions.
    pub default_permissions: Option<PermissionSet>,
    /// Slow-mode interval.
    pub slow_mode: Option<std::time::Duration>,
    /// Whether the conversation is archived.
    pub archived: Option<bool>,
    /// Creation time, if known.
    pub created_at: Option<DateTime<Utc>>,
    /// Conversation locale.
    pub locale: Option<String>,
    /// Adapter-independent metadata.
    pub metadata: BTreeMap<String, serde_json::Value>,
    /// Lossless platform-specific profile metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// Lifecycle state of a thread or topic.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThreadState {
    /// Open for messages.
    #[default]
    Open,
    /// Closed for messages.
    Closed,
    /// Archived by the platform.
    Archived,
    /// Hidden from normal display.
    Hidden,
    /// Deleted by the platform.
    Deleted,
    /// Platform-native thread state.
    PlatformNative(String),
}

/// Thread or topic metadata.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Thread {
    /// Conversation address of the thread.
    pub conversation: ConversationRef,
    /// Optional user-visible title.
    pub title: Option<String>,
    /// Thread creator, if known.
    pub creator: Option<User>,
    /// Current thread state.
    pub state: ThreadState,
    /// Creation time, if known.
    pub created_at: Option<DateTime<Utc>>,
    /// Automatic close time, if known.
    pub auto_close_at: Option<DateTime<Utc>>,
    /// Lossless platform-specific thread metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// Options used to create or update a thread.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ThreadOptions {
    /// Optional thread title.
    pub title: Option<String>,
    /// Automatic close duration.
    pub auto_close_after: Option<std::time::Duration>,
    /// Whether non-members may be invited.
    pub invitable: Option<bool>,
    /// Slow-mode interval.
    pub slow_mode: Option<std::time::Duration>,
    /// Optional icon identifier.
    pub icon: Option<String>,
    /// Lossless platform-specific thread options.
    pub platform_data: Option<PlatformNativeData>,
}

/// Options used to create an invite link.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InviteLinkOptions {
    /// Optional user-visible name.
    pub name: Option<String>,
    /// Expiration time.
    pub expires_at: Option<DateTime<Utc>>,
    /// Maximum uses.
    pub max_uses: Option<u32>,
    /// Whether joins require approval.
    pub require_approval: bool,
    /// Whether membership is temporary.
    pub temporary_membership: bool,
    /// Optional subscription price.
    pub subscription_price: Option<crate::commerce::Money>,
    /// Lossless platform-specific invite metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// Invite link issued for a conversation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InviteLink {
    /// Platform invite identifier.
    pub id: String,
    /// Invite URL.
    pub url: String,
    /// Conversation joined by the link.
    pub conversation: ConversationRef,
    /// Link creator, if known.
    pub creator: Option<User>,
    /// Link constraints.
    pub options: InviteLinkOptions,
    /// Current use count, if exposed.
    pub use_count: Option<u64>,
    /// Whether the link is revoked.
    pub revoked: bool,
    /// Lossless platform-specific invite metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// Pending request to join a conversation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JoinRequest {
    /// Platform request identifier.
    pub id: String,
    /// Conversation requested.
    pub conversation: ConversationRef,
    /// User requesting to join.
    pub user: User,
    /// Optional user-supplied message.
    pub message: Option<String>,
    /// Request time, if known.
    pub requested_at: Option<DateTime<Utc>>,
    /// Lossless platform-specific join metadata.
    pub platform_data: Option<PlatformNativeData>,
}
