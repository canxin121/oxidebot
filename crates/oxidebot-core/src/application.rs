//! Application commands, dynamic suggestions, mini apps, persistent surfaces,
//! and localized bot profiles.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use crate::{
    content::{FormValue, OutgoingMessage, RichLayout},
    conversation::{ConversationPermission, ConversationRef, PermissionSet},
    interaction::{Modal, PlatformNativeData},
    source::{message::File, user::User},
};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Localized<T> {
    pub default: T,
    pub translations: BTreeMap<String, T>,
}

impl<T> Localized<T> {
    #[must_use]
    pub fn new(default: T) -> Self {
        Self {
            default,
            translations: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn translation(mut self, locale: impl Into<String>, value: T) -> Self {
        self.translations.insert(locale.into(), value);
        self
    }

    #[must_use]
    pub fn resolve(&self, locale: Option<&str>) -> &T {
        let Some(locale) = locale else {
            return &self.default;
        };
        self.translations
            .get(locale)
            .or_else(|| {
                locale
                    .split_once('-')
                    .and_then(|(language, _)| self.translations.get(language))
            })
            .unwrap_or(&self.default)
    }
}

impl<T> From<T> for Localized<T> {
    fn from(default: T) -> Self {
        Self::new(default)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandKind {
    #[default]
    ChatInput,
    User,
    Message,
    PlatformNative(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CommandContext {
    Direct,
    Group,
    Channel,
    Administrator,
    #[default]
    Any,
    PlatformNative(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandChoice {
    pub name: Localized<String>,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandOptionType {
    Subcommand,
    SubcommandGroup,
    String,
    Integer,
    Number,
    Boolean,
    User,
    Conversation,
    Role,
    Mentionable,
    Attachment,
    PlatformNative(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandOption {
    /// Stable OxideBot node or field identifier. Adapters should preserve it
    /// when the platform can echo opaque metadata in autocomplete events.
    #[serde(default)]
    pub id: Option<String>,
    pub name: Localized<String>,
    pub description: Localized<String>,
    pub kind: CommandOptionType,
    pub required: bool,
    pub choices: Vec<CommandChoice>,
    pub options: Vec<CommandOption>,
    pub autocomplete: bool,
    pub min_value: Option<f64>,
    pub max_value: Option<f64>,
    pub min_length: Option<u32>,
    pub max_length: Option<u32>,
    pub allowed_conversation_kinds: Vec<crate::conversation::ConversationKind>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandDefinition {
    pub id: Option<String>,
    pub name: Localized<String>,
    pub description: Localized<String>,
    pub kind: CommandKind,
    pub options: Vec<CommandOption>,
    pub default_permissions: Option<PermissionSet>,
    pub contexts: BTreeSet<CommandContext>,
    pub ephemeral: bool,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandInvocation {
    pub command_id: Option<String>,
    pub name: String,
    pub path: Vec<String>,
    pub options: BTreeMap<String, Vec<FormValue>>,
    pub raw_text: Option<String>,
    pub target_user: Option<User>,
    pub target_message: Option<crate::conversation::MessageRef>,
    pub permissions: BTreeSet<ConversationPermission>,
    pub locale: Option<String>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SuggestionRequest {
    pub id: String,
    pub query: String,
    /// Stable OxideBot command ID when the platform exposes the originating
    /// structured command. Older adapters may leave this empty.
    #[serde(default)]
    pub command_id: Option<String>,
    /// Human-readable command name used as a fallback when no stable ID is
    /// available.
    #[serde(default)]
    pub command_name: Option<String>,
    /// Selected subcommand path, excluding the root command.
    #[serde(default)]
    pub command_path: Vec<String>,
    pub field_id: Option<String>,
    #[serde(default)]
    pub locale: Option<String>,
    pub user: User,
    pub conversation: Option<ConversationRef>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Suggestion {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub image: Option<File>,
    pub value: FormValue,
    pub message: Option<OutgoingMessage>,
    pub platform_data: Option<PlatformNativeData>,
}

/// A suggestion or inline-query result selected by a user.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SuggestionSelection {
    pub result_id: String,
    pub query: Option<String>,
    pub user: User,
    pub conversation: Option<ConversationRef>,
    pub message_id: Option<String>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MiniAppMode {
    #[default]
    Embedded,
    FullScreen,
    External,
    PlatformNative(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MiniAppLaunch {
    pub url: String,
    pub mode: MiniAppMode,
    pub initialization_data: Option<serde_json::Value>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerificationState {
    Verified,
    Invalid,
    #[default]
    Unverified,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MiniAppEvent {
    pub query_id: Option<String>,
    pub user: User,
    pub conversation: Option<ConversationRef>,
    pub data: serde_json::Value,
    pub verification: VerificationState,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppSurfaceKind {
    ChatMenu,
    Home,
    Modal,
    Sidebar,
    RichMenu,
    #[default]
    Unknown,
    PlatformNative(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SurfaceContent {
    Layout(RichLayout),
    Modal(Modal),
    MiniApp(MiniAppLaunch),
    PlatformNative(PlatformNativeData),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppSurface {
    pub id: Option<String>,
    pub title: Option<String>,
    pub kind: AppSurfaceKind,
    pub content: SurfaceContent,
    pub user_id: Option<String>,
    pub conversation: Option<ConversationRef>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BotProfile {
    pub names: BTreeMap<String, String>,
    pub descriptions: BTreeMap<String, String>,
    pub short_descriptions: BTreeMap<String, String>,
    pub avatar: Option<File>,
    pub default_permissions: Option<PermissionSet>,
    pub platform_data: Option<PlatformNativeData>,
}
