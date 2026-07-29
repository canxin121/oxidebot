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

/// Client reply keyboard displayed near a message composer.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReplyKeyboard {
    /// Ordered keyboard rows.
    pub rows: Vec<ReplyButtonRow>,
    /// Whether the client should resize the keyboard.
    pub resize: bool,
    /// Whether the client should hide it after one selection.
    pub one_time: bool,
    /// Whether the keyboard remains visible.
    pub persistent: bool,
    /// Optional composer placeholder.
    pub input_field_placeholder: Option<String>,
    /// Whether only selected users should see it.
    pub selective: bool,
}

impl ReplyKeyboard {
    /// Creates a reply keyboard from ordered rows.
    pub fn new(rows: impl IntoIterator<Item = ReplyButtonRow>) -> Self {
        Self {
            rows: rows.into_iter().collect(),
            ..Default::default()
        }
    }

    /// Enables or disables client keyboard resizing.
    pub fn resize(mut self, resize: bool) -> Self {
        self.resize = resize;
        self
    }

    /// Enables or disables one-time keyboard behavior.
    pub fn one_time(mut self, one_time: bool) -> Self {
        self.one_time = one_time;
        self
    }

    /// Enables or disables persistent keyboard behavior.
    pub fn persistent(mut self, persistent: bool) -> Self {
        self.persistent = persistent;
        self
    }

    /// Sets the composer placeholder.
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.input_field_placeholder = Some(placeholder.into());
        self
    }

    /// Enables or disables selective visibility.
    pub fn selective(mut self, selective: bool) -> Self {
        self.selective = selective;
        self
    }
}

/// One row in a reply keyboard.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReplyButtonRow {
    /// Buttons rendered in row order.
    pub buttons: Vec<ReplyButton>,
}

impl ReplyButtonRow {
    /// Creates a row from ordered reply buttons.
    pub fn new(buttons: impl IntoIterator<Item = ReplyButton>) -> Self {
        Self {
            buttons: buttons.into_iter().collect(),
        }
    }
}

/// Client reply-keyboard button.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReplyButton {
    /// User-visible button label.
    pub label: String,
    /// Action performed on selection.
    pub action: ReplyButtonAction,
    /// Requested visual style.
    pub style: ButtonStyle,
    /// Optional button icon.
    pub icon: Option<ButtonIcon>,
}

impl ReplyButton {
    /// Creates a reply button with default style.
    pub fn new(label: impl Into<String>, action: ReplyButtonAction) -> Self {
        Self {
            label: label.into(),
            action,
            style: ButtonStyle::Default,
            icon: None,
        }
    }

    /// Creates a button that sends its label as text.
    pub fn text(label: impl Into<String>) -> Self {
        Self::new(label, ReplyButtonAction::SendText)
    }

    /// Creates a button that asks the user to select users.
    pub fn request_users(label: impl Into<String>, request: RequestUsers) -> Self {
        Self::new(label, ReplyButtonAction::RequestUsers(request))
    }

    /// Creates a button that asks the user to select a chat.
    pub fn request_chat(label: impl Into<String>, request: RequestChat) -> Self {
        Self::new(label, ReplyButtonAction::RequestChat(request))
    }

    /// Creates a button that asks the user to create or select a managed bot.
    pub fn request_managed_bot(label: impl Into<String>, request: RequestManagedBot) -> Self {
        Self::new(label, ReplyButtonAction::RequestManagedBot(request))
    }

    /// Creates a button that requests a contact.
    pub fn request_contact(label: impl Into<String>) -> Self {
        Self::new(label, ReplyButtonAction::RequestContact)
    }

    /// Creates a button that requests a location.
    pub fn request_location(label: impl Into<String>) -> Self {
        Self::new(label, ReplyButtonAction::RequestLocation)
    }

    /// Creates a button that requests a poll of an optional kind.
    pub fn request_poll(label: impl Into<String>, kind: Option<PollKind>) -> Self {
        Self::new(label, ReplyButtonAction::RequestPoll { kind })
    }

    /// Creates a button that opens a web app.
    pub fn web_app(label: impl Into<String>, url: impl Into<String>) -> Self {
        Self::new(label, ReplyButtonAction::WebApp { url: url.into() })
    }

    /// Replaces the requested visual style.
    pub fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }

    /// Attaches an icon to the button.
    pub fn icon(mut self, icon: ButtonIcon) -> Self {
        self.icon = Some(icon);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ReplyButtonAction {
    SendText,
    RequestUsers(RequestUsers),
    RequestChat(RequestChat),
    RequestManagedBot(RequestManagedBot),
    RequestContact,
    RequestLocation,
    RequestPoll { kind: Option<PollKind> },
    WebApp { url: String },
    PlatformNative(PlatformNativeData),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PollKind {
    Regular,
    Quiz,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestUsers {
    pub request_id: i32,
    pub user_is_bot: Option<bool>,
    pub user_is_premium: Option<bool>,
    pub max_quantity: Option<u8>,
    pub request_name: bool,
    pub request_username: bool,
    pub request_photo: bool,
}

impl RequestUsers {
    pub fn new(request_id: i32) -> Self {
        Self {
            request_id,
            user_is_bot: None,
            user_is_premium: None,
            max_quantity: None,
            request_name: false,
            request_username: false,
            request_photo: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RequestChat {
    pub request_id: i32,
    pub chat_is_channel: bool,
    pub chat_is_forum: Option<bool>,
    pub chat_has_username: Option<bool>,
    pub chat_is_created: Option<bool>,
    pub bot_is_member: bool,
    pub request_title: bool,
    pub request_username: bool,
    pub request_photo: bool,
    /// Adapter-specific criteria, for example Telegram administrator rights.
    pub platform_data: Option<PlatformNativeData>,
}

impl RequestChat {
    pub fn new(request_id: i32, chat_is_channel: bool) -> Self {
        Self {
            request_id,
            chat_is_channel,
            chat_is_forum: None,
            chat_has_username: None,
            chat_is_created: None,
            bot_is_member: false,
            request_title: false,
            request_username: false,
            request_photo: false,
            platform_data: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestManagedBot {
    pub request_id: i32,
    pub suggested_name: Option<String>,
    pub suggested_username: Option<String>,
}

impl RequestManagedBot {
    pub fn new(request_id: i32) -> Self {
        Self {
            request_id,
            suggested_name: None,
            suggested_username: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformNativeData {
    pub platform: String,
    pub data: Value,
}

impl PlatformNativeData {
    pub fn new(platform: impl Into<String>, data: impl Into<Value>) -> Self {
        Self {
            platform: platform.into(),
            data: data.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotCommand {
    pub command: String,
    pub description: String,
    /// Whether responses to this command should be private to the invoking
    /// user on platforms that support ephemeral commands.
    pub is_ephemeral: bool,
}

impl BotCommand {
    pub fn new(command: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            description: description.into(),
            is_ephemeral: false,
        }
    }

    pub fn ephemeral(mut self, is_ephemeral: bool) -> Self {
        self.is_ephemeral = is_ephemeral;
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BotCommandQuery {
    pub scope: CommandScope,
    pub language_code: Option<String>,
}

impl BotCommandQuery {
    pub fn scope(mut self, scope: CommandScope) -> Self {
        self.scope = scope;
        self
    }

    pub fn language_code(mut self, language_code: impl Into<String>) -> Self {
        self.language_code = Some(language_code.into());
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BotCommandSet {
    pub commands: Vec<BotCommand>,
    pub scope: CommandScope,
    pub language_code: Option<String>,
}

impl BotCommandSet {
    pub fn new(commands: impl IntoIterator<Item = BotCommand>) -> Self {
        Self {
            commands: commands.into_iter().collect(),
            scope: CommandScope::Default,
            language_code: None,
        }
    }

    pub fn scope(mut self, scope: CommandScope) -> Self {
        self.scope = scope;
        self
    }

    pub fn language_code(mut self, language_code: impl Into<String>) -> Self {
        self.language_code = Some(language_code.into());
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum CommandScope {
    #[default]
    Default,
    AllPrivateChats,
    AllGroupChats,
    AllChatAdministrators,
    Chat {
        chat_id: String,
    },
    ChatAdministrators {
        chat_id: String,
    },
    ChatMember {
        chat_id: String,
        user_id: String,
    },
    PlatformNative(PlatformNativeData),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ChatMenu {
    Default,
    Commands,
    WebApp {
        text: String,
        url: String,
    },
    Actions {
        label: Option<String>,
        rows: Vec<ActionRow>,
    },
    PlatformNative(PlatformNativeData),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InteractionEvent {
    pub id: String,
    pub kind: InteractionKind,
    pub action_id: Option<String>,
    pub values: Vec<String>,
    pub user: User,
    pub conversation: Option<ConversationRef>,
    pub message: Option<Message>,
    /// An inline-message, view, modal, or other platform context identifier.
    pub context_id: Option<String>,
    /// A response handle is present only when the platform event can be
    /// acknowledged or answered. Message-like selections may intentionally
    /// have no handle.
    pub response: Option<InteractionResponseHandle>,
    /// Values keyed by component or form field identifier.
    pub fields: BTreeMap<String, Vec<FormValue>>,
    pub command: Option<CommandInvocation>,
    pub locale: Option<String>,
    pub permissions: BTreeSet<ConversationPermission>,
    /// Complete platform payload for fields that the common model cannot
    /// represent.
    pub data: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InteractionResponseHandle {
    pub id: String,
    pub deadline: Option<DateTime<Utc>>,
    pub ack_required: bool,
    pub followups_supported: bool,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InteractionKind {
    Button,
    Select,
    Command,
    Form,
    PlatformNative(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum InteractionResponse {
    Acknowledge,
    Defer {
        visibility: InteractionVisibility,
    },
    Notification {
        text: String,
        style: InteractionNotificationStyle,
        cache_time: Option<Duration>,
    },
    OpenUrl {
        url: String,
        cache_time: Option<Duration>,
    },
    Message {
        message: Message,
        visibility: InteractionVisibility,
    },
    UpdateMessage {
        message: Message,
    },
    OpenModal(Modal),
    ValidationErrors(BTreeMap<String, String>),
    Navigate(ViewNavigation),
    CloseView,
    PlatformNative(PlatformNativeData),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ViewNavigation {
    Push(Modal),
    Replace(Modal),
    Pop,
    OpenUrl(String),
    PlatformNative(PlatformNativeData),
}

impl InteractionResponse {
    pub fn toast(text: impl Into<String>) -> Self {
        Self::Notification {
            text: text.into(),
            style: InteractionNotificationStyle::Toast,
            cache_time: None,
        }
    }

    pub fn alert(text: impl Into<String>) -> Self {
        Self::Notification {
            text: text.into(),
            style: InteractionNotificationStyle::Alert,
            cache_time: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum InteractionNotificationStyle {
    #[default]
    Toast,
    Alert,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum InteractionVisibility {
    #[default]
    Public,
    Ephemeral,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Modal {
    pub id: String,
    pub title: String,
    pub fields: Vec<ModalField>,
    pub submit_label: Option<String>,
    pub close_label: Option<String>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ModalField {
    Text {
        id: String,
        label: String,
        placeholder: Option<String>,
        required: bool,
        multiline: bool,
        min_length: Option<u32>,
        max_length: Option<u32>,
    },
    Select(SelectMenu),
    Checkbox {
        id: String,
        label: String,
        value: String,
        required: bool,
    },
    Date {
        id: String,
        label: String,
        required: bool,
    },
    Time {
        id: String,
        label: String,
        required: bool,
    },
    DateTime {
        id: String,
        label: String,
        required: bool,
    },
    Number {
        id: String,
        label: String,
        required: bool,
        min: Option<f64>,
        max: Option<f64>,
    },
    Choice {
        id: String,
        label: String,
        options: Vec<ChoiceOption>,
        multiple: bool,
        required: bool,
    },
    Toggle {
        id: String,
        label: String,
        initial_value: bool,
    },
    File {
        id: String,
        label: String,
        required: bool,
        accepted_mime_types: Vec<String>,
        max_files: Option<u16>,
    },
    PlatformNative(PlatformNativeData),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsupportedInteractionError {
    pub feature: String,
}

impl UnsupportedInteractionError {
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
