//! Cross-platform interactive message, command, menu, and response types.
//!
//! The model intentionally covers the common interaction lifecycle shared by
//! Telegram, Discord, Slack, Teams, Lark, LINE, QQ, and similar platforms:
//! render components, receive an interaction event, then acknowledge or
//! answer it. Platform-only fields remain representable through
//! [`PlatformNativeData`] instead of being silently discarded.

use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
    time::Duration,
};

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use serde_json::Value;

use crate::{
    application::CommandInvocation,
    content::FormValue,
    conversation::{ConversationPermission, ConversationRef},
    source::{message::Message, user::User},
};

pub use crate::source::message::MessageOptions;

#[path = "interaction_reply.rs"]
mod interaction_reply;

pub use interaction_reply::*;

#[path = "interaction_lifecycle.rs"]
mod interaction_lifecycle;

pub use interaction_lifecycle::*;

#[path = "interaction_command.rs"]
mod command;

pub use command::{
    BotCommand, BotCommandQuery, BotCommandSet, ChatMenu, CommandScope, UnsupportedInteractionError,
};

/// Interactive UI attached to a message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MessageComponents {
    /// Message-attached action rows, such as Telegram inline keyboards or
    /// Discord message components.
    InlineKeyboard(InlineKeyboard),
    /// A client reply keyboard or quick-reply surface.
    ReplyKeyboard(ReplyKeyboard),
    /// Request that a client remove its reply keyboard.
    RemoveReplyKeyboard {
        /// Whether only selected users should see the removal.
        selective: bool,
    },
    /// Request that a client focus a reply input.
    ForceReply {
        /// Optional placeholder displayed by the reply input.
        input_field_placeholder: Option<String>,
        /// Whether only selected users should see the request.
        selective: bool,
    },
    /// A complete platform-native component payload.
    PlatformNative(PlatformNativeData),
}

/// Ordered rows of message-attached interaction components.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InlineKeyboard {
    /// Rows rendered by the platform.
    pub rows: Vec<ActionRow>,
}

impl InlineKeyboard {
    /// Creates a keyboard from ordered action rows.
    pub fn new(rows: impl IntoIterator<Item = ActionRow>) -> Self {
        Self {
            rows: rows.into_iter().collect(),
        }
    }
}

/// One horizontal row of interaction components.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionRow {
    /// Components rendered in row order.
    pub components: Vec<InteractionComponent>,
}

impl ActionRow {
    /// Creates a row from ordered components.
    pub fn new(components: impl IntoIterator<Item = InteractionComponent>) -> Self {
        Self {
            components: components.into_iter().collect(),
        }
    }

    /// Creates a row containing only buttons.
    pub fn buttons(buttons: impl IntoIterator<Item = Button>) -> Self {
        Self::new(buttons.into_iter().map(InteractionComponent::Button))
    }
}

/// One interactive component in an action row or modal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum InteractionComponent {
    /// Clickable button.
    Button(Button),
    /// Select-menu input.
    Select(SelectMenu),
    /// Modal or form input.
    Input(InputComponent),
    /// Lossless platform-native component.
    PlatformNative(PlatformNativeData),
}

impl From<Button> for InteractionComponent {
    fn from(value: Button) -> Self {
        Self::Button(value)
    }
}

impl From<SelectMenu> for InteractionComponent {
    fn from(value: SelectMenu) -> Self {
        Self::Select(value)
    }
}

/// Clickable interaction button.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Button {
    /// User-visible button label.
    pub label: String,
    /// Action performed when clicked.
    pub action: ButtonAction,
    /// Requested visual style.
    pub style: ButtonStyle,
    /// Whether the button cannot be clicked.
    pub disabled: bool,
    /// Optional button icon.
    pub icon: Option<ButtonIcon>,
}

impl Button {
    /// Creates a button with default style and enabled state.
    pub fn new(label: impl Into<String>, action: ButtonAction) -> Self {
        Self {
            label: label.into(),
            action,
            style: ButtonStyle::Default,
            disabled: false,
            icon: None,
        }
    }

    /// Creates a button that sends callback data to the bot.
    pub fn callback(label: impl Into<String>, data: impl Into<String>) -> Self {
        Self::new(label, ButtonAction::Callback { data: data.into() })
    }

    /// Creates a button that asks the client to send text.
    pub fn send_text(label: impl Into<String>, text: impl Into<String>) -> Self {
        Self::new(label, ButtonAction::SendText { text: text.into() })
    }

    /// Creates a button that opens a URL.
    pub fn url(label: impl Into<String>, url: impl Into<String>) -> Self {
        Self::new(label, ButtonAction::Url { url: url.into() })
    }

    /// Creates a button that opens a web app.
    pub fn web_app(label: impl Into<String>, url: impl Into<String>) -> Self {
        Self::new(label, ButtonAction::WebApp { url: url.into() })
    }

    /// Creates a button that starts a platform login flow.
    pub fn login(label: impl Into<String>, login: LoginAction) -> Self {
        Self::new(label, ButtonAction::Login(login))
    }

    /// Creates a button that switches a client into inline-query mode.
    pub fn switch_inline_query(
        label: impl Into<String>,
        query: impl Into<String>,
        target: InlineQueryTarget,
    ) -> Self {
        Self::new(
            label,
            ButtonAction::SwitchInlineQuery {
                query: query.into(),
                target,
            },
        )
    }

    /// Creates a button that copies text in capable clients.
    pub fn copy_text(label: impl Into<String>, text: impl Into<String>) -> Self {
        Self::new(label, ButtonAction::CopyText { text: text.into() })
    }

    /// Creates a button that launches a game.
    pub fn game(label: impl Into<String>) -> Self {
        Self::new(label, ButtonAction::Game)
    }

    /// Creates a button that starts payment.
    pub fn pay(label: impl Into<String>) -> Self {
        Self::new(label, ButtonAction::Pay)
    }

    /// Replaces the requested visual style.
    pub fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }

    /// Enables or disables the button.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Attaches an icon to the button.
    pub fn icon(mut self, icon: ButtonIcon) -> Self {
        self.icon = Some(icon);
        self
    }
}

/// Portable visual emphasis for a button.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ButtonStyle {
    /// Platform default appearance.
    #[default]
    Default,
    /// Primary action emphasis.
    Primary,
    /// Secondary action emphasis.
    Secondary,
    /// Positive or successful action emphasis.
    Success,
    /// Destructive or dangerous action emphasis.
    Danger,
}

/// Icon displayed alongside a button label.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ButtonIcon {
    /// Unicode emoji icon.
    Emoji(String),
    /// A platform-specific custom emoji identifier.
    Custom(String),
}

/// Action initiated by pressing a [`Button`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ButtonAction {
    /// Send opaque callback data to the bot.
    Callback {
        /// Opaque callback payload.
        data: String,
    },
    /// Ask the client to send a text/command message. This is supported by
    /// platforms such as LINE and QQ, but not by Telegram inline keyboards.
    SendText {
        /// Text or command sent by the client.
        text: String,
    },
    /// Open a URL.
    Url {
        /// URL to open.
        url: String,
    },
    /// Open a web application URL.
    WebApp {
        /// Web application URL.
        url: String,
    },
    /// Start a platform login flow.
    Login(LoginAction),
    /// Switch client into inline-query mode.
    SwitchInlineQuery {
        /// Initial inline query text.
        query: String,
        /// Conversation target-selection policy.
        target: InlineQueryTarget,
    },
    /// Copy text in a capable client.
    CopyText {
        /// Text to copy.
        text: String,
    },
    /// Launch a platform game.
    Game,
    /// Start a platform payment flow.
    Pay,
    /// Lossless platform-native button action.
    PlatformNative(PlatformNativeData),
}

/// Parameters for a platform login button action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginAction {
    /// Login URL.
    pub url: String,
    /// Optional text shown when forwarding login data.
    pub forward_text: Option<String>,
    /// Optional bot username selected by the platform.
    pub bot_username: Option<String>,
    /// Whether to request permission to write on behalf of the user.
    pub request_write_access: bool,
}

impl LoginAction {
    /// Creates a login action with optional fields unset.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            forward_text: None,
            bot_username: None,
            request_write_access: false,
        }
    }
}

/// Target-selection behavior for an inline query.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InlineQueryTarget {
    /// Let the user choose an allowed conversation.
    Choose,
    /// Use the current conversation.
    Current,
    /// Restrict the user choice using explicit criteria.
    Selected(ChosenChatCriteria),
}

/// Criteria used when choosing an inline-query conversation target.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChosenChatCriteria {
    /// Whether direct user chats are allowed.
    pub allow_user_chats: Option<bool>,
    /// Whether bot chats are allowed.
    pub allow_bot_chats: Option<bool>,
    /// Whether group chats are allowed.
    pub allow_group_chats: Option<bool>,
    /// Whether channels are allowed.
    pub allow_channel_chats: Option<bool>,
}

/// Platform select-menu component.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelectMenu {
    /// Stable component identifier.
    pub id: String,
    /// Data type selected by the menu.
    pub kind: SelectKind,
    /// Static options, when this select uses text choices.
    pub options: Vec<SelectOption>,
    /// Optional client-visible placeholder.
    pub placeholder: Option<String>,
    /// Minimum number of selections, if constrained.
    pub min_values: Option<u16>,
    /// Maximum number of selections, if constrained.
    pub max_values: Option<u16>,
    /// Whether the menu is disabled.
    pub disabled: bool,
}

/// Data type supplied by a select menu.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectKind {
    /// Static text choices.
    #[default]
    Text,
    /// User selection.
    User,
    /// Role selection.
    Role,
    /// Channel selection.
    Channel,
    /// User-or-role selection.
    Mentionable,
    /// Adapter-provided external selection.
    External,
}

/// One static value offered by a [`SelectMenu`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectOption {
    /// User-visible label.
    pub label: String,
    /// Value returned in the interaction event.
    pub value: String,
    /// Optional secondary description.
    pub description: Option<String>,
    /// Whether the option is selected initially.
    pub default: bool,
    /// Optional icon.
    pub emoji: Option<ButtonIcon>,
    /// Lossless platform-specific option metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// One typed choice offered by a form input.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChoiceOption {
    /// User-visible label.
    pub label: String,
    /// Value returned by the form.
    pub value: FormValue,
    /// Optional secondary description.
    pub description: Option<String>,
    /// Whether this choice is selected initially.
    pub default: bool,
    /// Optional icon.
    pub icon: Option<ButtonIcon>,
    /// Lossless platform-specific choice metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// One platform-neutral form input component.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum InputComponent {
    /// Text input.
    Text {
        /// Stable field identifier.
        id: String,
        /// User-visible field label.
        label: String,
        /// Optional input placeholder.
        placeholder: Option<String>,
        /// Optional initial text value.
        initial_value: Option<String>,
        /// Whether a value is required.
        required: bool,
        /// Whether multiline input is allowed.
        multiline: bool,
        /// Minimum text length, if constrained.
        min_length: Option<u32>,
        /// Maximum text length, if constrained.
        max_length: Option<u32>,
    },
    /// Numeric input.
    Number {
        /// Stable field identifier.
        id: String,
        /// User-visible field label.
        label: String,
        /// Optional input placeholder.
        placeholder: Option<String>,
        /// Optional initial numeric value.
        initial_value: Option<f64>,
        /// Whether a value is required.
        required: bool,
        /// Inclusive minimum value, if constrained.
        min: Option<f64>,
        /// Inclusive maximum value, if constrained.
        max: Option<f64>,
        /// Preferred numeric increment, if supported.
        step: Option<f64>,
    },
    /// Boolean checkbox input.
    Checkbox {
        /// Stable field identifier.
        id: String,
        /// User-visible field label.
        label: String,
        /// Initial checked state.
        initial_value: bool,
        /// Whether acceptance is required.
        required: bool,
    },
    /// Choice input.
    Choice {
        /// Stable field identifier.
        id: String,
        /// User-visible field label.
        label: String,
        /// Available choices.
        options: Vec<ChoiceOption>,
        /// Whether multiple choices may be selected.
        multiple: bool,
        /// Whether a choice is required.
        required: bool,
        /// Minimum choices, if constrained.
        min_values: Option<u16>,
        /// Maximum choices, if constrained.
        max_values: Option<u16>,
    },
    /// Boolean toggle input.
    Toggle {
        /// Stable field identifier.
        id: String,
        /// User-visible field label.
        label: String,
        /// Initial enabled state.
        initial_value: bool,
    },
    /// Date input.
    Date {
        /// Stable field identifier.
        id: String,
        /// User-visible field label.
        label: String,
        /// Optional initial date.
        initial_value: Option<NaiveDate>,
        /// Whether a date is required.
        required: bool,
    },
    /// Time input.
    Time {
        /// Stable field identifier.
        id: String,
        /// User-visible field label.
        label: String,
        /// Optional initial time.
        initial_value: Option<NaiveTime>,
        /// Whether a time is required.
        required: bool,
    },
    /// Date-time input.
    DateTime {
        /// Stable field identifier.
        id: String,
        /// User-visible field label.
        label: String,
        /// Optional initial date-time.
        initial_value: Option<DateTime<Utc>>,
        /// Whether a date-time is required.
        required: bool,
    },
    /// File-upload input.
    File {
        /// Stable field identifier.
        id: String,
        /// User-visible field label.
        label: String,
        /// Whether at least one file is required.
        required: bool,
        /// Minimum uploaded file count, if constrained.
        min_files: Option<u16>,
        /// Maximum uploaded file count, if constrained.
        max_files: Option<u16>,
        /// Accepted MIME types.
        accepted_mime_types: Vec<String>,
        /// Maximum size of one uploaded file in bytes, if constrained.
        max_file_bytes: Option<u64>,
    },
    /// Lossless platform-native input.
    PlatformNative(PlatformNativeData),
}

impl From<InputComponent> for InteractionComponent {
    fn from(value: InputComponent) -> Self {
        Self::Input(value)
    }
}
