//! Interaction events, acknowledgement responses, and modal navigation.
//!
//! This lifecycle model is shared by all adapters after a component or command
//! has been selected; UI component definitions remain in their own module.

use super::*;

/// Normalized incoming button, command, select, or form interaction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InteractionEvent {
    /// Platform interaction identifier.
    pub id: String,
    /// Interaction category.
    pub kind: InteractionKind,
    /// Clicked action or component identifier, if any.
    pub action_id: Option<String>,
    /// Raw selected values supplied by the platform.
    pub values: Vec<String>,
    /// User that initiated the interaction.
    pub user: User,
    /// Conversation in which it occurred, if any.
    pub conversation: Option<ConversationRef>,
    /// Originating message, if retained by the platform.
    pub message: Option<Message>,
    /// An inline-message, view, modal, or other platform context identifier.
    pub context_id: Option<String>,
    /// A response handle is present only when the platform event can be
    /// acknowledged or answered. Message-like selections may intentionally
    /// have no handle.
    pub response: Option<InteractionResponseHandle>,
    /// Values keyed by component or form field identifier.
    pub fields: BTreeMap<String, Vec<FormValue>>,
    /// Structured command invocation, when this is a command.
    pub command: Option<CommandInvocation>,
    /// User locale, if supplied.
    pub locale: Option<String>,
    /// Permissions known for the user in the conversation.
    pub permissions: BTreeSet<ConversationPermission>,
    /// Complete platform payload for fields that the common model cannot
    /// represent.
    pub data: Value,
}

/// Platform handle used to acknowledge or answer an interaction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InteractionResponseHandle {
    /// Platform response identifier.
    pub id: String,
    /// Platform acknowledgement deadline, if known.
    pub deadline: Option<DateTime<Utc>>,
    /// Whether the platform requires an acknowledgement.
    pub ack_required: bool,
    /// Whether follow-up responses are supported.
    pub followups_supported: bool,
    /// Lossless platform-specific response metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// Category of an incoming interaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InteractionKind {
    /// Button click.
    Button,
    /// Select-menu choice.
    Select,
    /// Platform command invocation.
    Command,
    /// Modal or form submission.
    Form,
    /// Platform-native interaction kind.
    PlatformNative(String),
}

/// Initial response produced for an answerable interaction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum InteractionResponse {
    /// Acknowledge without visible content.
    Acknowledge,
    /// Acknowledge and defer a later response.
    Defer {
        /// Requested visibility for the later response.
        visibility: InteractionVisibility,
    },
    /// Show a transient client notification.
    Notification {
        /// Notification text.
        text: String,
        /// Notification presentation style.
        style: InteractionNotificationStyle,
        /// Optional client cache duration.
        cache_time: Option<Duration>,
    },
    /// Ask the client to open a URL.
    OpenUrl {
        /// URL to open.
        url: String,
        /// Optional client cache duration.
        cache_time: Option<Duration>,
    },
    /// Send an initial interaction message.
    Message {
        /// Portable message content.
        message: Message,
        /// Requested message visibility.
        visibility: InteractionVisibility,
    },
    /// Replace the originating message.
    UpdateMessage {
        /// Replacement portable message.
        message: Message,
    },
    /// Open a modal dialog.
    OpenModal(Modal),
    /// Associate field identifiers with validation messages.
    ValidationErrors(BTreeMap<String, String>),
    /// Navigate within a client view stack.
    Navigate(ViewNavigation),
    /// Close the current client view.
    CloseView,
    /// Lossless platform-native response.
    PlatformNative(PlatformNativeData),
}

/// Navigation action performed by an interaction response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ViewNavigation {
    /// Push a modal onto the view stack.
    Push(Modal),
    /// Replace the current view with a modal.
    Replace(Modal),
    /// Pop the current view.
    Pop,
    /// Open a URL from the view.
    OpenUrl(String),
    /// Lossless platform-native navigation.
    PlatformNative(PlatformNativeData),
}

impl InteractionResponse {
    /// Creates a toast notification response.
    pub fn toast(text: impl Into<String>) -> Self {
        Self::Notification {
            text: text.into(),
            style: InteractionNotificationStyle::Toast,
            cache_time: None,
        }
    }

    /// Creates an alert notification response.
    pub fn alert(text: impl Into<String>) -> Self {
        Self::Notification {
            text: text.into(),
            style: InteractionNotificationStyle::Alert,
            cache_time: None,
        }
    }
}

/// Presentation style for a transient interaction notification.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum InteractionNotificationStyle {
    /// Non-blocking toast notification.
    #[default]
    Toast,
    /// Prominent alert notification.
    Alert,
}

/// Visibility requested for an interaction response.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum InteractionVisibility {
    /// Visible to the normal conversation audience.
    #[default]
    Public,
    /// Visible only to the interacting user where supported.
    Ephemeral,
}

/// Modal dialog opened by an interaction response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Modal {
    /// Stable modal identifier.
    pub id: String,
    /// User-visible modal title.
    pub title: String,
    /// Fields rendered by the modal.
    pub fields: Vec<ModalField>,
    /// Optional submit button label.
    pub submit_label: Option<String>,
    /// Optional close button label.
    pub close_label: Option<String>,
    /// Lossless platform-specific modal metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// One form field rendered inside a [`Modal`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ModalField {
    /// Text field.
    Text {
        /// Stable field identifier.
        id: String,
        /// User-visible field label.
        label: String,
        /// Optional input placeholder.
        placeholder: Option<String>,
        /// Whether a value is required.
        required: bool,
        /// Whether multiline input is allowed.
        multiline: bool,
        /// Minimum text length, if constrained.
        min_length: Option<u32>,
        /// Maximum text length, if constrained.
        max_length: Option<u32>,
    },
    /// Select menu field.
    Select(SelectMenu),
    /// Checkbox field.
    Checkbox {
        /// Stable field identifier.
        id: String,
        /// User-visible field label.
        label: String,
        /// Value returned when checked.
        value: String,
        /// Whether acceptance is required.
        required: bool,
    },
    /// Date field.
    Date {
        /// Stable field identifier.
        id: String,
        /// User-visible field label.
        label: String,
        /// Whether a date is required.
        required: bool,
    },
    /// Time field.
    Time {
        /// Stable field identifier.
        id: String,
        /// User-visible field label.
        label: String,
        /// Whether a time is required.
        required: bool,
    },
    /// Date-time field.
    DateTime {
        /// Stable field identifier.
        id: String,
        /// User-visible field label.
        label: String,
        /// Whether a date-time is required.
        required: bool,
    },
    /// Numeric field.
    Number {
        /// Stable field identifier.
        id: String,
        /// User-visible field label.
        label: String,
        /// Whether a number is required.
        required: bool,
        /// Inclusive minimum, if constrained.
        min: Option<f64>,
        /// Inclusive maximum, if constrained.
        max: Option<f64>,
    },
    /// Choice field.
    Choice {
        /// Stable field identifier.
        id: String,
        /// User-visible field label.
        label: String,
        /// Available choices.
        options: Vec<ChoiceOption>,
        /// Whether multiple values may be selected.
        multiple: bool,
        /// Whether a selection is required.
        required: bool,
    },
    /// Boolean toggle field.
    Toggle {
        /// Stable field identifier.
        id: String,
        /// User-visible field label.
        label: String,
        /// Initial state.
        initial_value: bool,
    },
    /// File-upload field.
    File {
        /// Stable field identifier.
        id: String,
        /// User-visible field label.
        label: String,
        /// Whether a file is required.
        required: bool,
        /// Accepted MIME types.
        accepted_mime_types: Vec<String>,
        /// Maximum accepted file count.
        max_files: Option<u16>,
    },
    /// Lossless platform-native modal field.
    PlatformNative(PlatformNativeData),
}
