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
    content::{
        DeliveryTime, FormValue, LinkPreviewOptions, MentionPolicy, MessageVisibility,
        NotificationPolicy, OutgoingMessage, ReplyOptions,
    },
    conversation::ConversationPermission,
    source::{group::Group, message::Message, user::User},
};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MessageOptions {
    pub components: Option<MessageComponents>,
    pub reply: Option<ReplyOptions>,
    pub notification: NotificationPolicy,
    pub visibility: MessageVisibility,
    pub link_preview: Option<LinkPreviewOptions>,
    pub mentions: Option<MentionPolicy>,
    pub protect_content: bool,
    pub delivery_time: DeliveryTime,
    pub idempotency_key: Option<String>,
    pub client_message_id: Option<String>,
    pub metadata: BTreeMap<String, Value>,
    pub platform_data: Option<PlatformNativeData>,
}

impl MessageOptions {
    pub fn components(mut self, components: MessageComponents) -> Self {
        self.components = Some(components);
        self
    }

    pub fn reply(mut self, reply: ReplyOptions) -> Self {
        self.reply = Some(reply);
        self
    }

    pub fn notification(mut self, notification: NotificationPolicy) -> Self {
        self.notification = notification;
        self
    }

    pub fn visibility(mut self, visibility: MessageVisibility) -> Self {
        self.visibility = visibility;
        self
    }

    pub fn link_preview(mut self, link_preview: LinkPreviewOptions) -> Self {
        self.link_preview = Some(link_preview);
        self
    }

    pub fn mentions(mut self, mentions: MentionPolicy) -> Self {
        self.mentions = Some(mentions);
        self
    }

    pub fn protect_content(mut self, protect_content: bool) -> Self {
        self.protect_content = protect_content;
        self
    }

    pub fn delivery_time(mut self, delivery_time: DeliveryTime) -> Self {
        self.delivery_time = delivery_time;
        self
    }

    pub fn is_empty(&self) -> bool {
        self.components.is_none()
            && self.reply.is_none()
            && self.notification == NotificationPolicy::Default
            && self.visibility == MessageVisibility::Public
            && self.link_preview.is_none()
            && self.mentions.is_none()
            && !self.protect_content
            && self.delivery_time == DeliveryTime::Immediate
            && self.idempotency_key.is_none()
            && self.client_message_id.is_none()
            && self.metadata.is_empty()
            && self.platform_data.is_none()
    }
}

/// Interactive UI attached to a message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MessageComponents {
    /// Message-attached action rows, such as Telegram inline keyboards or
    /// Discord message components.
    InlineKeyboard(InlineKeyboard),
    /// A client reply keyboard or quick-reply surface.
    ReplyKeyboard(ReplyKeyboard),
    RemoveReplyKeyboard {
        selective: bool,
    },
    ForceReply {
        input_field_placeholder: Option<String>,
        selective: bool,
    },
    /// A complete platform-native component payload.
    PlatformNative(PlatformNativeData),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InlineKeyboard {
    pub rows: Vec<ActionRow>,
}

impl InlineKeyboard {
    pub fn new(rows: impl IntoIterator<Item = ActionRow>) -> Self {
        Self {
            rows: rows.into_iter().collect(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionRow {
    pub components: Vec<InteractionComponent>,
}

impl ActionRow {
    pub fn new(components: impl IntoIterator<Item = InteractionComponent>) -> Self {
        Self {
            components: components.into_iter().collect(),
        }
    }

    pub fn buttons(buttons: impl IntoIterator<Item = Button>) -> Self {
        Self::new(buttons.into_iter().map(InteractionComponent::Button))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum InteractionComponent {
    Button(Button),
    Select(SelectMenu),
    Input(InputComponent),
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Button {
    pub label: String,
    pub action: ButtonAction,
    pub style: ButtonStyle,
    pub disabled: bool,
    pub icon: Option<ButtonIcon>,
}

impl Button {
    pub fn new(label: impl Into<String>, action: ButtonAction) -> Self {
        Self {
            label: label.into(),
            action,
            style: ButtonStyle::Default,
            disabled: false,
            icon: None,
        }
    }

    pub fn callback(label: impl Into<String>, data: impl Into<String>) -> Self {
        Self::new(label, ButtonAction::Callback { data: data.into() })
    }

    pub fn send_text(label: impl Into<String>, text: impl Into<String>) -> Self {
        Self::new(label, ButtonAction::SendText { text: text.into() })
    }

    pub fn url(label: impl Into<String>, url: impl Into<String>) -> Self {
        Self::new(label, ButtonAction::Url { url: url.into() })
    }

    pub fn web_app(label: impl Into<String>, url: impl Into<String>) -> Self {
        Self::new(label, ButtonAction::WebApp { url: url.into() })
    }

    pub fn login(label: impl Into<String>, login: LoginAction) -> Self {
        Self::new(label, ButtonAction::Login(login))
    }

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

    pub fn copy_text(label: impl Into<String>, text: impl Into<String>) -> Self {
        Self::new(label, ButtonAction::CopyText { text: text.into() })
    }

    pub fn game(label: impl Into<String>) -> Self {
        Self::new(label, ButtonAction::Game)
    }

    pub fn pay(label: impl Into<String>) -> Self {
        Self::new(label, ButtonAction::Pay)
    }

    pub fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn icon(mut self, icon: ButtonIcon) -> Self {
        self.icon = Some(icon);
        self
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ButtonStyle {
    #[default]
    Default,
    Primary,
    Secondary,
    Success,
    Danger,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ButtonIcon {
    Emoji(String),
    /// A platform-specific custom emoji identifier.
    Custom(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ButtonAction {
    Callback {
        data: String,
    },
    /// Ask the client to send a text/command message. This is supported by
    /// platforms such as LINE and QQ, but not by Telegram inline keyboards.
    SendText {
        text: String,
    },
    Url {
        url: String,
    },
    WebApp {
        url: String,
    },
    Login(LoginAction),
    SwitchInlineQuery {
        query: String,
        target: InlineQueryTarget,
    },
    CopyText {
        text: String,
    },
    Game,
    Pay,
    PlatformNative(PlatformNativeData),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginAction {
    pub url: String,
    pub forward_text: Option<String>,
    pub bot_username: Option<String>,
    pub request_write_access: bool,
}

impl LoginAction {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            forward_text: None,
            bot_username: None,
            request_write_access: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InlineQueryTarget {
    ChooseChat,
    CurrentChat,
    ChosenChat(ChosenChatCriteria),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChosenChatCriteria {
    pub allow_user_chats: Option<bool>,
    pub allow_bot_chats: Option<bool>,
    pub allow_group_chats: Option<bool>,
    pub allow_channel_chats: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelectMenu {
    pub id: String,
    pub kind: SelectKind,
    pub options: Vec<SelectOption>,
    pub placeholder: Option<String>,
    pub min_values: Option<u16>,
    pub max_values: Option<u16>,
    pub disabled: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectKind {
    #[default]
    Text,
    User,
    Role,
    Channel,
    Mentionable,
    External,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectOption {
    pub label: String,
    pub value: String,
    pub description: Option<String>,
    pub default: bool,
    pub emoji: Option<ButtonIcon>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChoiceOption {
    pub label: String,
    pub value: FormValue,
    pub description: Option<String>,
    pub default: bool,
    pub icon: Option<ButtonIcon>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum InputComponent {
    Text {
        id: String,
        label: String,
        placeholder: Option<String>,
        initial_value: Option<String>,
        required: bool,
        multiline: bool,
        min_length: Option<u32>,
        max_length: Option<u32>,
    },
    Number {
        id: String,
        label: String,
        placeholder: Option<String>,
        initial_value: Option<f64>,
        required: bool,
        min: Option<f64>,
        max: Option<f64>,
        step: Option<f64>,
    },
    Checkbox {
        id: String,
        label: String,
        initial_value: bool,
        required: bool,
    },
    Choice {
        id: String,
        label: String,
        options: Vec<ChoiceOption>,
        multiple: bool,
        required: bool,
        min_values: Option<u16>,
        max_values: Option<u16>,
    },
    Toggle {
        id: String,
        label: String,
        initial_value: bool,
    },
    Date {
        id: String,
        label: String,
        initial_value: Option<NaiveDate>,
        required: bool,
    },
    Time {
        id: String,
        label: String,
        initial_value: Option<NaiveTime>,
        required: bool,
    },
    DateTime {
        id: String,
        label: String,
        initial_value: Option<DateTime<Utc>>,
        required: bool,
    },
    File {
        id: String,
        label: String,
        required: bool,
        min_files: Option<u16>,
        max_files: Option<u16>,
        accepted_mime_types: Vec<String>,
        max_file_bytes: Option<u64>,
    },
    PlatformNative(PlatformNativeData),
}

impl From<InputComponent> for InteractionComponent {
    fn from(value: InputComponent) -> Self {
        Self::Input(value)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReplyKeyboard {
    pub rows: Vec<ReplyButtonRow>,
    pub resize: bool,
    pub one_time: bool,
    pub persistent: bool,
    pub input_field_placeholder: Option<String>,
    pub selective: bool,
}

impl ReplyKeyboard {
    pub fn new(rows: impl IntoIterator<Item = ReplyButtonRow>) -> Self {
        Self {
            rows: rows.into_iter().collect(),
            ..Default::default()
        }
    }

    pub fn resize(mut self, resize: bool) -> Self {
        self.resize = resize;
        self
    }

    pub fn one_time(mut self, one_time: bool) -> Self {
        self.one_time = one_time;
        self
    }

    pub fn persistent(mut self, persistent: bool) -> Self {
        self.persistent = persistent;
        self
    }

    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.input_field_placeholder = Some(placeholder.into());
        self
    }

    pub fn selective(mut self, selective: bool) -> Self {
        self.selective = selective;
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReplyButtonRow {
    pub buttons: Vec<ReplyButton>,
}

impl ReplyButtonRow {
    pub fn new(buttons: impl IntoIterator<Item = ReplyButton>) -> Self {
        Self {
            buttons: buttons.into_iter().collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReplyButton {
    pub label: String,
    pub action: ReplyButtonAction,
    pub style: ButtonStyle,
    pub icon: Option<ButtonIcon>,
}

impl ReplyButton {
    pub fn new(label: impl Into<String>, action: ReplyButtonAction) -> Self {
        Self {
            label: label.into(),
            action,
            style: ButtonStyle::Default,
            icon: None,
        }
    }

    pub fn text(label: impl Into<String>) -> Self {
        Self::new(label, ReplyButtonAction::SendText)
    }

    pub fn request_users(label: impl Into<String>, request: RequestUsers) -> Self {
        Self::new(label, ReplyButtonAction::RequestUsers(request))
    }

    pub fn request_chat(label: impl Into<String>, request: RequestChat) -> Self {
        Self::new(label, ReplyButtonAction::RequestChat(request))
    }

    pub fn request_managed_bot(label: impl Into<String>, request: RequestManagedBot) -> Self {
        Self::new(label, ReplyButtonAction::RequestManagedBot(request))
    }

    pub fn request_contact(label: impl Into<String>) -> Self {
        Self::new(label, ReplyButtonAction::RequestContact)
    }

    pub fn request_location(label: impl Into<String>) -> Self {
        Self::new(label, ReplyButtonAction::RequestLocation)
    }

    pub fn request_poll(label: impl Into<String>, kind: Option<PollKind>) -> Self {
        Self::new(label, ReplyButtonAction::RequestPoll { kind })
    }

    pub fn web_app(label: impl Into<String>, url: impl Into<String>) -> Self {
        Self::new(label, ReplyButtonAction::WebApp { url: url.into() })
    }

    pub fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }

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
    pub group: Option<Group>,
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

impl InteractionEvent {
    /// Responds through the same bot API object used by event handlers.
    pub async fn respond(
        &self,
        bot: crate::bot::BotObject,
        response: InteractionResponse,
    ) -> anyhow::Result<()> {
        let handle = self.response.as_ref().ok_or_else(|| {
            UnsupportedInteractionError::new("responding to this non-answerable interaction")
        })?;
        match response {
            InteractionResponse::Defer { visibility } => {
                bot.defer_interaction(handle.clone(), visibility).await
            }
            InteractionResponse::Message {
                mut message,
                visibility,
            } => {
                message.options.visibility = match visibility {
                    InteractionVisibility::Public => MessageVisibility::Public,
                    InteractionVisibility::Ephemeral => MessageVisibility::Ephemeral,
                };
                if handle.ack_required {
                    bot.defer_interaction(handle.clone(), visibility).await?;
                }
                bot.send_interaction_followup(handle.clone(), message)
                    .await
                    .map(|_| ())
            }
            InteractionResponse::UpdateMessage { message } => {
                bot.edit_interaction_response(handle.clone(), message).await
            }
            response => bot.answer_interaction(handle.id.clone(), response).await,
        }
    }
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
        message: OutgoingMessage,
        visibility: InteractionVisibility,
    },
    UpdateMessage {
        message: OutgoingMessage,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InteractionCapabilities {
    pub inline_buttons: bool,
    pub reply_keyboard: bool,
    pub select_menus: bool,
    pub modals: bool,
    pub commands: bool,
    pub chat_menu: bool,
    pub web_apps: bool,
    pub update_components: bool,
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
