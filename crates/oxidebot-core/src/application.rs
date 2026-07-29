//! Application commands, dynamic suggestions, mini apps, persistent surfaces,
//! and localized bot profiles.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use crate::{
    content::{FormValue, RichLayout},
    conversation::{ConversationPermission, ConversationRef, PermissionSet},
    interaction::{Modal, PlatformNativeData},
    source::{
        message::{File, Message},
        user::User,
    },
};

/// A value with a default representation and optional locale-specific overrides.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Localized<T> {
    /// Value used when no matching translation exists.
    pub default: T,
    /// Exact-locale values keyed by BCP 47 locale tag.
    pub translations: BTreeMap<String, T>,
}

impl<T> Localized<T> {
    /// Creates a localized value with no translations.
    #[must_use]
    pub fn new(default: T) -> Self {
        Self {
            default,
            translations: BTreeMap::new(),
        }
    }

    /// Adds or replaces one locale-specific value.
    #[must_use]
    pub fn translation(mut self, locale: impl Into<String>, value: T) -> Self {
        self.translations.insert(locale.into(), value);
        self
    }

    /// Resolves an exact locale, then its language, then the default value.
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

/// Kind of a platform-visible application command.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandKind {
    /// Text or slash-style command entered in a conversation.
    #[default]
    ChatInput,
    /// Context command that targets a user.
    User,
    /// Context command that targets a message.
    Message,
    /// A command kind without a portable OxideBot equivalent.
    PlatformNative(String),
}

/// Context in which a structured command may be invoked.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CommandContext {
    /// Direct conversation with the bot.
    Direct,
    /// Group or room conversation.
    Group,
    /// Broadcast channel or similar one-way conversation.
    Channel,
    /// Invocation by an administrator.
    Administrator,
    /// Any context supported by the platform.
    #[default]
    Any,
    /// A context without a portable OxideBot equivalent.
    PlatformNative(String),
}

/// One named choice offered by a structured command option.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandChoice {
    /// Localized label visible to the user.
    pub name: Localized<String>,
    /// Value submitted when this choice is selected.
    pub value: String,
}

/// Data kind accepted by a structured command option.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandOptionType {
    /// A nested command below the root command.
    Subcommand,
    /// A group of nested subcommands.
    SubcommandGroup,
    /// Arbitrary string input.
    String,
    /// Signed integer input.
    Integer,
    /// Floating-point numeric input.
    Number,
    /// Boolean input.
    Boolean,
    /// User selection.
    User,
    /// Conversation or channel selection.
    Conversation,
    /// Role selection.
    Role,
    /// Selection that may refer to a user or role.
    Mentionable,
    /// Uploaded attachment input.
    Attachment,
    /// An option kind without a portable OxideBot equivalent.
    PlatformNative(String),
}

/// One node in a structured application-command schema.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandOption {
    /// Stable OxideBot node or field identifier. Adapters should preserve it
    /// when the platform can echo opaque metadata in autocomplete events.
    #[serde(default)]
    pub id: Option<String>,
    /// Localized option name presented by the platform.
    pub name: Localized<String>,
    /// Localized explanation presented by the platform.
    pub description: Localized<String>,
    /// Shape of values accepted by this option.
    pub kind: CommandOptionType,
    /// Whether the platform must receive a value for this option.
    pub required: bool,
    /// Fixed values offered by this option.
    pub choices: Vec<CommandChoice>,
    /// Nested options for subcommand or subcommand-group nodes.
    pub options: Vec<CommandOption>,
    /// Whether the platform may request dynamic suggestions while typing.
    pub autocomplete: bool,
    /// Inclusive minimum numeric value, when applicable.
    pub min_value: Option<f64>,
    /// Inclusive maximum numeric value, when applicable.
    pub max_value: Option<f64>,
    /// Inclusive minimum text length, when applicable.
    pub min_length: Option<u32>,
    /// Inclusive maximum text length, when applicable.
    pub max_length: Option<u32>,
    /// Conversation kinds selectable by a conversation option.
    pub allowed_conversation_kinds: Vec<crate::conversation::ConversationKind>,
    /// Lossless platform-specific option metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// A platform-visible application command definition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandDefinition {
    /// Platform-assigned command identifier, if known.
    pub id: Option<String>,
    /// Localized command name.
    pub name: Localized<String>,
    /// Localized command description.
    pub description: Localized<String>,
    /// Command invocation kind.
    pub kind: CommandKind,
    /// Root-level command options.
    pub options: Vec<CommandOption>,
    /// Optional default permissions required to invoke the command.
    pub default_permissions: Option<PermissionSet>,
    /// Contexts in which the command is available.
    pub contexts: BTreeSet<CommandContext>,
    /// Whether command responses default to ephemeral visibility.
    pub ephemeral: bool,
    /// Lossless platform-specific command metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// A normalized invocation of a structured application command.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandInvocation {
    /// Stable platform command identifier, if exposed.
    pub command_id: Option<String>,
    /// Root command name.
    pub name: String,
    /// Selected nested subcommand path.
    pub path: Vec<String>,
    /// Values grouped by their command option name.
    pub options: BTreeMap<String, Vec<FormValue>>,
    /// Original textual invocation when the platform exposed it.
    pub raw_text: Option<String>,
    /// User targeted by a user context command.
    pub target_user: Option<User>,
    /// Message targeted by a message context command.
    pub target_message: Option<crate::conversation::MessageRef>,
    /// Permissions known for the invoking user.
    pub permissions: BTreeSet<ConversationPermission>,
    /// User locale supplied by the platform.
    pub locale: Option<String>,
    /// Lossless platform-specific invocation metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// A request for dynamic command-option or inline suggestions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SuggestionRequest {
    /// Platform request identifier used when sending the answer.
    pub id: String,
    /// User-entered search text.
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
    /// Stable option or field identifier receiving the suggestions.
    pub field_id: Option<String>,
    /// Requesting user's locale, when supplied by the platform.
    #[serde(default)]
    pub locale: Option<String>,
    /// User requesting suggestions.
    pub user: User,
    /// Conversation in which the request occurred, if any.
    pub conversation: Option<ConversationRef>,
    /// Maximum number of suggestions requested by the platform.
    pub limit: Option<u32>,
    /// Opaque pagination cursor from the platform.
    pub cursor: Option<String>,
    /// Lossless platform-specific request metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// One dynamic suggestion or inline-query result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Suggestion {
    /// Stable result identifier returned if the user selects this item.
    pub id: String,
    /// Primary user-visible title.
    pub title: String,
    /// Optional secondary user-visible description.
    pub description: Option<String>,
    /// Optional thumbnail or preview image.
    pub image: Option<File>,
    /// Value inserted into the active command field.
    pub value: FormValue,
    /// Optional message sent when this result is selected.
    pub message: Option<Message>,
    /// Lossless platform-specific result metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// A suggestion or inline-query result selected by a user.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SuggestionSelection {
    /// Identifier of the selected suggestion result.
    pub result_id: String,
    /// Original query text, when supplied by the platform.
    pub query: Option<String>,
    /// User who selected the result.
    pub user: User,
    /// Conversation in which the result was selected, if any.
    pub conversation: Option<ConversationRef>,
    /// Identifier of the message created for the selected result, if any.
    pub message_id: Option<String>,
    /// Lossless platform-specific selection metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// Presentation mode for a mini app launch.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MiniAppMode {
    /// Open inside the platform's normal embedded view.
    #[default]
    Embedded,
    /// Open in a platform full-screen view.
    FullScreen,
    /// Open outside the platform's embedded UI.
    External,
    /// A presentation mode without a portable OxideBot equivalent.
    PlatformNative(String),
}

/// Data needed to launch a mini app.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MiniAppLaunch {
    /// Absolute URL loaded by the mini app.
    pub url: String,
    /// Requested presentation mode.
    pub mode: MiniAppMode,
    /// Signed or platform-native initialization payload.
    pub initialization_data: Option<serde_json::Value>,
    /// Lossless platform-specific launch metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// Verification result for platform-provided mini-app data.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerificationState {
    /// The adapter validated the supplied data.
    Verified,
    /// The adapter attempted validation and it failed.
    Invalid,
    /// The adapter cannot validate the supplied data.
    #[default]
    Unverified,
}

/// Incoming event emitted by a mini app.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MiniAppEvent {
    /// Platform query identifier that may require an answer.
    pub query_id: Option<String>,
    /// User that triggered the mini-app event.
    pub user: User,
    /// Conversation associated with the event, if any.
    pub conversation: Option<ConversationRef>,
    /// Lossless mini-app data payload.
    pub data: serde_json::Value,
    /// Result of validating platform-supplied initialization data.
    pub verification: VerificationState,
    /// Lossless platform-specific event metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// Kind of persistent or platform-managed application surface.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppSurfaceKind {
    /// Menu shown in a conversation UI.
    ChatMenu,
    /// Bot or application home surface.
    Home,
    /// Modal dialog surface.
    Modal,
    /// Side-panel surface.
    Sidebar,
    /// Platform rich-menu surface.
    RichMenu,
    /// Unknown portable surface kind.
    #[default]
    Unknown,
    /// A surface kind without a portable OxideBot equivalent.
    PlatformNative(String),
}

/// Content rendered inside an application surface.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SurfaceContent {
    /// Portable rich layout content.
    Layout(RichLayout),
    /// Portable modal content.
    Modal(Modal),
    /// Mini-app launch content.
    MiniApp(MiniAppLaunch),
    /// Lossless platform-native surface content.
    PlatformNative(PlatformNativeData),
}

/// A published application surface scoped to a user or conversation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppSurface {
    /// Platform-assigned surface identifier, if known.
    pub id: Option<String>,
    /// Optional user-visible surface title.
    pub title: Option<String>,
    /// Platform presentation kind.
    pub kind: AppSurfaceKind,
    /// Content rendered by the surface.
    pub content: SurfaceContent,
    /// Optional user scope for the surface.
    pub user_id: Option<String>,
    /// Optional conversation scope for the surface.
    pub conversation: Option<ConversationRef>,
    /// Lossless platform-specific surface metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// Localized profile data displayed for a bot by a platform.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BotProfile {
    /// Display names keyed by locale.
    pub names: BTreeMap<String, String>,
    /// Full descriptions keyed by locale.
    pub descriptions: BTreeMap<String, String>,
    /// Short descriptions keyed by locale.
    pub short_descriptions: BTreeMap<String, String>,
    /// Optional profile avatar.
    pub avatar: Option<File>,
    /// Optional default bot permissions.
    pub default_permissions: Option<PermissionSet>,
    /// Lossless platform-specific profile metadata.
    pub platform_data: Option<PlatformNativeData>,
}
