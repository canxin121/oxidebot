use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub profile: Option<UserProfile>,
    pub group_info: Option<UserGroupInfo>,
}

#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
pub enum Role {
    Owner,
    Admin,
    #[default]
    Member,
    Guest,
    Unknown,
}

#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
pub struct UserGroupInfo {
    pub alias: Option<String>,
    pub role: Option<Role>,
    pub join_time: Option<DateTime<Utc>>,
    pub last_active_time: Option<DateTime<Utc>>,
    pub level: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Sex {
    Male,
    Female,
    Other,
    #[default]
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

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct UserProfile {
    pub nickname: Option<String>,
    pub sex: Option<Sex>,
    pub age: Option<u64>,
    pub avatar: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub signature: Option<String>,
    pub level: Option<String>,
}
