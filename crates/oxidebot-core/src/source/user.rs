use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub profile: Option<UserProfile>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Gender {
    Male,
    Female,
    Other,
    #[default]
    Unknown,
}

impl From<&str> for Gender {
    fn from(value: &str) -> Self {
        if value == "男" || value.eq_ignore_ascii_case("male") {
            Self::Male
        } else if value == "女" || value.eq_ignore_ascii_case("female") {
            Self::Female
        } else {
            Self::Unknown
        }
    }
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct UserProfile {
    pub display_name: Option<String>,
    pub gender: Option<Gender>,
    pub age: Option<u64>,
    pub avatar: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub signature: Option<String>,
    pub level: Option<String>,
}
