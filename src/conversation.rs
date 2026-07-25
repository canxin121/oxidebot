//! Cross-platform conversation, thread, membership, permission, and paging models.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};

use crate::{
    interaction::PlatformNativeData,
    source::{message::File, user::User},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ConversationKind {
    Direct,
    Group,
    Channel,
    Thread,
    Topic,
    Forum,
    #[default]
    Unknown,
    PlatformNative(String),
}

/// A platform conversation address. `parent` represents nesting such as a
/// thread inside a channel or a topic inside a forum.
#[derive(Clone, Debug, PartialEq)]
pub struct ConversationRef {
    pub id: String,
    pub kind: ConversationKind,
    pub parent: Option<Box<ConversationRef>>,
    pub platform_data: Option<PlatformNativeData>,
}

impl ConversationRef {
    pub fn new(id: impl Into<String>, kind: ConversationKind) -> Self {
        Self {
            id: id.into(),
            kind,
            parent: None,
            platform_data: None,
        }
    }

    pub fn direct(id: impl Into<String>) -> Self {
        Self::new(id, ConversationKind::Direct)
    }

    pub fn group(id: impl Into<String>) -> Self {
        Self::new(id, ConversationKind::Group)
    }

    pub fn child_of(mut self, parent: ConversationRef) -> Self {
        self.parent = Some(Box::new(parent));
        self
    }

    pub fn platform_data(mut self, platform_data: PlatformNativeData) -> Self {
        self.platform_data = Some(platform_data);
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MessageRef {
    pub id: String,
    pub conversation: Option<ConversationRef>,
    pub platform_data: Option<PlatformNativeData>,
}

impl MessageRef {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            conversation: None,
            platform_data: None,
        }
    }

    pub fn in_conversation(mut self, conversation: ConversationRef) -> Self {
        self.conversation = Some(conversation);
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MessageTarget {
    pub conversation: ConversationRef,
    /// Optional recipients for platforms that address multiple users inside a
    /// conversation or support a fan-out request.
    pub recipients: Vec<String>,
    pub platform_data: Option<PlatformNativeData>,
}

impl MessageTarget {
    pub fn new(conversation: ConversationRef) -> Self {
        Self {
            conversation,
            recipients: Vec::new(),
            platform_data: None,
        }
    }

    pub fn recipients(mut self, recipients: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.recipients = recipients.into_iter().map(Into::into).collect();
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageRequest {
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
    pub previous_cursor: Option<String>,
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

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConversationPermission {
    SendMessages,
    SendMedia,
    SendPolls,
    SendReactions,
    AddMembers,
    RemoveMembers,
    BanMembers,
    PinMessages,
    ManageMessages,
    ManageThreads,
    ManageProfile,
    ManageInvites,
    ManageRoles,
    ManageTags,
    EditOwnTag,
    ManageCalls,
    #[default]
    Unknown,
    PlatformNative(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PermissionSet {
    pub allow: BTreeSet<ConversationPermission>,
    pub deny: BTreeSet<ConversationPermission>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoleRef {
    pub id: String,
    pub name: Option<String>,
    pub permissions: PermissionSet,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConversationMember {
    pub user: User,
    pub roles: Vec<RoleRef>,
    pub permissions: PermissionSet,
    pub joined_at: Option<DateTime<Utc>>,
    pub nickname: Option<String>,
    /// A short member label or tag, distinct from an administrator title.
    pub tag: Option<String>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConversationProfile {
    pub name: Option<String>,
    pub username: Option<String>,
    pub description: Option<String>,
    pub avatar: Option<File>,
    pub member_count: Option<u64>,
    pub kind: Option<ConversationKind>,
    pub parent: Option<ConversationRef>,
    pub default_permissions: Option<PermissionSet>,
    pub slow_mode: Option<std::time::Duration>,
    pub archived: Option<bool>,
    pub created_at: Option<DateTime<Utc>>,
    pub locale: Option<String>,
    pub metadata: BTreeMap<String, serde_json::Value>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ThreadState {
    #[default]
    Open,
    Closed,
    Archived,
    Hidden,
    Deleted,
    PlatformNative(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Thread {
    pub conversation: ConversationRef,
    pub title: Option<String>,
    pub creator: Option<User>,
    pub state: ThreadState,
    pub created_at: Option<DateTime<Utc>>,
    pub auto_close_at: Option<DateTime<Utc>>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThreadOptions {
    pub title: Option<String>,
    pub auto_close_after: Option<std::time::Duration>,
    pub invitable: Option<bool>,
    pub slow_mode: Option<std::time::Duration>,
    pub icon: Option<String>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct InviteLinkOptions {
    pub name: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub max_uses: Option<u32>,
    pub require_approval: bool,
    pub temporary_membership: bool,
    pub subscription_price: Option<crate::commerce::Money>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InviteLink {
    pub id: String,
    pub url: String,
    pub conversation: ConversationRef,
    pub creator: Option<User>,
    pub options: InviteLinkOptions,
    pub use_count: Option<u64>,
    pub revoked: bool,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct JoinRequest {
    pub id: String,
    pub conversation: ConversationRef,
    pub user: User,
    pub message: Option<String>,
    pub requested_at: Option<DateTime<Utc>>,
    pub platform_data: Option<PlatformNativeData>,
}
