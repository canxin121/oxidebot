use serde::{Deserialize, Serialize};

/// A platform user and optional profile data.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct User {
    /// Platform-local stable user identifier.
    pub id: String,
    /// Profile details exposed by the platform.
    pub profile: Option<UserProfile>,
}

/// Optional gender value exposed by a platform profile.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Gender {
    /// Male gender.
    Male,
    /// Female gender.
    Female,
    /// A non-binary or platform-defined gender.
    Other,
    /// Gender was not supplied or is unknown.
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

/// Optional profile attributes exposed for a user.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct UserProfile {
    /// User-visible display name.
    pub display_name: Option<String>,
    /// Reported gender.
    pub gender: Option<Gender>,
    /// Reported age.
    pub age: Option<u64>,
    /// Avatar URL or platform identifier.
    pub avatar: Option<String>,
    /// Email address.
    pub email: Option<String>,
    /// Phone number.
    pub phone: Option<String>,
    /// Profile signature or bio.
    pub signature: Option<String>,
    /// Platform-defined level or rank.
    pub level: Option<String>,
}
