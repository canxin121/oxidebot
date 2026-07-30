//! Reply-keyboard and platform-native interaction components.
//!
//! Reply-keyboard controls are a distinct surface from message-attached action
//! rows, but share the canonical button style and native-data representations.

use super::*;

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

/// Action initiated by selecting a [`ReplyButton`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ReplyButtonAction {
    /// Send the button text as a message.
    SendText,
    /// Ask the user to select users.
    RequestUsers(RequestUsers),
    /// Ask the user to select a chat.
    RequestChat(RequestChat),
    /// Ask the user to create or select a managed bot.
    RequestManagedBot(RequestManagedBot),
    /// Ask the user to share a contact.
    RequestContact,
    /// Ask the user to share a location.
    RequestLocation,
    /// Ask the user to create a poll.
    RequestPoll {
        /// Optional required poll kind.
        kind: Option<PollKind>,
    },
    /// Open a web application.
    WebApp {
        /// Web application URL.
        url: String,
    },
    /// Lossless platform-native reply action.
    PlatformNative(PlatformNativeData),
}

/// Kind of poll requested by a reply button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PollKind {
    /// Standard multiple-choice poll.
    Regular,
    /// Quiz-style poll with a correct answer.
    Quiz,
}

/// Criteria and returned fields for a user-selection request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestUsers {
    /// Platform request identifier echoed in the response.
    pub request_id: i32,
    /// Optional constraint on whether selected users are bots.
    pub user_is_bot: Option<bool>,
    /// Optional constraint on whether selected users are premium users.
    pub user_is_premium: Option<bool>,
    /// Maximum selected user count.
    pub max_quantity: Option<u8>,
    /// Request selected users' display names.
    pub request_name: bool,
    /// Request selected users' usernames.
    pub request_username: bool,
    /// Request selected users' profile photos.
    pub request_photo: bool,
}

impl RequestUsers {
    /// Creates an unconstrained user-selection request.
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

/// Criteria and returned fields for a chat-selection request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RequestChat {
    /// Platform request identifier echoed in the response.
    pub request_id: i32,
    /// Whether the requested chat must be a channel.
    pub chat_is_channel: bool,
    /// Optional constraint on forum capability.
    pub chat_is_forum: Option<bool>,
    /// Optional constraint on public username availability.
    pub chat_has_username: Option<bool>,
    /// Optional constraint on whether the user created the chat.
    pub chat_is_created: Option<bool>,
    /// Whether the configured bot must already be a member.
    pub bot_is_member: bool,
    /// Request selected chat title.
    pub request_title: bool,
    /// Request selected chat username.
    pub request_username: bool,
    /// Request selected chat photo.
    pub request_photo: bool,
    /// Adapter-specific criteria, for example Telegram administrator rights.
    pub platform_data: Option<PlatformNativeData>,
}

impl RequestChat {
    /// Creates a chat-selection request with its required channel type.
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

/// Criteria for selecting or creating a managed bot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestManagedBot {
    /// Platform request identifier echoed in the response.
    pub request_id: i32,
    /// Optional suggested display name.
    pub suggested_name: Option<String>,
    /// Optional suggested username.
    pub suggested_username: Option<String>,
}

impl RequestManagedBot {
    /// Creates a managed-bot request with no naming suggestions.
    pub fn new(request_id: i32) -> Self {
        Self {
            request_id,
            suggested_name: None,
            suggested_username: None,
        }
    }
}

/// Lossless platform-specific interaction data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformNativeData {
    /// Platform that owns the payload.
    pub platform: String,
    /// JSON payload preserved without portable interpretation.
    pub data: Value,
}

impl PlatformNativeData {
    /// Associates a platform name with a JSON payload.
    pub fn new(platform: impl Into<String>, data: impl Into<Value>) -> Self {
        Self {
            platform: platform.into(),
            data: data.into(),
        }
    }
}
