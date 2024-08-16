use chrono::{DateTime, Utc};
use hyper::Uri;

#[derive(Clone, Debug, PartialEq, Default)]
pub struct User {
    pub id: String,
    pub profile: Option<UserProfile>,
    pub group_info: Option<UserGroupInfo>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Role {
    Owner,
    Admin,
    Member,
    Guest,
    Unknown,
}

impl Default for Role {
    fn default() -> Self {
        Role::Unknown
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct UserGroupInfo {
    pub title: Option<String>,
    pub role: Option<Role>,
    pub join_time: Option<DateTime<Utc>>,
    pub last_active_time: Option<DateTime<Utc>>,
    pub level: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Sex {
    Male,
    Female,
    Other,
    Unknown,
}

impl From<&str> for Sex {
    fn from(value: &str) -> Self {
        if value == "男" || value.to_lowercase() == "male" {
            Sex::Male
        } else if value == "女" || value.to_lowercase() == "female" {
            Sex::Female
        } else {
            Sex::Unknown
        }
    }
}

impl Default for Sex {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct UserProfile {
    pub nickname: Option<String>,
    pub sex: Option<Sex>,
    pub age: Option<u8>,
    pub avatar: Option<Uri>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub signature: Option<String>,
    pub level: Option<String>,
}
