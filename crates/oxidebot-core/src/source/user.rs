use serde::{Deserialize, Serialize};

use crate::UserId;

/// A platform user and optional profile data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct User {
    /// Platform-local stable user identifier.
    pub id: UserId,
    /// Profile details exposed by the platform.
    pub profile: Option<UserProfile>,
}

impl User {
    /// Creates a platform user with no optional profile data.
    #[must_use]
    pub fn new(id: impl Into<UserId>) -> Self {
        Self {
            id: id.into(),
            profile: None,
        }
    }

    /// Attaches profile data reported by the platform.
    #[must_use]
    pub fn profile(mut self, profile: UserProfile) -> Self {
        self.profile = Some(profile);
        self
    }
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
