//! Platform-visible bot command, menu, and interaction capability contracts.

use super::*;

/// Platform-visible bot command definition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotCommand {
    /// Command invocation text without a prefix.
    pub command: String,
    /// User-visible command description.
    pub description: String,
    /// Whether responses to this command should be private to the invoking
    /// user on platforms that support ephemeral commands.
    pub is_ephemeral: bool,
}

impl BotCommand {
    /// Creates a non-ephemeral command definition.
    pub fn new(command: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            description: description.into(),
            is_ephemeral: false,
        }
    }

    /// Sets whether responses should be private where supported.
    pub fn ephemeral(mut self, is_ephemeral: bool) -> Self {
        self.is_ephemeral = is_ephemeral;
        self
    }
}

/// Scope and locale query for retrieving bot commands.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BotCommandQuery {
    /// Scope to query.
    pub scope: CommandScope,
    /// Optional BCP 47 language code.
    pub language_code: Option<String>,
}

impl BotCommandQuery {
    /// Replaces the command scope.
    pub fn scope(mut self, scope: CommandScope) -> Self {
        self.scope = scope;
        self
    }

    /// Sets the command language code.
    pub fn language_code(mut self, language_code: impl Into<String>) -> Self {
        self.language_code = Some(language_code.into());
        self
    }
}

/// Command definitions installed for one scope and locale.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BotCommandSet {
    /// Commands to install.
    pub commands: Vec<BotCommand>,
    /// Scope in which the commands are visible.
    pub scope: CommandScope,
    /// Optional BCP 47 language code.
    pub language_code: Option<String>,
}

impl BotCommandSet {
    /// Creates a set with default scope and no locale override.
    pub fn new(commands: impl IntoIterator<Item = BotCommand>) -> Self {
        Self {
            commands: commands.into_iter().collect(),
            scope: CommandScope::Default,
            language_code: None,
        }
    }

    /// Replaces the command scope.
    pub fn scope(mut self, scope: CommandScope) -> Self {
        self.scope = scope;
        self
    }

    /// Sets the command language code.
    pub fn language_code(mut self, language_code: impl Into<String>) -> Self {
        self.language_code = Some(language_code.into());
        self
    }
}

/// Platform scope in which bot commands are visible.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum CommandScope {
    /// Platform default scope.
    #[default]
    Default,
    /// All private chats.
    AllPrivateChats,
    /// All group chats.
    AllGroupChats,
    /// Administrators in all chats.
    AllChatAdministrators,
    /// One specified chat.
    Chat {
        /// Platform chat identifier.
        chat_id: String,
    },
    /// Administrators in one specified chat.
    ChatAdministrators {
        /// Platform chat identifier.
        chat_id: String,
    },
    /// One specified member in one specified chat.
    ChatMember {
        /// Platform chat identifier.
        chat_id: String,
        /// Platform user identifier.
        user_id: String,
    },
    /// Lossless platform-native command scope.
    PlatformNative(PlatformNativeData),
}

/// Platform chat-menu configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ChatMenu {
    /// Platform default menu.
    Default,
    /// Command list menu.
    Commands,
    /// Web application menu item.
    WebApp {
        /// User-visible menu text.
        text: String,
        /// Web application URL.
        url: String,
    },
    /// Custom action rows menu.
    Actions {
        /// Optional user-visible menu label.
        label: Option<String>,
        /// Action rows shown by the menu.
        rows: Vec<ActionRow>,
    },
    /// Lossless platform-native menu configuration.
    PlatformNative(PlatformNativeData),
}

/// Error returned when an adapter cannot perform an interaction feature.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsupportedInteractionError {
    /// Name of the unavailable interaction feature.
    pub feature: String,
}

impl UnsupportedInteractionError {
    /// Creates an unsupported-interaction error for `feature`.
    pub fn new(feature: impl Into<String>) -> Self {
        Self {
            feature: feature.into(),
        }
    }
}

impl fmt::Display for UnsupportedInteractionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "interaction feature {:?} is not supported by this adapter",
            self.feature
        )
    }
}

impl Error for UnsupportedInteractionError {}
