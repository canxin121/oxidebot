//! Lossless command input tokens and typed value conversion.

use super::*;

/// One lossless command token. Non-text message segments and native form
/// values remain typed until a field requests conversion.
// `MessageSegment` remains inline to preserve the ergonomic public value API.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum CommandValue {
    /// Text token from command input.
    Text(String),
    /// Mention token retaining its user ID.
    Mention(oxidebot_core::UserId),
    /// File token retaining its portable descriptor.
    File(File),
    /// Rich message segment supplied as a command token.
    Segment(MessageSegment),
    /// Typed value supplied by a native platform invocation.
    Form(FormValue),
}

impl CommandValue {
    /// Borrows text when this value is textual.
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(value) | Self::Form(FormValue::Text(value)) => Some(value),
            Self::Mention(_) | Self::File(_) | Self::Segment(_) | Self::Form(_) => None,
        }
    }

    /// Converts this value to a canonical rich message segment.
    #[must_use]
    pub fn into_segment(self) -> MessageSegment {
        match self {
            Self::Text(value) | Self::Form(FormValue::Text(value)) => MessageSegment::text(value),
            Self::Mention(user_id) => MessageSegment::at(user_id),
            Self::File(file) | Self::Form(FormValue::File(file)) => MessageSegment::file(file),
            Self::Form(FormValue::User(user)) => MessageSegment::at(user.id),
            Self::Form(value) => MessageSegment::text(form_value_label(&value)),
            Self::Segment(segment) => segment,
        }
    }

    pub(in crate::command) fn scalar_text(self) -> Result<String, CommandParseError> {
        match self {
            Self::Text(value) | Self::Form(FormValue::Text(value)) => Ok(value),
            Self::Form(FormValue::Integer(value)) => Ok(value.to_string()),
            Self::Form(FormValue::Number(value)) => Ok(value.to_string()),
            Self::Form(FormValue::Boolean(value)) => Ok(value.to_string()),
            Self::Form(FormValue::Date(value)) => Ok(value.to_string()),
            Self::Form(FormValue::Time(value)) => Ok(value.to_string()),
            Self::Form(FormValue::DateTime(value)) => Ok(value.to_rfc3339()),
            other => Err(CommandParseError::UnexpectedValue {
                expected: "scalar text",
                actual: value_kind(&other),
            }),
        }
    }
}

pub(in crate::command) fn form_value_label(value: &FormValue) -> String {
    match value {
        FormValue::Text(value) => value.clone(),
        FormValue::Integer(value) => value.to_string(),
        FormValue::Number(value) => value.to_string(),
        FormValue::Boolean(value) => value.to_string(),
        FormValue::Date(value) => value.to_string(),
        FormValue::Time(value) => value.to_string(),
        FormValue::DateTime(value) => value.to_rfc3339(),
        FormValue::User(value) => format!("@{}", value.id),
        FormValue::Conversation(value) => value.id.to_string(),
        FormValue::File(value) => {
            if value.name.is_empty() {
                "[file]".to_owned()
            } else {
                value.name.clone()
            }
        }
        FormValue::Json(value) => value.to_string(),
    }
}

/// Converts one typed command token into a field value.
pub trait FromCommandValue: Sized {
    /// Converts one lossless command value into this typed argument value.
    fn from_command_value(value: CommandValue) -> Result<Self, CommandParseError>;
}

impl FromCommandValue for CommandValue {
    fn from_command_value(value: CommandValue) -> Result<Self, CommandParseError> {
        Ok(value)
    }
}

impl FromCommandValue for MessageSegment {
    fn from_command_value(value: CommandValue) -> Result<Self, CommandParseError> {
        Ok(value.into_segment())
    }
}

impl FromCommandValue for FormValue {
    fn from_command_value(value: CommandValue) -> Result<Self, CommandParseError> {
        match value {
            CommandValue::Form(value) => Ok(value),
            CommandValue::Text(value) => Ok(Self::Text(value)),
            CommandValue::Mention(user_id) => Ok(Self::Text(user_id.to_string())),
            CommandValue::File(file) => Ok(Self::File(file)),
            CommandValue::Segment(segment) => Ok(Self::Text(format!("{segment:?}"))),
        }
    }
}

impl FromCommandValue for String {
    fn from_command_value(value: CommandValue) -> Result<Self, CommandParseError> {
        match value {
            CommandValue::Text(value) | CommandValue::Form(FormValue::Text(value)) => Ok(value),
            other => Err(CommandParseError::UnexpectedValue {
                expected: "text",
                actual: value_kind(&other),
            }),
        }
    }
}

impl FromCommandValue for File {
    fn from_command_value(value: CommandValue) -> Result<Self, CommandParseError> {
        match value {
            CommandValue::File(file) | CommandValue::Form(FormValue::File(file)) => Ok(file),
            CommandValue::Segment(MessageSegment::Media { media, .. }) => Ok(media.file),
            other => Err(CommandParseError::UnexpectedValue {
                expected: "file",
                actual: value_kind(&other),
            }),
        }
    }
}

/// A typed `@user` command argument.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Mention(pub oxidebot_core::UserId);

impl FromCommandValue for Mention {
    fn from_command_value(value: CommandValue) -> Result<Self, CommandParseError> {
        match value {
            CommandValue::Mention(user_id) => Ok(Self(user_id)),
            CommandValue::Segment(MessageSegment::At { user_id }) => Ok(Self(user_id)),
            CommandValue::Form(FormValue::User(user)) => Ok(Self(user.id)),
            other => Err(CommandParseError::UnexpectedValue {
                expected: "mention",
                actual: value_kind(&other),
            }),
        }
    }
}

impl FromCommandValue for User {
    fn from_command_value(value: CommandValue) -> Result<Self, CommandParseError> {
        match value {
            CommandValue::Form(FormValue::User(user)) => Ok(user),
            other => Err(CommandParseError::UnexpectedValue {
                expected: "user",
                actual: value_kind(&other),
            }),
        }
    }
}

impl FromCommandValue for ConversationRef {
    fn from_command_value(value: CommandValue) -> Result<Self, CommandParseError> {
        match value {
            CommandValue::Form(FormValue::Conversation(conversation)) => Ok(conversation),
            other => Err(CommandParseError::UnexpectedValue {
                expected: "conversation",
                actual: value_kind(&other),
            }),
        }
    }
}

impl FromCommandValue for serde_json::Value {
    fn from_command_value(value: CommandValue) -> Result<Self, CommandParseError> {
        match value {
            CommandValue::Form(FormValue::Json(value)) => Ok(value),
            other => Err(CommandParseError::UnexpectedValue {
                expected: "JSON",
                actual: value_kind(&other),
            }),
        }
    }
}

macro_rules! from_scalar {
    ($($type:ty),+ $(,)?) => {$ (
        impl FromCommandValue for $type {
            fn from_command_value(value: CommandValue) -> Result<Self, CommandParseError> {
                let text = value.scalar_text()?;
                text.parse::<$type>().map_err(|error| CommandParseError::InvalidValue {
                    value: text,
                    expected: std::any::type_name::<$type>(),
                    reason: error.to_string(),
                })
            }
        }
    )+};
}

from_scalar!(bool, char, i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64);

pub(in crate::command) fn value_kind(value: &CommandValue) -> &'static str {
    match value {
        CommandValue::Text(_) => "text",
        CommandValue::Mention(_) => "mention",
        CommandValue::File(_) => "file",
        CommandValue::Segment(_) => "message segment",
        CommandValue::Form(FormValue::Text(_)) => "native text",
        CommandValue::Form(FormValue::Integer(_)) => "native integer",
        CommandValue::Form(FormValue::Number(_)) => "native number",
        CommandValue::Form(FormValue::Boolean(_)) => "native boolean",
        CommandValue::Form(FormValue::Date(_)) => "native date",
        CommandValue::Form(FormValue::Time(_)) => "native time",
        CommandValue::Form(FormValue::DateTime(_)) => "native date-time",
        CommandValue::Form(FormValue::User(_)) => "native user",
        CommandValue::Form(FormValue::Conversation(_)) => "native conversation",
        CommandValue::Form(FormValue::File(_)) => "native file",
        CommandValue::Form(FormValue::Json(_)) => "native JSON",
    }
}
