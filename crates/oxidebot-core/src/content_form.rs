//! Typed values submitted by command and form controls.

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    conversation::ConversationRef,
    source::{message::File, user::User},
};

/// Typed value submitted by a command, select, or form field.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum FormValue {
    /// Text value.
    Text(String),
    /// Integer value.
    Integer(i64),
    /// Floating-point value.
    Number(f64),
    /// Boolean value.
    Boolean(bool),
    /// Calendar date.
    Date(NaiveDate),
    /// Clock time.
    Time(NaiveTime),
    /// UTC date-time.
    DateTime(DateTime<Utc>),
    /// Selected user.
    User(User),
    /// Selected conversation.
    Conversation(ConversationRef),
    /// Uploaded or selected file.
    File(File),
    /// Lossless JSON value.
    Json(Value),
}
