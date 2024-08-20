use chrono::{DateTime, Utc};
use hyper::Uri;

use super::{message::Message, user::User};

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Group {
    pub id: String,
    pub profile: Option<GroupProfile>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct GroupProfile {
    pub name: Option<String>,
    pub avatar: Option<Uri>,
    pub member_count: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct GroupAnnouncement {
    pub id: String,
    pub time: DateTime<Utc>,
    pub title: String,
    pub content: String,
    pub author: User,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct GroupHighlight {
    pub id: String,
    pub sender: Option<User>,
    pub setter: Option<User>,
    pub send_time: Option<DateTime<Utc>>,
    pub set_time: Option<DateTime<Utc>>,
    pub title: Option<String>,
    pub message: Option<Message>,
}
