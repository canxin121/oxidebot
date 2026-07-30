//! Immutable command-tree definitions and builder validation.
//!
//! This module owns localized schema metadata, branches, command construction,
//! native publication definitions, and command-tree invariants. Parsing,
//! rendering, catalog lookup, and asynchronous value resolution stay separate.

use super::*;

/// Localized text used by command schemas, diagnostics, help, completion, and
/// native command publication.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LocalizedText {
    default: Arc<str>,
    translations: BTreeMap<Arc<str>, Arc<str>>,
}

impl LocalizedText {
    /// Creates text with a default locale value.
    #[must_use]
    pub fn new(default: impl Into<Arc<str>>) -> Self {
        Self {
            default: default.into(),
            translations: BTreeMap::new(),
        }
    }

    /// Adds or replaces a translation for `locale`.
    #[must_use]
    pub fn translation(mut self, locale: impl Into<Arc<str>>, value: impl Into<Arc<str>>) -> Self {
        self.translations.insert(locale.into(), value.into());
        self
    }

    /// Resolves exact locale, language fallback, then the default text.
    #[must_use]
    pub fn resolve(&self, locale: Option<&str>) -> &str {
        let Some(locale) = locale else {
            return &self.default;
        };
        self.translations
            .get(locale)
            .or_else(|| {
                locale
                    .split_once('-')
                    .and_then(|(language, _)| self.translations.get(language))
            })
            .map_or(self.default.as_ref(), Arc::as_ref)
    }

    /// Returns whether every localized value is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.default.is_empty() && self.translations.values().all(|value| value.is_empty())
    }

    /// Converts this runtime value to the portable core localization model.
    #[must_use]
    pub fn to_core(&self) -> Localized<String> {
        Localized {
            default: self.default.to_string(),
            translations: self
                .translations
                .iter()
                .map(|(locale, value)| (locale.to_string(), value.to_string()))
                .collect(),
        }
    }
}

impl<T> From<T> for LocalizedText
where
    T: Into<Arc<str>>,
{
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

/// Portable type information shared by text parsing, help, completion, and
/// platform-native command publication.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum CommandValueKind {
    /// Arbitrary UTF-8 text.
    #[default]
    String,
    /// Signed integer value.
    Integer,
    /// Floating-point number.
    Number,
    /// Boolean value.
    Boolean,
    /// Platform user reference.
    User,
    /// Platform conversation reference.
    Conversation,
    /// Platform role reference.
    Role,
    /// A platform-mentionable entity.
    Mentionable,
    /// File attachment reference.
    Attachment,
    /// Canonical rich message segment.
    MessageSegment,
    /// Adapter-defined native value kind.
    PlatformNative(Arc<str>),
}

impl CommandValueKind {
    pub(in crate::command) fn native_kind(&self) -> CommandOptionType {
        match self {
            Self::String | Self::MessageSegment => CommandOptionType::String,
            Self::Integer => CommandOptionType::Integer,
            Self::Number => CommandOptionType::Number,
            Self::Boolean => CommandOptionType::Boolean,
            Self::User => CommandOptionType::User,
            Self::Conversation => CommandOptionType::Conversation,
            Self::Role => CommandOptionType::Role,
            Self::Mentionable => CommandOptionType::Mentionable,
            Self::Attachment => CommandOptionType::Attachment,
            Self::PlatformNative(kind) => CommandOptionType::PlatformNative(kind.to_string()),
        }
    }
}

/// How repeated appearances of an option affect its parsed value.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ArgumentAction {
    /// Store the last supplied value.
    #[default]
    Store,
    /// Append each supplied value.
    Append,
    /// Count repeated option occurrences.
    Count,
    /// Set the flag value to true when present.
    SetTrue,
    /// Set the flag value to false when present.
    SetFalse,
}

/// One choice attached to an argument schema.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArgumentChoice {
    /// Localized label shown to users.
    pub name: LocalizedText,
    /// Canonical string value supplied to parsing.
    pub value: Arc<str>,
    /// Additional parseable spellings that normalize to [`Self::value`].
    pub aliases: Vec<Arc<str>>,
}

impl ArgumentChoice {
    /// Creates a choice with localized display name and canonical value.
    #[must_use]
    pub fn new(name: impl Into<LocalizedText>, value: impl Into<Arc<str>>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            aliases: Vec::new(),
        }
    }

    /// Adds an alternative spelling accepted by text parsing.
    #[must_use]
    pub fn alias(mut self, value: impl Into<Arc<str>>) -> Self {
        self.aliases.push(value.into());
        self
    }

    pub(in crate::command) fn matches(&self, value: &str) -> bool {
        self.value.as_ref() == value || self.aliases.iter().any(|alias| alias.as_ref() == value)
    }
}

/// Strongly typed finite command value.
///
/// Derive this trait with `#[derive(oxidebot::CommandEnum)]` to keep text
/// parsing, help, completion, and platform-native choices synchronized.
pub trait CommandEnum: Sized + Send + 'static {
    /// Returns every canonical choice and its accepted aliases.
    fn choices() -> Vec<ArgumentChoice>;
}

/// Cross-field constraint for a named set of arguments.
///
/// Groups model relations that cannot be expressed as pairwise
/// [`ArgumentSpec::requires`] or [`ArgumentSpec::conflicts_with`] rules, such
/// as requiring exactly one credential source.
#[derive(Clone, Debug)]
pub struct ArgumentGroup {
    pub(in crate::command) name: Arc<str>,
    pub(in crate::command) arguments: Vec<Arc<str>>,
    pub(in crate::command) required: bool,
    pub(in crate::command) multiple: bool,
}

impl ArgumentGroup {
    /// Creates an optional group that permits more than one member.
    #[must_use]
    pub fn new(name: impl Into<Arc<str>>) -> Self {
        Self {
            name: name.into(),
            arguments: Vec::new(),
            required: false,
            multiple: true,
        }
    }

    /// Adds a field name to this group.
    #[must_use]
    pub fn argument(mut self, name: impl Into<Arc<str>>) -> Self {
        self.arguments.push(name.into());
        self
    }

    /// Adds several field names to this group.
    #[must_use]
    pub fn arguments<I, T>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<Arc<str>>,
    {
        self.arguments.extend(names.into_iter().map(Into::into));
        self
    }

    /// Requires at least one group member to be present.
    #[must_use]
    pub const fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    /// Allows more than one group member to be present.
    #[must_use]
    pub const fn multiple(mut self, multiple: bool) -> Self {
        self.multiple = multiple;
        self
    }

    /// Requires exactly one group member to be present.
    #[must_use]
    pub const fn exactly_one(mut self) -> Self {
        self.required = true;
        self.multiple = false;
        self
    }

    /// Returns the stable group name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the field names constrained by this group.
    #[must_use]
    pub fn arguments_ref(&self) -> &[Arc<str>] {
        &self.arguments
    }
}
