use crate::{handler::RouteScope, Shortcut};
use oxidebot_core::{
    application::{
        CommandChoice, CommandContext, CommandDefinition, CommandInvocation, CommandKind,
        CommandOption, CommandOptionType, Localized, Suggestion, SuggestionRequest,
    },
    conversation::{ConversationKind, ConversationRef},
    source::{
        message::{File, Message, MessageSegment},
        user::User,
    },
    BotIdentity, FormValue, TemplateValue, TranslationCatalog,
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt,
    sync::Arc,
    time::Duration,
};
use thiserror::Error;

/// A deterministic identifier for one command tree.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CommandId(pub u64);

/// A deterministic identifier for one node inside a command tree.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CommandNodeId(pub u32);

/// A schema-local identifier for one command argument.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CommandFieldId(pub u32);

/// Compile-time marker generated for one field in a `CommandArgs` schema.
///
/// Field markers keep completion and async resolution next to the feature that
/// owns the command without exposing stringly typed field names in application
/// code. The stable runtime field ID is still resolved from the canonical
/// command tree, so help, parsing, completion, and platform commands share one
/// source of truth.
pub trait CommandFieldTag: Clone + Copy + Send + Sync + 'static {
    /// Generated field name as declared in the command schema.
    const NAME: &'static str;
}

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
    fn native_kind(&self) -> CommandOptionType {
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
}

impl ArgumentChoice {
    /// Creates a choice with localized display name and canonical value.
    #[must_use]
    pub fn new(name: impl Into<LocalizedText>, value: impl Into<Arc<str>>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

/// A true node in the command grammar. Unlike a Web route mount, this models
/// user-visible subcommand syntax and is reused by parsing, help, completion,
/// and native command publication.
#[derive(Clone, Debug)]
pub struct CommandBranch {
    id: CommandNodeId,
    name: Arc<str>,
    description: LocalizedText,
    aliases: Vec<Arc<str>>,
    schema: Option<CommandSchema>,
    children: Vec<CommandBranch>,
    completion: Option<CompletionConfig>,
    hidden: bool,
}

impl CommandBranch {
    /// Creates a subcommand grammar node named `name`.
    #[must_use]
    pub fn new(name: impl Into<Arc<str>>) -> Self {
        let name = name.into();
        Self {
            id: CommandNodeId(stable_hash(name.as_bytes()) as u32),
            name,
            description: LocalizedText::default(),
            aliases: Vec::new(),
            schema: None,
            children: Vec::new(),
            completion: None,
            hidden: false,
        }
    }

    /// Sets the default localized branch description.
    #[must_use]
    pub fn description(mut self, description: impl Into<LocalizedText>) -> Self {
        self.description = description.into();
        self
    }

    /// Adds a locale-specific branch description.
    #[must_use]
    pub fn description_translation(
        mut self,
        locale: impl Into<Arc<str>>,
        description: impl Into<Arc<str>>,
    ) -> Self {
        self.description = self.description.translation(locale, description);
        self
    }

    /// Adds an alternative branch name for parsing.
    #[must_use]
    pub fn alias(mut self, alias: impl Into<Arc<str>>) -> Self {
        self.aliases.push(alias.into());
        self
    }

    /// Attaches an argument schema to this branch.
    #[must_use]
    pub fn schema(mut self, mut schema: CommandSchema) -> Self {
        schema.rebase(&self.name);
        self.schema = Some(schema);
        self
    }

    /// Attaches the schema generated by `CommandArgs`.
    #[must_use]
    pub fn args<T: CommandArgs>(self) -> Self {
        self.schema(T::schema())
    }

    /// Adds a nested user-visible subcommand branch.
    #[must_use]
    pub fn subcommand(mut self, mut branch: CommandBranch) -> Self {
        branch.rebase(&format!("{}.{}", self.name, branch.name));
        self.children.push(branch);
        self
    }

    /// Enables interactive recovery for missing required arguments on this branch.
    #[must_use]
    pub fn completion(mut self, completion: CompletionConfig) -> Self {
        self.completion = Some(completion);
        self
    }

    /// Hides this branch from help and completion while retaining parse support.
    #[must_use]
    pub const fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }

    /// Returns this branch's stable grammar-node ID.
    #[must_use]
    pub const fn id(&self) -> CommandNodeId {
        self.id
    }

    /// Returns the canonical branch name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns direct nested subcommand branches.
    #[must_use]
    pub fn children(&self) -> &[CommandBranch] {
        &self.children
    }

    /// Returns this branch's argument schema, if it accepts arguments.
    #[must_use]
    pub fn schema_ref(&self) -> Option<&CommandSchema> {
        self.schema.as_ref()
    }

    /// Resolves this branch description for the requested locale.
    #[must_use]
    pub fn localized_description(&self, locale: Option<&str>) -> &str {
        self.description.resolve(locale)
    }

    /// Returns alternative branch names accepted by the parser.
    #[must_use]
    pub fn aliases_list(&self) -> &[Arc<str>] {
        &self.aliases
    }

    /// Returns this branch's interactive recovery configuration, if set.
    #[must_use]
    pub fn completion_ref(&self) -> Option<&CompletionConfig> {
        self.completion.as_ref()
    }

    /// Returns whether this branch is hidden from discovery surfaces.
    #[must_use]
    pub const fn is_hidden(&self) -> bool {
        self.hidden
    }

    fn native_option(&self) -> CommandOption {
        let kind = if self.children.is_empty() {
            CommandOptionType::Subcommand
        } else {
            CommandOptionType::SubcommandGroup
        };
        let mut options = self
            .schema
            .as_ref()
            .map_or_else(Vec::new, CommandSchema::native_options);
        options.extend(self.children.iter().map(CommandBranch::native_option));
        CommandOption {
            id: Some(format!("oxidebot-node:{}", self.id.0)),
            name: Localized::new(self.name.replace('_', "-")),
            description: if self.description.is_empty() {
                Localized::new(self.name.to_string())
            } else {
                self.description.to_core()
            },
            kind,
            required: false,
            choices: Vec::new(),
            options,
            autocomplete: false,
            min_value: None,
            max_value: None,
            min_length: None,
            max_length: None,
            allowed_conversation_kinds: Vec::new(),
            platform_data: None,
        }
    }

    fn rebase(&mut self, path: &str) {
        self.id = CommandNodeId(stable_hash(path.as_bytes()) as u32);
        if let Some(schema) = &mut self.schema {
            schema.rebase(path);
        }
        for child in &mut self.children {
            child.rebase(&format!("{path}.{}", child.name));
        }
    }

    fn matches(&self, value: &str, case_sensitive: bool) -> bool {
        same_word(value, &self.name, case_sensitive)
            || self
                .aliases
                .iter()
                .any(|alias| same_word(value, alias, case_sensitive))
    }

    fn validate(&self, path: &str) -> Result<(), String> {
        if self.name.trim().is_empty()
            || self.name.trim() != self.name.as_ref()
            || self.name.chars().any(char::is_whitespace)
        {
            return Err(format!("invalid subcommand name `{path}`"));
        }
        let mut names = HashSet::new();
        for name in std::iter::once(&self.name).chain(self.aliases.iter()) {
            if name.trim().is_empty()
                || name.trim() != name.as_ref()
                || name.chars().any(char::is_whitespace)
            {
                return Err(format!(
                    "invalid subcommand name or alias `{name}` in `{path}`"
                ));
            }
            if !names.insert(name.to_ascii_lowercase()) {
                return Err(format!("duplicate subcommand alias in `{path}`"));
            }
        }
        if let Some(schema) = &self.schema {
            schema.validate()?;
        }
        if self.schema.is_some() && !self.children.is_empty() {
            return Err(format!(
                "subcommand `{path}` cannot contain both arguments and nested subcommands"
            ));
        }
        if let Some(completion) = &self.completion {
            if completion.timeout.is_zero() || completion.max_rounds == 0 {
                return Err(format!(
                    "subcommand `{path}` has an invalid completion policy"
                ));
            }
        }
        let mut child_names = HashSet::new();
        for child in &self.children {
            for name in std::iter::once(&child.name).chain(child.aliases.iter()) {
                if !child_names.insert(name.to_ascii_lowercase()) {
                    return Err(format!(
                        "subcommand `{path}` contains duplicate child name or alias `{name}`"
                    ));
                }
            }
            child.validate(&format!("{path} {}", child.name))?;
        }
        Ok(())
    }
}

fn stable_hash(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Declares one user-facing command.
#[derive(Clone, Debug)]
pub struct Command {
    id: CommandId,
    name: Arc<str>,
    description: LocalizedText,
    aliases: Vec<Arc<str>>,
    prefixes: Vec<Arc<str>>,
    category: Option<Arc<str>>,
    examples: Vec<Arc<str>>,
    case_sensitive: bool,
    hidden: bool,
    schema: Option<CommandSchema>,
    branches: Vec<CommandBranch>,
    completion: Option<CompletionConfig>,
    shortcuts: Vec<Shortcut>,
}

/// Creates a command using `/` as its prefix.
#[must_use]
pub fn command(name: impl Into<Arc<str>>) -> Command {
    Command::new(name)
}

impl Command {
    #[must_use]
    pub fn new(name: impl Into<Arc<str>>) -> Self {
        let name = name.into();
        Self {
            id: CommandId(stable_hash(name.as_bytes())),
            name,
            description: LocalizedText::default(),
            aliases: Vec::new(),
            prefixes: vec![Arc::from("/")],
            category: None,
            examples: Vec::new(),
            case_sensitive: true,
            hidden: false,
            schema: None,
            branches: Vec::new(),
            completion: None,
            shortcuts: Vec::new(),
        }
    }

    #[must_use]
    pub fn description(mut self, description: impl Into<LocalizedText>) -> Self {
        self.description = description.into();
        self
    }

    #[must_use]
    pub fn description_translation(
        mut self,
        locale: impl Into<Arc<str>>,
        description: impl Into<Arc<str>>,
    ) -> Self {
        self.description = self.description.translation(locale, description);
        self
    }

    #[must_use]
    pub fn alias(mut self, alias: impl Into<Arc<str>>) -> Self {
        self.aliases.push(alias.into());
        self
    }

    #[must_use]
    pub fn aliases<I, T>(mut self, aliases: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<Arc<str>>,
    {
        self.aliases.extend(aliases.into_iter().map(Into::into));
        self
    }

    /// Replaces the accepted prefixes. An empty string enables natural-language
    /// commands without a prefix.
    #[must_use]
    pub fn prefixes<I, T>(mut self, prefixes: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<Arc<str>>,
    {
        self.prefixes = prefixes.into_iter().map(Into::into).collect();
        self
    }

    #[must_use]
    pub fn prefix(self, prefix: impl Into<Arc<str>>) -> Self {
        self.prefixes([prefix])
    }

    #[must_use]
    pub fn no_prefix(self) -> Self {
        self.prefix(Arc::<str>::from(""))
    }

    #[must_use]
    pub const fn case_insensitive(mut self) -> Self {
        self.case_sensitive = false;
        self
    }

    #[must_use]
    pub const fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }

    #[must_use]
    pub fn category(mut self, category: impl Into<Arc<str>>) -> Self {
        self.category = Some(category.into());
        self
    }

    #[must_use]
    pub fn example(mut self, example: impl Into<Arc<str>>) -> Self {
        self.examples.push(example.into());
        self
    }

    #[must_use]
    pub fn schema(mut self, mut schema: CommandSchema) -> Self {
        schema.rebase(&self.name);
        self.schema = Some(schema);
        self
    }

    #[must_use]
    pub fn args<T>(self) -> Self
    where
        T: CommandArgs,
    {
        self.schema(T::schema())
    }

    #[must_use]
    pub fn completion(mut self, completion: CompletionConfig) -> Self {
        self.completion = Some(completion);
        self
    }

    #[must_use]
    pub fn shortcut(mut self, shortcut: Shortcut) -> Self {
        self.shortcuts.push(shortcut);
        self
    }

    #[must_use]
    pub fn shortcuts(&self) -> &[Shortcut] {
        &self.shortcuts
    }

    #[must_use]
    pub fn subcommand(mut self, mut branch: CommandBranch) -> Self {
        branch.rebase(&format!("{}.{}", self.name, branch.name));
        self.branches.push(branch);
        self
    }

    #[must_use]
    pub fn subcommand_args<T>(self, name: impl Into<Arc<str>>) -> Self
    where
        T: CommandArgs,
    {
        self.subcommand(CommandBranch::new(name).schema(T::schema()))
    }

    #[must_use]
    pub const fn id(&self) -> CommandId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn description_text(&self) -> &str {
        self.description.resolve(None)
    }

    #[must_use]
    pub fn localized_description(&self, locale: Option<&str>) -> &str {
        self.description.resolve(locale)
    }

    #[must_use]
    pub fn aliases_list(&self) -> &[Arc<str>] {
        &self.aliases
    }

    #[must_use]
    pub fn prefixes_list(&self) -> &[Arc<str>] {
        &self.prefixes
    }

    #[must_use]
    pub fn schema_ref(&self) -> Option<&CommandSchema> {
        self.schema.as_ref()
    }

    #[must_use]
    pub fn branches(&self) -> &[CommandBranch] {
        &self.branches
    }

    #[must_use]
    pub fn completion_ref(&self) -> Option<&CompletionConfig> {
        self.completion.as_ref()
    }

    /// Resolves a deterministic field ID from the same command tree used by
    /// parsing, help, completion, and platform-native publication.
    #[must_use]
    pub fn field_id(&self, branch: &[&str], field: &str) -> Option<CommandFieldId> {
        let branch_names = branch
            .iter()
            .map(|name| Arc::<str>::from(*name))
            .collect::<Vec<_>>();
        self.schema_for_branch_names(&branch_names)
            .find(field)
            .map(ArgumentSpec::id)
    }

    #[must_use]
    pub fn is_hidden(&self) -> bool {
        self.hidden
    }

    #[must_use]
    pub fn display_name(&self) -> String {
        let prefix = self.prefixes.first().map_or("", AsRef::as_ref);
        format!("{prefix}{}", self.name)
    }

    /// Stable-in-process structural fingerprint used to ensure one public
    /// command ID never refers to different parser grammars in the shared
    /// registry. Presentation-only metadata is intentionally excluded.
    pub(crate) fn structural_fingerprint(&self) -> u64 {
        let structure = format!(
            "aliases={:?};prefixes={:?};case={};schema={:?};branches={:?};completion={:?};shortcuts={:?}",
            self.aliases,
            self.prefixes,
            self.case_sensitive,
            self.schema,
            self.branches,
            self.completion,
            self.shortcuts,
        );
        stable_hash(structure.as_bytes())
    }

    /// Canonical root names used to reject ambiguous registrations across
    /// overlapping Bot scopes. Prefix variants belong to one command schema,
    /// so two separately registered commands with the same logical name are
    /// considered a conflict even when their textual prefixes differ.
    pub(crate) fn registration_keys(&self) -> Vec<Arc<str>> {
        let mut keys = Vec::new();
        for name in std::iter::once(&self.name).chain(self.aliases.iter()) {
            let key: Arc<str> = Arc::from(normalize_native_name(name).to_ascii_lowercase());
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        // Hash collisions are improbable but must fail deterministically rather
        // than aliasing two command schemas in caches and native publication.
        keys.push(Arc::from(format!("\0command-id:{:016x}", self.id.0)));
        keys
    }

    #[must_use]
    pub fn usage(&self) -> String {
        let mut usage = self.display_name();
        if let Some(schema) = &self.schema {
            for argument in schema.arguments() {
                usage.push(' ');
                usage.push_str(&argument.usage_fragment());
            }
        }
        usage
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        let max_key = oxidebot_core::event::kernel::MAX_ROUTE_KEY_BYTES;
        if self.name.trim().is_empty() {
            return Err("command name cannot be empty".into());
        }
        if self.prefixes.is_empty() {
            return Err(format!("command `{}` has no prefixes", self.name));
        }

        let mut names = HashSet::new();
        for name in std::iter::once(&self.name).chain(self.aliases.iter()) {
            let canonical = name.split_whitespace().collect::<Vec<_>>().join(" ");
            if canonical.is_empty() || canonical != name.as_ref() {
                return Err(format!(
                    "command path `{name}` must use single spaces without leading or trailing whitespace"
                ));
            }
            if name.len() > max_key {
                return Err(format!("command path `{name}` exceeds {max_key} bytes"));
            }
            let root = name
                .split_whitespace()
                .next()
                .ok_or_else(|| format!("command `{}` has an empty route root", self.name))?;
            if root.len() > max_key {
                return Err(format!(
                    "command route root `{root}` exceeds {max_key} bytes"
                ));
            }
            let identity = if self.case_sensitive {
                name.to_string()
            } else {
                name.to_ascii_lowercase()
            };
            if !names.insert(identity) {
                return Err(format!(
                    "command `{}` contains a duplicate name or alias",
                    self.name
                ));
            }
        }

        let mut prefixes = HashSet::new();
        for prefix in &self.prefixes {
            if prefix.len() > max_key {
                return Err(format!(
                    "command `{}` contains an oversized prefix",
                    self.name
                ));
            }
            if prefix.chars().any(char::is_whitespace) {
                return Err(format!(
                    "command `{}` contains a whitespace prefix",
                    self.name
                ));
            }
            if !prefixes.insert(prefix.as_ref()) {
                return Err(format!(
                    "command `{}` contains a duplicate prefix",
                    self.name
                ));
            }
        }

        if self.shortcuts.len() > 256 {
            return Err(format!(
                "command `{}` contains more than 256 shortcuts",
                self.name
            ));
        }
        let mut shortcut_patterns = HashSet::new();
        for shortcut in &self.shortcuts {
            let identity = (shortcut.is_regex(), shortcut.pattern_text().to_owned());
            if !shortcut_patterns.insert(identity) {
                return Err(format!(
                    "command `{}` contains a duplicate shortcut `{}`",
                    self.name,
                    shortcut.pattern_text(),
                ));
            }
        }

        if let Some(completion) = &self.completion {
            if completion.timeout.is_zero() {
                return Err(format!(
                    "command `{}` has a zero completion timeout",
                    self.name
                ));
            }
            if completion.max_rounds == 0 {
                return Err(format!(
                    "command `{}` has zero completion rounds",
                    self.name
                ));
            }
            if completion
                .cancel_words
                .iter()
                .any(|word| word.trim().is_empty())
            {
                return Err(format!(
                    "command `{}` has an empty completion cancel word",
                    self.name
                ));
            }
        }
        if let Some(schema) = &self.schema {
            schema.validate()?;
        }
        if self.schema.is_some() && !self.branches.is_empty() {
            return Err(format!(
                "command `{}` cannot contain both root arguments and subcommands; move shared values into explicit branch options",
                self.name
            ));
        }
        let mut branch_names = HashSet::new();
        for branch in &self.branches {
            for name in std::iter::once(&branch.name).chain(branch.aliases.iter()) {
                let identity = if self.case_sensitive {
                    name.to_string()
                } else {
                    name.to_ascii_lowercase()
                };
                if !branch_names.insert(identity) {
                    return Err(format!(
                        "command `{}` contains duplicate subcommand name or alias `{name}`",
                        self.name
                    ));
                }
            }
            branch.validate(&format!("{} {}", self.name, branch.name))?;
        }

        let mut node_ids = HashMap::<CommandNodeId, String>::new();
        let mut field_ids = HashMap::<CommandFieldId, String>::new();
        if let Some(schema) = &self.schema {
            collect_schema_field_ids(schema, &self.name, &mut field_ids)?;
        }
        for branch in &self.branches {
            collect_branch_ids(
                branch,
                &format!("{} {}", self.name, branch.name),
                &mut node_ids,
                &mut field_ids,
            )?;
        }
        Ok(())
    }

    /// Returns exact pre-decode keys when this command can use the fast `/name`
    /// path. `None` means it needs the broad message candidate table.
    pub(crate) fn fast_route_keys(&self) -> Option<Vec<Arc<str>>> {
        if !self.case_sensitive
            || self.prefixes.len() != 1
            || self
                .prefixes
                .first()
                .is_none_or(|prefix| prefix.as_ref() != "/")
        {
            return None;
        }
        let mut keys = Vec::new();
        for name in std::iter::once(&self.name).chain(self.aliases.iter()) {
            let root = name.split_whitespace().next()?;
            if root.is_empty() {
                return None;
            }
            let root: Arc<str> = Arc::from(root);
            if !keys.contains(&root) {
                keys.push(root);
            }
        }
        Some(keys)
    }

    /// Matches one canonical message without runtime-added shortcuts. This is
    /// useful for parser tools and focused command tests that do not need to
    /// start the full runtime.
    pub fn match_message(
        &self,
        message: &Message,
    ) -> Result<Option<CommandMatch>, CommandParseError> {
        self.match_message_with_shortcuts(message, &[])
    }

    /// Matches and parses one canonical message, returning a structured
    /// command result or a precise parse error.
    pub fn parse_message(&self, message: &Message) -> Result<CommandMatch, CommandParseError> {
        let matched =
            self.match_message(message)?
                .ok_or_else(|| CommandParseError::NotMatched {
                    command: Arc::clone(&self.name),
                })?;
        let parsed = matched.parse_active()?;
        Ok(matched.with_parsed(parsed))
    }

    pub(crate) fn match_message_with_shortcuts(
        &self,
        message: &Message,
        runtime_shortcuts: &[Shortcut],
    ) -> Result<Option<CommandMatch>, CommandParseError> {
        let tokens = tokenize_segments(&message.segments)?;
        if let Some(result) = self.match_tokens(tokens) {
            return Ok(Some(result));
        }
        let input = message.extract_plain_text();
        for shortcut in self.shortcuts.iter().chain(runtime_shortcuts) {
            let Some(rewritten) = shortcut.rewrite(&input) else {
                continue;
            };
            let tokens = tokenize_text(&rewritten)?
                .into_iter()
                .map(CommandValue::Text)
                .collect();
            if let Some(result) = self.match_tokens(tokens) {
                return Ok(Some(result));
            }
        }
        Ok(None)
    }

    fn match_tokens(&self, tokens: Vec<CommandValue>) -> Option<CommandMatch> {
        let first = tokens.first()?.as_text()?;
        let mut matched: Option<(usize, Arc<str>, Arc<str>)> = None;
        let names = std::iter::once(&self.name).chain(self.aliases.iter());

        for prefix in &self.prefixes {
            let Some(root) = first.strip_prefix(prefix.as_ref()) else {
                continue;
            };
            let root = root.split('@').next().unwrap_or(root);
            for name in names.clone() {
                let path = name.split_whitespace().collect::<Vec<_>>();
                let Some(expected_root) = path.first() else {
                    continue;
                };
                if !same_word(root, expected_root, self.case_sensitive) {
                    continue;
                }
                let mut consumed = 1;
                let mut complete = true;
                for expected in path.iter().skip(1) {
                    let Some(actual) = tokens.get(consumed).and_then(CommandValue::as_text) else {
                        complete = false;
                        break;
                    };
                    if !same_word(actual, expected, self.case_sensitive) {
                        complete = false;
                        break;
                    }
                    consumed += 1;
                }
                if complete && matched.as_ref().is_none_or(|(best, _, _)| consumed > *best) {
                    matched = Some((consumed, Arc::clone(name), Arc::clone(prefix)));
                }
            }
        }

        let (mut consumed, invoked_as, prefix) = matched?;
        let mut schema = self.schema.clone().unwrap_or_default();
        let mut branch_path = Vec::new();
        let mut branch_names = Vec::new();
        let mut completion = self.completion.clone();
        let mut children = self.branches.as_slice();

        while let Some(actual) = tokens.get(consumed).and_then(CommandValue::as_text) {
            let Some(branch) = children
                .iter()
                .find(|branch| branch.matches(actual, self.case_sensitive))
            else {
                break;
            };
            consumed += 1;
            branch_path.push(branch.id);
            branch_names.push(Arc::clone(&branch.name));
            if let Some(branch_schema) = &branch.schema {
                schema = schema.merged(branch_schema);
            }
            if let Some(branch_completion) = &branch.completion {
                completion = Some(branch_completion.clone());
            }
            children = branch.children.as_slice();
        }

        Some(CommandMatch {
            command: self.clone(),
            invoked_as,
            prefix,
            branch_path: branch_path.into(),
            branch_names: branch_names.into(),
            values: tokens.into_iter().skip(consumed).collect::<Vec<_>>().into(),
            schema: Arc::new(schema),
            parsed: None,
            completion,
            source: CommandSource::Text,
            locale: None,
        })
    }

    pub(crate) fn match_invocation(
        &self,
        invocation: &CommandInvocation,
    ) -> Result<Option<CommandMatch>, CommandParseError> {
        let native_name = normalize_native_name(&invocation.name);
        let matched_name = std::iter::once(&self.name)
            .chain(self.aliases.iter())
            .find(|name| {
                same_word(
                    &native_name,
                    &normalize_native_name(name),
                    self.case_sensitive,
                )
            });
        let Some(invoked_as) = matched_name else {
            return Ok(None);
        };

        let mut path = invocation.path.as_slice();
        if path.first().is_some_and(|first| {
            same_word(
                &normalize_native_name(first),
                &normalize_native_name(invoked_as),
                self.case_sensitive,
            )
        }) {
            path = &path[1..];
        }

        let mut schema = self.schema.clone().unwrap_or_default();
        let mut branch_path = Vec::new();
        let mut branch_names = Vec::new();
        let mut completion = self.completion.clone();
        let mut children = self.branches.as_slice();
        for component in path {
            let Some(branch) = children
                .iter()
                .find(|branch| branch.matches(component, self.case_sensitive))
            else {
                return Err(unknown_subcommand(children, component, self.case_sensitive));
            };
            branch_path.push(branch.id);
            branch_names.push(Arc::clone(&branch.name));
            if let Some(branch_schema) = &branch.schema {
                schema = schema.merged(branch_schema);
            }
            if let Some(branch_completion) = &branch.completion {
                completion = Some(branch_completion.clone());
            }
            children = branch.children.as_slice();
        }

        let parsed = parse_native_arguments(&schema, invocation)?;
        let values = invocation
            .options
            .values()
            .flatten()
            .cloned()
            .map(CommandValue::Form)
            .collect::<Vec<_>>();
        Ok(Some(CommandMatch {
            command: self.clone(),
            invoked_as: Arc::clone(invoked_as),
            prefix: Arc::from(""),
            branch_path: branch_path.into(),
            branch_names: branch_names.into(),
            values: values.into(),
            schema: Arc::new(schema),
            parsed: Some(Arc::new(parsed)),
            completion,
            source: CommandSource::Native(Arc::new(invocation.clone())),
            locale: invocation.locale.as_deref().map(Arc::from),
        }))
    }

    #[must_use]
    pub fn definition(&self) -> CommandDefinition {
        let mut options = self
            .schema
            .as_ref()
            .map_or_else(Vec::new, CommandSchema::native_options);
        options.extend(self.branches.iter().map(CommandBranch::native_option));
        CommandDefinition {
            id: Some(format!("oxidebot:{:016x}", self.id.0)),
            name: Localized::new(normalize_native_name(&self.name)),
            description: if self.description.is_empty() {
                Localized::new(self.name.to_string())
            } else {
                self.description.to_core()
            },
            kind: CommandKind::ChatInput,
            options,
            default_permissions: None,
            contexts: std::iter::once(CommandContext::Any).collect(),
            ephemeral: false,
            platform_data: None,
        }
    }

    #[must_use]
    pub fn suggest(&self, input: &str, cursor: usize, locale: Option<&str>) -> Vec<CompletionItem> {
        suggest_for_command(self, input, cursor, locale)
    }

    #[must_use]
    pub fn usage_for(&self, branch_names: &[Arc<str>]) -> String {
        let mut usage = self.display_name();
        for branch in branch_names {
            usage.push(' ');
            usage.push_str(branch);
        }
        if !branch_children(self, branch_names).is_empty() {
            usage.push_str(" <subcommand>");
        }
        let schema = self.schema_for_branch_names(branch_names);
        for argument in schema.arguments() {
            usage.push(' ');
            usage.push_str(&argument.usage_fragment());
        }
        usage
    }

    fn schema_for_branch_names(&self, branch_names: &[Arc<str>]) -> CommandSchema {
        let mut schema = self.schema.clone().unwrap_or_default();
        let mut children = self.branches.as_slice();
        for name in branch_names {
            let Some(branch) = children.iter().find(|branch| branch.name == *name) else {
                break;
            };
            if let Some(branch_schema) = &branch.schema {
                schema = schema.merged(branch_schema);
            }
            children = branch.children.as_slice();
        }
        schema
    }
}

fn collect_schema_field_ids(
    schema: &CommandSchema,
    path: &str,
    ids: &mut HashMap<CommandFieldId, String>,
) -> Result<(), String> {
    for argument in schema.arguments() {
        let field_path = format!("{path}.{}", argument.name());
        if let Some(previous) = ids.insert(argument.id(), field_path.clone()) {
            return Err(format!(
                "command grammar field ID {:?} collides between `{previous}` and `{field_path}`",
                argument.id(),
            ));
        }
    }
    Ok(())
}

fn collect_branch_ids(
    branch: &CommandBranch,
    path: &str,
    node_ids: &mut HashMap<CommandNodeId, String>,
    field_ids: &mut HashMap<CommandFieldId, String>,
) -> Result<(), String> {
    if let Some(previous) = node_ids.insert(branch.id(), path.to_owned()) {
        return Err(format!(
            "command grammar node ID {:?} collides between `{previous}` and `{path}`",
            branch.id(),
        ));
    }
    if let Some(schema) = branch.schema_ref() {
        collect_schema_field_ids(schema, path, field_ids)?;
    }
    for child in branch.children() {
        collect_branch_ids(
            child,
            &format!("{path} {}", child.name()),
            node_ids,
            field_ids,
        )?;
    }
    Ok(())
}

fn normalize_native_name(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
        .replace('_', "-")
}
fn same_word(actual: &str, expected: &str, case_sensitive: bool) -> bool {
    if case_sensitive {
        actual == expected
    } else {
        actual.eq_ignore_ascii_case(expected)
    }
}

/// Interactive recovery policy for missing required arguments.
#[derive(Clone, Debug)]
pub struct CompletionConfig {
    pub timeout: Duration,
    pub max_rounds: usize,
    pub cancel_words: Arc<[Arc<str>]>,
}

impl CompletionConfig {
    /// Creates the default interactive recovery policy.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the maximum wait time for one interactive response.
    #[must_use]
    pub const fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Sets the maximum number of interactive recovery rounds.
    #[must_use]
    pub const fn max_rounds(mut self, max_rounds: usize) -> Self {
        self.max_rounds = max_rounds;
        self
    }

    /// Replaces case-insensitive words that cancel interactive recovery.
    #[must_use]
    pub fn cancel_words<I, T>(mut self, words: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<Arc<str>>,
    {
        self.cancel_words = words.into_iter().map(Into::into).collect::<Vec<_>>().into();
        self
    }

    /// Returns whether `text` matches a configured cancellation word.
    #[must_use]
    pub fn is_cancelled(&self, text: &str) -> bool {
        self.cancel_words
            .iter()
            .any(|word| text.trim().eq_ignore_ascii_case(word))
    }
}

impl Default for CompletionConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(60),
            max_rounds: 3,
            cancel_words: vec![Arc::from("cancel"), Arc::from("stop"), Arc::from("取消")].into(),
        }
    }
}

/// Byte range in the original command input replaced by a completion item.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SourceSpan {
    /// Inclusive byte offset at which replacement begins.
    pub start: usize,
    /// Exclusive byte offset at which replacement ends.
    pub end: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionKind {
    /// Top-level command name.
    Command,
    /// Command grammar branch.
    Subcommand,
    /// Named option switch.
    Option,
    /// Positional argument.
    Argument,
    /// Declared argument choice.
    Choice,
}

/// One renderer-neutral completion result. The display and description use the
/// same canonical message IR as ordinary replies.
#[derive(Clone, Debug, PartialEq)]
pub struct CompletionItem {
    /// Text inserted into the input span.
    pub value: String,
    /// Canonical rich display representation.
    pub display: Message,
    /// Optional rich explanatory text.
    pub description: Option<Message>,
    /// Input span replaced by this completion.
    pub replace: SourceSpan,
    /// Semantic category for renderer presentation.
    pub kind: CompletionKind,
    /// Field that requested the completion, when applicable.
    pub field_id: Option<CommandFieldId>,
}

impl CompletionItem {
    #[must_use]
    pub fn new(value: impl Into<String>, kind: CompletionKind, replace: SourceSpan) -> Self {
        let value = value.into();
        Self {
            display: Message::text(value.clone()),
            value,
            description: None,
            replace,
            kind,
            field_id: None,
        }
    }

    #[must_use]
    pub fn description(mut self, description: impl Into<Message>) -> Self {
        self.description = Some(description.into());
        self
    }

    #[must_use]
    pub const fn field(mut self, field_id: CommandFieldId) -> Self {
        self.field_id = Some(field_id);
        self
    }

    pub(crate) fn into_suggestion(self, index: usize) -> Suggestion {
        Suggestion {
            id: format!("oxidebot-completion-{index}"),
            title: self.display.get_raw_text(),
            description: self.description.as_ref().map(Message::get_raw_text),
            image: None,
            value: FormValue::Text(self.value),
            message: Some(self.display),
            platform_data: None,
        }
    }
}

fn cursor_prefix(input: &str, cursor: usize) -> (&str, SourceSpan) {
    let mut end = cursor.min(input.len());
    while !input.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    let prefix = &input[..end];
    let start = prefix
        .char_indices()
        .rev()
        .find_map(|(index, value)| value.is_whitespace().then_some(index + value.len_utf8()))
        .unwrap_or(0);
    (prefix, SourceSpan { start, end })
}

fn branch_children<'a>(command: &'a Command, path: &[Arc<str>]) -> &'a [CommandBranch] {
    let mut children = command.branches.as_slice();
    for name in path {
        let Some(branch) = children.iter().find(|branch| branch.name == *name) else {
            return &[];
        };
        children = branch.children.as_slice();
    }
    children
}

struct CompletionScan<'a> {
    supplied_positionals: usize,
    pending_option: Option<&'a ArgumentSpec>,
}

fn scan_completed_values<'a>(
    schema: &'a CommandSchema,
    values: &[CommandValue],
) -> CompletionScan<'a> {
    let mut supplied_positionals = 0;
    let mut options_enabled = true;
    let mut index = 0;

    while index < values.len() {
        let text = values[index].as_text();
        if options_enabled && text == Some("--") {
            options_enabled = false;
            index += 1;
            continue;
        }

        if options_enabled {
            if let Some(text) = text {
                if let Some(option) = text.strip_prefix("--").filter(|value| !value.is_empty()) {
                    let (name, attached) = option
                        .split_once('=')
                        .map_or((option, None), |(name, value)| (name, Some(value)));
                    if let Some(spec) = schema
                        .arguments
                        .iter()
                        .find(|argument| argument.long.as_deref() == Some(name))
                    {
                        if !spec.flag && attached.is_none() {
                            if index + 1 >= values.len() {
                                return CompletionScan {
                                    supplied_positionals,
                                    pending_option: Some(spec),
                                };
                            }
                            index += 2;
                        } else {
                            index += 1;
                        }
                        continue;
                    }
                    index += 1;
                    continue;
                }

                let negative_number =
                    text.starts_with('-') && text.len() > 1 && text.parse::<f64>().is_ok();
                if !negative_number {
                    if let Some(shorts) = text.strip_prefix('-').filter(|value| !value.is_empty()) {
                        let chars = shorts.chars().collect::<Vec<_>>();
                        let original_index = index;
                        let mut short_index = 0;
                        let mut matched = false;
                        while short_index < chars.len() {
                            let short = chars[short_index];
                            let Some(spec) = schema
                                .arguments
                                .iter()
                                .find(|argument| argument.short == Some(short))
                            else {
                                break;
                            };
                            matched = true;
                            if spec.flag {
                                short_index += 1;
                                continue;
                            }
                            if short_index + 1 == chars.len() {
                                if index + 1 >= values.len() {
                                    return CompletionScan {
                                        supplied_positionals,
                                        pending_option: Some(spec),
                                    };
                                }
                                index += 2;
                            } else {
                                index += 1;
                            }
                            break;
                        }
                        if matched {
                            if index == original_index {
                                index += 1;
                            }
                            continue;
                        }
                        index += 1;
                        continue;
                    }
                }
            }
        }

        supplied_positionals += 1;
        index += 1;
    }

    CompletionScan {
        supplied_positionals,
        pending_option: None,
    }
}

fn positional_argument(
    schema: &CommandSchema,
    supplied_positionals: usize,
) -> Option<&ArgumentSpec> {
    for (index, argument) in schema
        .arguments()
        .iter()
        .filter(|argument| argument.is_positional())
        .enumerate()
    {
        if argument.rest || argument.multiple {
            return (supplied_positionals >= index).then_some(argument);
        }
        if supplied_positionals == index {
            return Some(argument);
        }
    }
    None
}

fn attached_option_value<'a>(
    schema: &'a CommandSchema,
    partial: &'a str,
    replace: SourceSpan,
) -> Option<(&'a ArgumentSpec, &'a str, SourceSpan)> {
    if let Some(option) = partial.strip_prefix("--") {
        let (name, value) = option.split_once('=')?;
        let argument = schema
            .arguments
            .iter()
            .find(|argument| argument.long.as_deref() == Some(name) && !argument.flag)?;
        let start = replace.start + partial.find('=')? + 1;
        return Some((
            argument,
            value,
            SourceSpan {
                start,
                end: replace.end,
            },
        ));
    }

    let shorts = partial.strip_prefix('-')?;
    if shorts.is_empty() || partial.parse::<f64>().is_ok() {
        return None;
    }
    for (offset, short) in shorts.char_indices() {
        let argument = schema
            .arguments
            .iter()
            .find(|argument| argument.short == Some(short))?;
        if argument.flag {
            continue;
        }
        let value_start = offset + short.len_utf8();
        if value_start >= shorts.len() {
            return None;
        }
        let value = &shorts[value_start..];
        return Some((
            argument,
            value,
            SourceSpan {
                start: replace.start + 1 + value_start,
                end: replace.end,
            },
        ));
    }
    None
}

fn suggest_argument(
    argument: &ArgumentSpec,
    partial: &str,
    replace: SourceSpan,
    locale: Option<&str>,
) -> Vec<CompletionItem> {
    if argument.choices.is_empty() {
        let value = format!(
            "<{}>",
            argument.value_name.as_deref().unwrap_or(argument.name())
        );
        let mut item =
            CompletionItem::new(value, CompletionKind::Argument, replace).field(argument.id);
        let description = argument.help_text(locale);
        if !description.is_empty() {
            item.description = Some(Message::text(description));
        }
        return vec![item];
    }

    argument
        .choices
        .iter()
        .filter(|choice| partial.is_empty() || choice.value.starts_with(partial))
        .map(|choice| {
            let mut item =
                CompletionItem::new(choice.value.to_string(), CompletionKind::Choice, replace)
                    .field(argument.id);
            item.display = Message::text(choice.name.resolve(locale));
            item
        })
        .collect()
}

fn suggest_for_command(
    command: &Command,
    input: &str,
    cursor: usize,
    locale: Option<&str>,
) -> Vec<CompletionItem> {
    let (prefix, replace) = cursor_prefix(input, cursor);
    let ends_with_space = prefix.chars().last().is_some_and(char::is_whitespace);
    let tokens = tokenize_text(prefix)
        .unwrap_or_else(|_| prefix.split_whitespace().map(ToOwned::to_owned).collect());
    if tokens.is_empty() {
        return Vec::new();
    }
    let partial = if ends_with_space {
        ""
    } else {
        tokens.last().map_or("", String::as_str)
    };
    let values = tokens
        .iter()
        .cloned()
        .map(CommandValue::Text)
        .collect::<Vec<_>>();
    let Some(matched) = command.match_tokens(values) else {
        return Vec::new();
    };

    let mut output = Vec::new();
    let children = branch_children(command, matched.branch_names());
    if matched.values().len() <= usize::from(!ends_with_space) && !children.is_empty() {
        for branch in children.iter().filter(|branch| {
            !branch.hidden
                && (partial.is_empty()
                    || branch.name.starts_with(partial)
                    || branch
                        .aliases
                        .iter()
                        .any(|alias| alias.starts_with(partial)))
        }) {
            let mut item =
                CompletionItem::new(branch.name.to_string(), CompletionKind::Subcommand, replace);
            let description = branch.localized_description(locale);
            if !description.is_empty() {
                item.description = Some(Message::text(description));
            }
            output.push(item);
        }
        if !output.is_empty() {
            return output;
        }
    }

    let schema = matched.schema();
    let completed_len = matched
        .values()
        .len()
        .saturating_sub(usize::from(!ends_with_space));
    let scan = scan_completed_values(schema, &matched.values()[..completed_len]);
    if let Some((argument, value, value_replace)) = attached_option_value(schema, partial, replace)
    {
        return suggest_argument(argument, value, value_replace, locale);
    }
    if let Some(argument) = scan.pending_option {
        return suggest_argument(argument, partial, replace, locale);
    }

    if partial.starts_with('-') || ends_with_space {
        for argument in schema
            .arguments()
            .iter()
            .filter(|argument| argument.long.is_some() || argument.short.is_some())
        {
            let candidate = argument
                .long
                .as_ref()
                .map(|value| format!("--{value}"))
                .or_else(|| argument.short.map(|value| format!("-{value}")))
                .expect("filtered command option has a name");
            if !partial.is_empty() && !candidate.starts_with(partial) {
                continue;
            }
            let mut item =
                CompletionItem::new(candidate, CompletionKind::Option, replace).field(argument.id);
            let description = argument.help_text(locale);
            if !description.is_empty() {
                item.description = Some(Message::text(description));
            }
            output.push(item);
        }
        if !output.is_empty() && partial.starts_with('-') {
            return output;
        }
    }

    if let Some(argument) = positional_argument(schema, scan.supplied_positionals) {
        output.extend(suggest_argument(argument, partial, replace, locale));
    }
    output
}

/// Runtime-neutral schema generated by `#[derive(CommandArgs)]` or built by
/// hand. The same schema drives text parsing, native commands, help, completion,
/// validation, and interactive recovery.
#[derive(Clone, Debug, Default)]
pub struct CommandSchema {
    arguments: Vec<ArgumentSpec>,
}

impl CommandSchema {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn argument(mut self, argument: ArgumentSpec) -> Self {
        self.arguments.push(argument);
        self
    }

    #[must_use]
    pub fn arguments(&self) -> &[ArgumentSpec] {
        &self.arguments
    }

    #[must_use]
    pub fn find(&self, name: &str) -> Option<&ArgumentSpec> {
        self.arguments
            .iter()
            .find(|argument| argument.name.as_ref() == name)
    }

    #[must_use]
    pub fn find_by_id(&self, id: CommandFieldId) -> Option<&ArgumentSpec> {
        self.arguments.iter().find(|argument| argument.id == id)
    }

    #[must_use]
    pub fn merged(&self, other: &Self) -> Self {
        let mut arguments =
            Vec::with_capacity(self.arguments.len().saturating_add(other.arguments.len()));
        arguments.extend(self.arguments.iter().cloned());
        arguments.extend(other.arguments.iter().cloned());
        Self { arguments }
    }

    fn rebase(&mut self, path: &str) {
        for argument in &mut self.arguments {
            argument.rebase(path);
        }
    }

    #[must_use]
    pub fn native_options(&self) -> Vec<CommandOption> {
        self.arguments
            .iter()
            .map(ArgumentSpec::native_option)
            .collect()
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        let mut ids = HashSet::new();
        let mut names = HashSet::new();
        let mut longs = HashSet::new();
        let mut shorts = HashSet::new();
        let mut seen_optional_positional = false;
        let mut seen_variadic = false;
        for argument in &self.arguments {
            if !ids.insert(argument.id) {
                return Err(format!(
                    "command schema contains colliding field id {:?} for `{}`",
                    argument.id, argument.name
                ));
            }
            if argument.name.trim().is_empty()
                || argument.name.trim() != argument.name.as_ref()
                || argument.name.chars().any(char::is_whitespace)
                || !names.insert(argument.name.clone())
            {
                return Err(format!("duplicate or invalid argument `{}`", argument.name));
            }
            if argument.long.as_deref().is_some_and(|name| {
                name.is_empty()
                    || name.starts_with('-')
                    || name.contains('=')
                    || name.chars().any(char::is_whitespace)
            }) {
                return Err(format!("invalid long option for `{}`", argument.name));
            }
            if argument
                .short
                .is_some_and(|name| name == '-' || name.is_whitespace() || name.is_ascii_digit())
            {
                return Err(format!("invalid short option for `{}`", argument.name));
            }
            if argument.flag && argument.multiple && argument.action != ArgumentAction::Count {
                return Err(format!(
                    "flag `{}` cannot be multiple unless it uses the count action",
                    argument.name
                ));
            }
            if argument.flag && argument.default.is_some() {
                return Err(format!(
                    "flag `{}` cannot have a default value",
                    argument.name
                ));
            }
            if argument.flag && argument.required {
                return Err(format!("flag `{}` cannot be required", argument.name));
            }
            if argument.default.is_some() && argument.required {
                return Err(format!(
                    "defaulted argument `{}` cannot also be required",
                    argument.name
                ));
            }
            if argument.default.is_some() && (argument.multiple || argument.rest) {
                return Err(format!(
                    "variadic argument `{}` cannot have a default value",
                    argument.name
                ));
            }
            if argument.flag && argument.is_positional() {
                return Err(format!(
                    "flag `{}` needs a long or short option name",
                    argument.name
                ));
            }
            if matches!(
                argument.action,
                ArgumentAction::SetTrue | ArgumentAction::SetFalse | ArgumentAction::Count
            ) && argument.is_positional()
            {
                return Err(format!(
                    "action {:?} on `{}` requires a long or short option",
                    argument.action, argument.name
                ));
            }
            if argument.action == ArgumentAction::Append && !argument.is_multiple() {
                return Err(format!(
                    "append action on `{}` requires multiple values",
                    argument.name
                ));
            }
            if argument
                .long
                .as_ref()
                .is_some_and(|name| !longs.insert(name.clone()))
            {
                return Err(format!("duplicate long option for `{}`", argument.name));
            }
            if argument.short.is_some_and(|name| !shorts.insert(name)) {
                return Err(format!("duplicate short option for `{}`", argument.name));
            }
            if argument.rest && !argument.is_positional() {
                return Err(format!(
                    "rest argument `{}` must be positional",
                    argument.name
                ));
            }
            if argument.rest || (argument.multiple && argument.is_positional()) {
                if seen_variadic {
                    return Err("a command can have only one variadic positional argument".into());
                }
                seen_variadic = true;
            } else if seen_variadic && argument.is_positional() {
                return Err("variadic argument must be the final positional argument".into());
            }
            if argument.is_positional() {
                if !argument.required || argument.default.is_some() {
                    seen_optional_positional = true;
                } else if seen_optional_positional && !argument.rest {
                    return Err("required positional argument follows an optional one".into());
                }
            }
            if argument
                .min_value
                .zip(argument.max_value)
                .is_some_and(|(min, max)| min > max)
            {
                return Err(format!("invalid numeric range for `{}`", argument.name));
            }
            if argument
                .min_length
                .zip(argument.max_length)
                .is_some_and(|(min, max)| min > max)
            {
                return Err(format!("invalid length range for `{}`", argument.name));
            }
            let mut choice_values = HashSet::new();
            if argument
                .choices
                .iter()
                .any(|choice| !choice_values.insert(choice.value.clone()))
            {
                return Err(format!("duplicate choice for `{}`", argument.name));
            }
        }

        for argument in &self.arguments {
            for required in &argument.requires {
                if required == &argument.name || !names.contains(required) {
                    return Err(format!(
                        "argument `{}` requires unknown or self field `{required}`",
                        argument.name
                    ));
                }
            }
            for conflict in &argument.conflicts {
                if conflict == &argument.name || !names.contains(conflict) {
                    return Err(format!(
                        "argument `{}` conflicts with unknown or self field `{conflict}`",
                        argument.name
                    ));
                }
            }
        }
        Ok(())
    }
}

/// One command argument or option.
#[derive(Clone, Debug)]
pub struct ArgumentSpec {
    id: CommandFieldId,
    name: Arc<str>,
    help: LocalizedText,
    prompt: Option<LocalizedText>,
    value_name: Option<Arc<str>>,
    long: Option<Arc<str>>,
    short: Option<char>,
    required: bool,
    multiple: bool,
    rest: bool,
    flag: bool,
    default: Option<Arc<str>>,
    kind: CommandValueKind,
    action: ArgumentAction,
    choices: Vec<ArgumentChoice>,
    autocomplete: bool,
    min_value: Option<f64>,
    max_value: Option<f64>,
    min_length: Option<u32>,
    max_length: Option<u32>,
    allowed_conversation_kinds: Vec<ConversationKind>,
    requires: Vec<Arc<str>>,
    conflicts: Vec<Arc<str>>,
}

impl ArgumentSpec {
    #[must_use]
    pub fn new(name: impl Into<Arc<str>>) -> Self {
        let name = name.into();
        Self {
            id: CommandFieldId(stable_hash(name.as_bytes()) as u32),
            name,
            help: LocalizedText::default(),
            prompt: None,
            value_name: None,
            long: None,
            short: None,
            required: true,
            multiple: false,
            rest: false,
            flag: false,
            default: None,
            kind: CommandValueKind::String,
            action: ArgumentAction::Store,
            choices: Vec::new(),
            autocomplete: false,
            min_value: None,
            max_value: None,
            min_length: None,
            max_length: None,
            allowed_conversation_kinds: Vec::new(),
            requires: Vec::new(),
            conflicts: Vec::new(),
        }
    }

    fn rebase(&mut self, path: &str) {
        self.id = CommandFieldId(stable_hash(format!("{path}.{}", self.name).as_bytes()) as u32);
    }

    #[must_use]
    pub fn help(mut self, help: impl Into<LocalizedText>) -> Self {
        self.help = help.into();
        self
    }

    #[must_use]
    pub fn help_translation(
        mut self,
        locale: impl Into<Arc<str>>,
        help: impl Into<Arc<str>>,
    ) -> Self {
        self.help = self.help.translation(locale, help);
        self
    }

    #[must_use]
    pub fn prompt(mut self, prompt: impl Into<LocalizedText>) -> Self {
        self.prompt = Some(prompt.into());
        self
    }

    #[must_use]
    pub fn prompt_translation(
        mut self,
        locale: impl Into<Arc<str>>,
        prompt: impl Into<Arc<str>>,
    ) -> Self {
        let value = self
            .prompt
            .take()
            .unwrap_or_default()
            .translation(locale, prompt);
        self.prompt = Some(value);
        self
    }

    #[must_use]
    pub fn value_name(mut self, value_name: impl Into<Arc<str>>) -> Self {
        self.value_name = Some(value_name.into());
        self
    }

    #[must_use]
    pub fn long(mut self, long: impl Into<Arc<str>>) -> Self {
        self.long = Some(long.into());
        self
    }

    #[must_use]
    pub const fn short(mut self, short: char) -> Self {
        self.short = Some(short);
        self
    }

    #[must_use]
    pub const fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    #[must_use]
    pub const fn multiple(mut self, multiple: bool) -> Self {
        self.multiple = multiple;
        self
    }

    #[must_use]
    pub const fn rest(mut self, rest: bool) -> Self {
        self.rest = rest;
        self
    }

    #[must_use]
    pub const fn flag(mut self, flag: bool) -> Self {
        self.flag = flag;
        if flag {
            self.required = false;
            if matches!(self.action, ArgumentAction::Store) {
                self.action = ArgumentAction::SetTrue;
            }
        }
        self
    }

    #[must_use]
    pub fn default_value(mut self, default: impl Into<Arc<str>>) -> Self {
        self.default = Some(default.into());
        self.required = false;
        self
    }

    #[must_use]
    pub fn kind(mut self, kind: CommandValueKind) -> Self {
        self.kind = kind;
        self
    }

    #[must_use]
    pub fn action(mut self, action: ArgumentAction) -> Self {
        self.action = action;
        if matches!(
            action,
            ArgumentAction::SetTrue | ArgumentAction::SetFalse | ArgumentAction::Count
        ) {
            self.flag = true;
            self.required = false;
        }
        if action == ArgumentAction::Append {
            self.multiple = true;
        }
        self
    }

    #[must_use]
    pub fn choice(mut self, choice: ArgumentChoice) -> Self {
        self.choices.push(choice);
        self
    }

    #[must_use]
    pub const fn autocomplete(mut self, enabled: bool) -> Self {
        self.autocomplete = enabled;
        self
    }

    #[must_use]
    pub const fn min_value(mut self, value: f64) -> Self {
        self.min_value = Some(value);
        self
    }

    #[must_use]
    pub const fn max_value(mut self, value: f64) -> Self {
        self.max_value = Some(value);
        self
    }

    #[must_use]
    pub const fn min_length(mut self, value: u32) -> Self {
        self.min_length = Some(value);
        self
    }

    #[must_use]
    pub const fn max_length(mut self, value: u32) -> Self {
        self.max_length = Some(value);
        self
    }

    #[must_use]
    pub fn allowed_conversation_kind(mut self, kind: ConversationKind) -> Self {
        self.allowed_conversation_kinds.push(kind);
        self
    }

    #[must_use]
    pub fn requires(mut self, name: impl Into<Arc<str>>) -> Self {
        self.requires.push(name.into());
        self
    }

    #[must_use]
    pub fn conflicts_with(mut self, name: impl Into<Arc<str>>) -> Self {
        self.conflicts.push(name.into());
        self
    }

    #[must_use]
    pub const fn id(&self) -> CommandFieldId {
        self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn long_name(&self) -> Option<&str> {
        self.long.as_deref()
    }

    #[must_use]
    pub const fn short_name(&self) -> Option<char> {
        self.short
    }

    #[must_use]
    pub const fn is_multiple(&self) -> bool {
        self.multiple || self.rest || matches!(self.action, ArgumentAction::Append)
    }

    #[must_use]
    pub const fn is_flag(&self) -> bool {
        self.flag
    }

    #[must_use]
    pub const fn action_kind(&self) -> ArgumentAction {
        self.action
    }

    #[must_use]
    pub fn value_kind(&self) -> &CommandValueKind {
        &self.kind
    }

    #[must_use]
    pub fn choices(&self) -> &[ArgumentChoice] {
        &self.choices
    }

    #[must_use]
    pub const fn is_autocomplete(&self) -> bool {
        self.autocomplete
    }

    #[must_use]
    pub fn help_text(&self, locale: Option<&str>) -> &str {
        self.help.resolve(locale)
    }

    #[must_use]
    pub fn prompt_text(&self) -> String {
        self.prompt_text_for(None)
    }

    #[must_use]
    pub fn prompt_text_for(&self, locale: Option<&str>) -> String {
        self.prompt.as_ref().map_or_else(
            || {
                if locale.is_some_and(|value| !value.starts_with("zh")) {
                    format!("Enter {}:", self.name)
                } else {
                    format!("请输入 {}：", self.name)
                }
            },
            |prompt| prompt.resolve(locale).to_owned(),
        )
    }

    #[must_use]
    pub fn is_required(&self) -> bool {
        self.required && self.default.is_none() && !self.flag
    }

    #[must_use]
    pub fn is_positional(&self) -> bool {
        self.long.is_none() && self.short.is_none()
    }

    fn usage_fragment(&self) -> String {
        let value = self
            .value_name
            .as_deref()
            .unwrap_or(self.name.as_ref())
            .to_ascii_uppercase();
        let body = if let Some(long) = &self.long {
            if self.flag {
                self.short.map_or_else(
                    || format!("--{long}"),
                    |short| format!("-{short}, --{long}"),
                )
            } else {
                self.short.map_or_else(
                    || format!("--{long} <{value}>"),
                    |short| format!("-{short}, --{long} <{value}>"),
                )
            }
        } else if let Some(short) = self.short {
            if self.flag {
                format!("-{short}")
            } else {
                format!("-{short} <{value}>")
            }
        } else if self.multiple || self.rest {
            format!("<{value}>...")
        } else {
            format!("<{value}>")
        };
        if self.is_required() {
            body
        } else {
            format!("[{body}]")
        }
    }

    fn native_option(&self) -> CommandOption {
        CommandOption {
            id: Some(format!("oxidebot:{}", self.id.0)),
            name: Localized::new(
                self.long
                    .as_deref()
                    .unwrap_or(self.name.as_ref())
                    .replace('_', "-"),
            ),
            description: if self.help.is_empty() {
                Localized::new(self.name.to_string())
            } else {
                self.help.to_core()
            },
            kind: self.kind.native_kind(),
            required: self.is_required(),
            choices: self
                .choices
                .iter()
                .map(|choice| CommandChoice {
                    name: choice.name.to_core(),
                    value: choice.value.to_string(),
                })
                .collect(),
            options: Vec::new(),
            autocomplete: self.autocomplete,
            min_value: self.min_value,
            max_value: self.max_value,
            min_length: self.min_length,
            max_length: self.max_length,
            allowed_conversation_kinds: self.allowed_conversation_kinds.clone(),
            platform_data: None,
        }
    }
}

/// One lossless command token. Non-text message segments and native form
/// values remain typed until a field requests conversion.
// `MessageSegment` remains inline to preserve the ergonomic public value API.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum CommandValue {
    Text(String),
    Mention(String),
    File(File),
    Segment(MessageSegment),
    Form(FormValue),
}

impl CommandValue {
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(value) | Self::Form(FormValue::Text(value)) => Some(value),
            Self::Mention(_) | Self::File(_) | Self::Segment(_) | Self::Form(_) => None,
        }
    }

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

    fn scalar_text(self) -> Result<String, CommandParseError> {
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

fn form_value_label(value: &FormValue) -> String {
    match value {
        FormValue::Text(value) => value.clone(),
        FormValue::Integer(value) => value.to_string(),
        FormValue::Number(value) => value.to_string(),
        FormValue::Boolean(value) => value.to_string(),
        FormValue::Date(value) => value.to_string(),
        FormValue::Time(value) => value.to_string(),
        FormValue::DateTime(value) => value.to_rfc3339(),
        FormValue::User(value) => format!("@{}", value.id),
        FormValue::Conversation(value) => value.id.clone(),
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
            CommandValue::Mention(user_id) => Ok(Self::Text(user_id)),
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
pub struct Mention(pub String);

impl FromCommandValue for Mention {
    fn from_command_value(value: CommandValue) -> Result<Self, CommandParseError> {
        match value {
            CommandValue::Mention(user_id)
            | CommandValue::Segment(MessageSegment::At { user_id }) => Ok(Self(user_id)),
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

fn value_kind(value: &CommandValue) -> &'static str {
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

/// Parsed values indexed by both schema field name and deterministic field ID.
#[derive(Clone, Debug, Default)]
pub struct ParsedArguments {
    values: HashMap<Arc<str>, Vec<CommandValue>>,
    values_by_id: HashMap<CommandFieldId, Vec<CommandValue>>,
    present: HashSet<Arc<str>>,
    present_ids: HashSet<CommandFieldId>,
    flags: HashSet<Arc<str>>,
    flag_ids: HashSet<CommandFieldId>,
    counts: HashMap<Arc<str>, u32>,
    counts_by_id: HashMap<CommandFieldId, u32>,
}

impl ParsedArguments {
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.present.contains(name)
    }

    #[must_use]
    pub fn contains_id(&self, id: CommandFieldId) -> bool {
        self.present_ids.contains(&id)
    }

    pub fn required<T>(&self, name: &str) -> Result<T, CommandParseError>
    where
        T: FromCommandValue,
    {
        let value = self
            .values
            .get(name)
            .and_then(|values| values.first())
            .cloned()
            .ok_or_else(|| CommandParseError::MissingArgument {
                name: Arc::from(name),
                prompt: Arc::from(format!("请输入 {name}：")),
            })?;
        T::from_command_value(value)
    }

    pub fn required_id<T>(&self, id: CommandFieldId) -> Result<T, CommandParseError>
    where
        T: FromCommandValue,
    {
        let value = self
            .values_by_id
            .get(&id)
            .and_then(|values| values.first())
            .cloned()
            .ok_or(CommandParseError::MissingFieldId { id })?;
        T::from_command_value(value)
    }

    pub fn optional<T>(&self, name: &str) -> Result<Option<T>, CommandParseError>
    where
        T: FromCommandValue,
    {
        self.values
            .get(name)
            .and_then(|values| values.first())
            .cloned()
            .map(T::from_command_value)
            .transpose()
    }

    pub fn optional_id<T>(&self, id: CommandFieldId) -> Result<Option<T>, CommandParseError>
    where
        T: FromCommandValue,
    {
        self.values_by_id
            .get(&id)
            .and_then(|values| values.first())
            .cloned()
            .map(T::from_command_value)
            .transpose()
    }

    pub fn many<T>(&self, name: &str) -> Result<Vec<T>, CommandParseError>
    where
        T: FromCommandValue,
    {
        self.values
            .get(name)
            .into_iter()
            .flatten()
            .cloned()
            .map(T::from_command_value)
            .collect()
    }

    pub fn many_id<T>(&self, id: CommandFieldId) -> Result<Vec<T>, CommandParseError>
    where
        T: FromCommandValue,
    {
        self.values_by_id
            .get(&id)
            .into_iter()
            .flatten()
            .cloned()
            .map(T::from_command_value)
            .collect()
    }

    #[must_use]
    pub fn flag(&self, name: &str) -> bool {
        self.flags.contains(name)
    }

    #[must_use]
    pub fn flag_id(&self, id: CommandFieldId) -> bool {
        self.flag_ids.contains(&id)
    }

    #[must_use]
    pub fn count(&self, name: &str) -> u32 {
        self.counts.get(name).copied().unwrap_or_default()
    }

    #[must_use]
    pub fn count_id(&self, id: CommandFieldId) -> u32 {
        self.counts_by_id.get(&id).copied().unwrap_or_default()
    }

    #[must_use]
    pub fn values(&self, name: &str) -> &[CommandValue] {
        self.values.get(name).map_or(&[], Vec::as_slice)
    }

    #[must_use]
    pub fn values_id(&self, id: CommandFieldId) -> &[CommandValue] {
        self.values_by_id.get(&id).map_or(&[], Vec::as_slice)
    }

    fn mark_present(&mut self, spec: &ArgumentSpec) {
        self.present.insert(Arc::clone(&spec.name));
        self.present_ids.insert(spec.id);
    }

    fn insert_value(&mut self, spec: &ArgumentSpec, value: CommandValue) {
        self.mark_present(spec);
        self.values
            .entry(Arc::clone(&spec.name))
            .or_default()
            .push(value.clone());
        self.values_by_id.entry(spec.id).or_default().push(value);
    }

    fn set_flag(&mut self, spec: &ArgumentSpec, value: bool) {
        self.mark_present(spec);
        if value {
            self.flags.insert(Arc::clone(&spec.name));
            self.flag_ids.insert(spec.id);
        } else {
            self.flags.remove(&spec.name);
            self.flag_ids.remove(&spec.id);
        }
    }

    fn increment_count(&mut self, spec: &ArgumentSpec) {
        self.mark_present(spec);
        *self.counts.entry(Arc::clone(&spec.name)).or_default() += 1;
        *self.counts_by_id.entry(spec.id).or_default() += 1;
    }

    fn set_count(&mut self, spec: &ArgumentSpec, value: u32) {
        self.mark_present(spec);
        self.counts.insert(Arc::clone(&spec.name), value);
        self.counts_by_id.insert(spec.id, value);
    }

    fn validate(
        &self,
        schema: &CommandSchema,
        locale: Option<&str>,
    ) -> Result<(), CommandParseError> {
        for spec in schema.arguments() {
            if spec.is_required() && !self.contains(spec.name()) {
                return Err(CommandParseError::MissingArgument {
                    name: Arc::clone(&spec.name),
                    prompt: Arc::from(spec.prompt_text_for(locale)),
                });
            }
            if !self.contains(spec.name()) {
                continue;
            }
            for required in &spec.requires {
                if !self.contains(required) {
                    return Err(CommandParseError::Requires {
                        argument: Arc::clone(&spec.name),
                        required: Arc::clone(required),
                    });
                }
            }
            for conflict in &spec.conflicts {
                if self.contains(conflict) {
                    return Err(CommandParseError::Conflict {
                        left: Arc::clone(&spec.name),
                        right: Arc::clone(conflict),
                    });
                }
            }
            for value in self.values(spec.name()) {
                validate_argument_value(spec, value)?;
            }
        }
        Ok(())
    }
}

/// Where a command match originated.
#[derive(Clone, Debug)]
pub enum CommandSource {
    Text,
    Native(Arc<CommandInvocation>),
}

/// Structured result shared by text commands and platform-native commands.
/// Parsing happens once before extractors run.
#[derive(Clone, Debug)]
pub struct CommandMatch {
    command: Command,
    invoked_as: Arc<str>,
    prefix: Arc<str>,
    branch_path: Arc<[CommandNodeId]>,
    branch_names: Arc<[Arc<str>]>,
    values: Arc<[CommandValue]>,
    schema: Arc<CommandSchema>,
    parsed: Option<Arc<ParsedArguments>>,
    completion: Option<CompletionConfig>,
    source: CommandSource,
    locale: Option<Arc<str>>,
}

impl CommandMatch {
    #[must_use]
    pub fn command(&self) -> &Command {
        &self.command
    }

    #[must_use]
    pub fn invoked_as(&self) -> &str {
        &self.invoked_as
    }

    #[must_use]
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    #[must_use]
    pub fn values(&self) -> &[CommandValue] {
        &self.values
    }

    #[must_use]
    pub fn schema(&self) -> &CommandSchema {
        &self.schema
    }

    #[must_use]
    pub fn arguments(&self) -> Option<&ParsedArguments> {
        self.parsed.as_deref()
    }

    #[must_use]
    pub fn source(&self) -> &CommandSource {
        &self.source
    }

    #[must_use]
    pub fn locale(&self) -> Option<&str> {
        self.locale.as_deref()
    }

    pub(crate) fn with_locale(mut self, locale: Option<Arc<str>>) -> Self {
        self.locale = locale;
        self
    }

    #[must_use]
    pub fn branch_path(&self) -> &[CommandNodeId] {
        &self.branch_path
    }

    #[must_use]
    pub fn branch_names(&self) -> &[Arc<str>] {
        &self.branch_names
    }

    /// Returns a lightweight owned view with leading command-tree branches
    /// removed. Parsed values and the active schema remain shared.
    #[must_use]
    pub fn descend(&self, levels: usize) -> Self {
        let branch_path = self
            .branch_path
            .iter()
            .skip(levels)
            .copied()
            .collect::<Vec<_>>();
        let branch_names = self
            .branch_names
            .iter()
            .skip(levels)
            .cloned()
            .collect::<Vec<_>>();
        Self {
            command: self.command.clone(),
            invoked_as: Arc::clone(&self.invoked_as),
            prefix: Arc::clone(&self.prefix),
            branch_path: branch_path.into(),
            branch_names: branch_names.into(),
            values: Arc::clone(&self.values),
            schema: Arc::clone(&self.schema),
            parsed: self.parsed.clone(),
            completion: self.completion.clone(),
            source: self.source.clone(),
            locale: self.locale.clone(),
        }
    }

    #[must_use]
    pub fn selected_branch_name(&self) -> Option<&str> {
        self.branch_names.last().map(AsRef::as_ref)
    }

    #[must_use]
    pub fn is_branch(&self, path: &[&str]) -> bool {
        self.branch_names.len() == path.len()
            && self
                .branch_names
                .iter()
                .zip(path)
                .all(|(actual, expected)| actual.eq_ignore_ascii_case(expected))
    }

    #[must_use]
    pub fn completion_ref(&self) -> Option<&CompletionConfig> {
        self.completion.as_ref()
    }

    pub fn text_values(&self) -> impl Iterator<Item = &str> {
        self.values.iter().filter_map(CommandValue::as_text)
    }

    fn incomplete_branch_error(&self) -> Option<CommandParseError> {
        let children = branch_children(&self.command, &self.branch_names);
        if children.is_empty() {
            return None;
        }
        let choices = branch_choice_list(children);
        let Some(value) = self.values.first() else {
            return Some(CommandParseError::MissingSubcommand { choices });
        };
        let Some(value) = value.as_text() else {
            return Some(CommandParseError::UnexpectedValue {
                expected: "subcommand",
                actual: value_kind(value),
            });
        };
        Some(unknown_subcommand(
            children,
            value,
            self.command.case_sensitive,
        ))
    }

    pub fn parse_with(&self, schema: &CommandSchema) -> Result<ParsedArguments, CommandParseError> {
        if let Some(error) = self.incomplete_branch_error() {
            return Err(error);
        }
        if std::ptr::eq(schema, self.schema.as_ref()) {
            if let Some(parsed) = &self.parsed {
                return Ok(parsed.as_ref().clone());
            }
        }
        parse_arguments(schema, &self.values, self.locale())
    }

    pub fn parse_active(&self) -> Result<ParsedArguments, CommandParseError> {
        if let Some(error) = self.incomplete_branch_error() {
            return Err(error);
        }
        self.parsed.as_ref().map_or_else(
            || parse_arguments(&self.schema, &self.values, self.locale()),
            |parsed| Ok(parsed.as_ref().clone()),
        )
    }

    pub(crate) fn with_parsed(mut self, parsed: ParsedArguments) -> Self {
        self.parsed = Some(Arc::new(parsed));
        self
    }

    pub(crate) fn with_appended(&self, mut values: Vec<CommandValue>) -> Self {
        let mut combined = Vec::with_capacity(self.values.len().saturating_add(values.len()));
        combined.extend(self.values.iter().cloned());
        combined.append(&mut values);
        let mut next = self.clone();
        next.values = combined.into();
        next.parsed = None;
        next
    }

    pub(crate) fn with_answer(&self, name: &str, values: Vec<CommandValue>) -> Self {
        let Some(spec) = self.schema.find(name) else {
            return self.with_appended(values);
        };
        let Some(option) = spec
            .long_name()
            .map(|long| format!("--{long}"))
            .or_else(|| spec.short_name().map(|short| format!("-{short}")))
        else {
            return self.with_appended(values);
        };

        let mut appended = Vec::with_capacity(values.len().saturating_mul(2));
        if spec.is_multiple() {
            for value in values {
                appended.push(CommandValue::Text(option.clone()));
                appended.push(value);
            }
        } else {
            appended.push(CommandValue::Text(option));
            appended.extend(values);
        }
        self.with_appended(appended)
    }
}

/// Converts the shared structured match into one handler argument.
pub trait FromCommandMatch: Sized + Send + 'static {
    fn from_match(result: &CommandMatch) -> Result<Self, CommandParseError>;
}

/// Implemented by strongly typed flat command argument structs.
pub trait CommandArgs: Sized + Send + 'static {
    fn schema() -> CommandSchema;
    fn from_arguments(arguments: &ParsedArguments) -> Result<Self, CommandParseError>;

    fn parse(result: &CommandMatch) -> Result<Self, CommandParseError> {
        let arguments = result.parse_with(&Self::schema())?;
        Self::from_arguments(&arguments)
    }

    #[must_use]
    fn command(name: impl Into<Arc<str>>) -> Command {
        Command::new(name).schema(Self::schema())
    }

    /// Defines and binds a flat argument command in one expression.
    #[must_use]
    fn feature<S, H, T>(name: impl Into<Arc<str>>, handler: H) -> crate::Feature<S>
    where
        S: Send + Sync + 'static,
        H: crate::IntoHandler<T, S>,
    {
        crate::Feature::command(Self::command(name), handler)
    }
}

impl<T> FromCommandMatch for T
where
    T: CommandArgs,
{
    fn from_match(result: &CommandMatch) -> Result<Self, CommandParseError> {
        let arguments = if let Some(arguments) = result.arguments() {
            arguments.clone()
        } else {
            result.parse_active()?
        };
        T::from_arguments(&arguments)
    }
}

/// Implemented by enums derived with `#[derive(BotCommand)]`.
///
/// The name deliberately describes the command grammar rather than colliding
/// with the platform menu `BotCommand` model from `oxidebot-core`.
pub trait CommandTree: FromCommandMatch {
    fn command() -> Command;

    /// Defines and binds the complete typed command tree in one expression.
    #[must_use]
    fn feature<S, H, T>(handler: H) -> crate::Feature<S>
    where
        S: Send + Sync + 'static,
        H: crate::IntoHandler<T, S>,
    {
        crate::Feature::command(Self::command(), handler)
    }
}

/// Command syntax and type conversion failures.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum CommandParseError {
    #[error("message did not match command `{command}`")]
    NotMatched { command: Arc<str> },
    #[error("missing required subcommand; expected one of: {choices}")]
    MissingSubcommand { choices: Arc<str> },
    #[error("unknown subcommand `{value}`{suggestion}; expected one of: {choices}")]
    UnknownSubcommand {
        value: Arc<str>,
        suggestion: CompletionSuggestion,
        choices: Arc<str>,
    },
    #[error("missing required argument `{name}`")]
    MissingArgument { name: Arc<str>, prompt: Arc<str> },
    #[error("missing schema field {id:?}")]
    MissingFieldId { id: CommandFieldId },
    #[error("unknown option `{option}`{suggestion}")]
    UnknownOption {
        option: Arc<str>,
        suggestion: CompletionSuggestion,
    },
    #[error("unknown native option `{option}`")]
    UnknownNativeOption { option: Arc<str> },
    #[error("option `{option}` requires a value")]
    MissingOptionValue { option: Arc<str> },
    #[error("option `{option}` was provided more than once")]
    DuplicateOption { option: Arc<str> },
    #[error("unexpected extra argument `{value}`")]
    ExtraArgument { value: Arc<str> },
    #[error("expected {expected}, received {actual}")]
    UnexpectedValue {
        expected: &'static str,
        actual: &'static str,
    },
    #[error("could not parse `{value}` as {expected}: {reason}")]
    InvalidValue {
        value: String,
        expected: &'static str,
        reason: String,
    },
    #[error("`{argument}` must be one of: {choices}")]
    InvalidChoice {
        argument: Arc<str>,
        choices: Arc<str>,
    },
    #[error("`{argument}` is outside the accepted range")]
    OutOfRange { argument: Arc<str> },
    #[error("`{argument}` has length {actual}, expected {expected}")]
    InvalidLength {
        argument: Arc<str>,
        actual: usize,
        expected: Arc<str>,
    },
    #[error("`{argument}` requires `{required}`")]
    Requires {
        argument: Arc<str>,
        required: Arc<str>,
    },
    #[error("`{left}` conflicts with `{right}`")]
    Conflict { left: Arc<str>, right: Arc<str> },
    #[error("unterminated quote in command")]
    UnterminatedQuote,
    #[error("interactive command completion was cancelled")]
    Cancelled,
    #[error("interactive command completion exceeded its configured rounds")]
    CompletionExhausted,
}

impl CommandParseError {
    #[must_use]
    pub fn missing_prompt(&self) -> Option<&str> {
        match self {
            Self::MissingArgument { prompt, .. } => Some(prompt),
            _ => None,
        }
    }

    #[must_use]
    pub fn missing_name(&self) -> Option<&str> {
        match self {
            Self::MissingArgument { name, .. } => Some(name),
            _ => None,
        }
    }

    #[must_use]
    pub fn localized_message(&self, locale: Option<&str>) -> String {
        let chinese = locale.is_none_or(|locale| locale.starts_with("zh"));
        if !chinese {
            return self.to_string();
        }
        match self {
            Self::NotMatched { command } => format!("消息没有匹配命令 `{command}`"),
            Self::MissingSubcommand { choices } => {
                format!("缺少子命令；可用子命令：{choices}")
            }
            Self::UnknownSubcommand {
                value,
                suggestion,
                choices,
            } => {
                if let Some(suggested) = suggestion.value() {
                    format!(
                        "未知子命令 `{value}`；你是不是想输入 `{suggested}`？可用子命令：{choices}"
                    )
                } else {
                    format!("未知子命令 `{value}`；可用子命令：{choices}")
                }
            }
            Self::MissingArgument { name, .. } => format!("缺少必填参数 `{name}`"),
            Self::MissingFieldId { id } => format!("命令字段 {id:?} 不存在"),
            Self::UnknownOption { option, suggestion } => {
                if let Some(value) = suggestion.value() {
                    format!("未知选项 `{option}`；你是不是想输入 `{value}`？")
                } else {
                    format!("未知选项 `{option}`")
                }
            }
            Self::UnknownNativeOption { option } => format!("平台命令包含未知选项 `{option}`"),
            Self::MissingOptionValue { option } => format!("选项 `{option}` 需要一个值"),
            Self::DuplicateOption { option } => format!("选项 `{option}` 不能重复出现"),
            Self::ExtraArgument { value } => format!("存在多余参数 `{value}`"),
            Self::UnexpectedValue { expected, actual } => {
                format!("需要 {expected}，但收到 {actual}")
            }
            Self::InvalidValue {
                value,
                expected,
                reason,
            } => format!("无法把 `{value}` 解析为 {expected}：{reason}"),
            Self::InvalidChoice { argument, choices } => {
                format!("参数 `{argument}` 必须是以下值之一：{choices}")
            }
            Self::OutOfRange { argument } => format!("参数 `{argument}` 超出允许范围"),
            Self::InvalidLength {
                argument,
                actual,
                expected,
            } => format!("参数 `{argument}` 长度为 {actual}，要求 {expected}"),
            Self::Requires { argument, required } => {
                format!("参数 `{argument}` 需要同时提供 `{required}`")
            }
            Self::Conflict { left, right } => {
                format!("参数 `{left}` 与 `{right}` 不能同时使用")
            }
            Self::UnterminatedQuote => "命令中存在未闭合的引号".to_owned(),
            Self::Cancelled => "已取消命令补全".to_owned(),
            Self::CompletionExhausted => "命令补全已达到最大轮数".to_owned(),
        }
    }
}

/// Display helper that avoids storing a formatted suggestion in every error.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CompletionSuggestion(Option<Arc<str>>);

impl CompletionSuggestion {
    fn new(value: Option<Arc<str>>) -> Self {
        Self(value)
    }

    #[must_use]
    pub fn value(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

impl fmt::Display for CompletionSuggestion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(value) = &self.0 {
            write!(formatter, "; did you mean `{value}`?")
        } else {
            Ok(())
        }
    }
}

fn parse_arguments(
    schema: &CommandSchema,
    values: &[CommandValue],
    locale: Option<&str>,
) -> Result<ParsedArguments, CommandParseError> {
    let mut output = ParsedArguments::default();
    let mut positional = Vec::new();
    let mut options_enabled = true;
    let mut index = 0;

    while index < values.len() {
        let value = &values[index];
        let text = value.as_text();
        if options_enabled && text == Some("--") {
            options_enabled = false;
            index += 1;
            continue;
        }
        if options_enabled {
            if let Some(text) = text {
                if let Some(option) = text.strip_prefix("--").filter(|value| !value.is_empty()) {
                    let (name, attached) = option
                        .split_once('=')
                        .map_or((option, None), |(name, value)| (name, Some(value)));
                    let spec = schema
                        .arguments
                        .iter()
                        .find(|argument| argument.long.as_deref() == Some(name))
                        .ok_or_else(|| unknown_option(schema, format!("--{name}")))?;
                    if spec.flag {
                        if attached.is_some() {
                            return Err(CommandParseError::ExtraArgument {
                                value: Arc::from(text),
                            });
                        }
                        apply_flag_action(&mut output, spec);
                    } else {
                        let option_value = if let Some(attached) = attached {
                            CommandValue::Text(attached.to_owned())
                        } else {
                            index += 1;
                            values.get(index).cloned().ok_or_else(|| {
                                CommandParseError::MissingOptionValue {
                                    option: Arc::from(format!("--{name}")),
                                }
                            })?
                        };
                        if !spec.is_multiple() && output.contains(spec.name()) {
                            return Err(CommandParseError::DuplicateOption {
                                option: Arc::from(format!("--{name}")),
                            });
                        }
                        output.insert_value(spec, option_value);
                    }
                    index += 1;
                    continue;
                }

                let negative_number =
                    text.starts_with('-') && text.len() > 1 && text.parse::<f64>().is_ok();
                if !negative_number {
                    if let Some(shorts) = text.strip_prefix('-').filter(|value| !value.is_empty()) {
                        let chars = shorts.chars().collect::<Vec<_>>();
                        let mut short_index = 0;
                        while short_index < chars.len() {
                            let short = chars[short_index];
                            let spec = schema
                                .arguments
                                .iter()
                                .find(|argument| argument.short == Some(short))
                                .ok_or_else(|| unknown_option(schema, format!("-{short}")))?;
                            if spec.flag {
                                apply_flag_action(&mut output, spec);
                                short_index += 1;
                                continue;
                            }
                            let remainder = chars[short_index + 1..].iter().collect::<String>();
                            let option_value = if remainder.is_empty() {
                                index += 1;
                                values.get(index).cloned().ok_or_else(|| {
                                    CommandParseError::MissingOptionValue {
                                        option: Arc::from(format!("-{short}")),
                                    }
                                })?
                            } else {
                                CommandValue::Text(remainder)
                            };
                            if !spec.is_multiple() && output.contains(spec.name()) {
                                return Err(CommandParseError::DuplicateOption {
                                    option: Arc::from(format!("-{short}")),
                                });
                            }
                            output.insert_value(spec, option_value);
                            break;
                        }
                        index += 1;
                        continue;
                    }
                }
            }
        }
        positional.push(value.clone());
        index += 1;
    }

    let positional_specs = schema
        .arguments
        .iter()
        .filter(|argument| argument.is_positional())
        .collect::<Vec<_>>();
    let mut value_index = 0;
    for spec in positional_specs {
        if spec.rest || spec.multiple {
            if value_index < positional.len() {
                for value in &positional[value_index..] {
                    output.insert_value(spec, value.clone());
                }
                value_index = positional.len();
            }
        } else if let Some(value) = positional.get(value_index) {
            output.insert_value(spec, value.clone());
            value_index += 1;
        }
    }
    if let Some(value) = positional.get(value_index) {
        return Err(CommandParseError::ExtraArgument {
            value: Arc::from(
                value
                    .as_text()
                    .map_or_else(|| value_kind(value).to_owned(), ToOwned::to_owned),
            ),
        });
    }

    output.validate(schema, locale)?;
    Ok(output)
}

fn parse_native_arguments(
    schema: &CommandSchema,
    invocation: &CommandInvocation,
) -> Result<ParsedArguments, CommandParseError> {
    let mut output = ParsedArguments::default();
    for (name, values) in &invocation.options {
        let Some(spec) = schema.arguments.iter().find(|argument| {
            argument.name.as_ref() == name || argument.long.as_deref() == Some(name)
        }) else {
            return Err(CommandParseError::UnknownNativeOption {
                option: Arc::from(name.as_str()),
            });
        };
        if spec.flag {
            match values.first() {
                Some(FormValue::Boolean(value)) => output.set_flag(spec, *value),
                Some(FormValue::Integer(value)) if spec.action == ArgumentAction::Count => {
                    output.set_count(spec, u32::try_from(*value).unwrap_or(u32::MAX));
                }
                Some(_) | None => apply_flag_action(&mut output, spec),
            }
            continue;
        }
        if !spec.is_multiple() && values.len() > 1 {
            return Err(CommandParseError::DuplicateOption {
                option: Arc::from(name.as_str()),
            });
        }
        for value in values {
            output.insert_value(spec, CommandValue::Form(value.clone()));
        }
    }
    output.validate(schema, invocation.locale.as_deref())?;
    Ok(output)
}

fn apply_flag_action(output: &mut ParsedArguments, spec: &ArgumentSpec) {
    match spec.action {
        ArgumentAction::Count => output.increment_count(spec),
        ArgumentAction::SetFalse => output.set_flag(spec, false),
        ArgumentAction::Store | ArgumentAction::Append | ArgumentAction::SetTrue => {
            output.set_flag(spec, true);
        }
    }
}

fn validate_argument_value(
    spec: &ArgumentSpec,
    value: &CommandValue,
) -> Result<(), CommandParseError> {
    let text = value
        .as_text()
        .map(ToOwned::to_owned)
        .or_else(|| match value {
            CommandValue::Form(value) => Some(form_value_label(value)),
            _ => None,
        });
    if !spec.choices.is_empty() {
        let valid = text.as_deref().is_some_and(|text| {
            spec.choices
                .iter()
                .any(|choice| choice.value.as_ref() == text)
        });
        if !valid {
            return Err(CommandParseError::InvalidChoice {
                argument: Arc::clone(&spec.name),
                choices: Arc::from(
                    spec.choices
                        .iter()
                        .map(|choice| choice.value.as_ref())
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
            });
        }
    }
    if spec.min_value.is_some() || spec.max_value.is_some() {
        let number = text
            .as_deref()
            .and_then(|value| value.parse::<f64>().ok())
            .ok_or_else(|| CommandParseError::OutOfRange {
                argument: Arc::clone(&spec.name),
            })?;
        if spec.min_value.is_some_and(|min| number < min)
            || spec.max_value.is_some_and(|max| number > max)
        {
            return Err(CommandParseError::OutOfRange {
                argument: Arc::clone(&spec.name),
            });
        }
    }
    if spec.min_length.is_some() || spec.max_length.is_some() {
        let actual = text.as_deref().map_or(1, |value| value.chars().count());
        if spec.min_length.is_some_and(|min| actual < min as usize)
            || spec.max_length.is_some_and(|max| actual > max as usize)
        {
            let expected = match (spec.min_length, spec.max_length) {
                (Some(min), Some(max)) => format!("{min}..={max}"),
                (Some(min), None) => format!(">={min}"),
                (None, Some(max)) => format!("<={max}"),
                (None, None) => String::new(),
            };
            return Err(CommandParseError::InvalidLength {
                argument: Arc::clone(&spec.name),
                actual,
                expected: Arc::from(expected),
            });
        }
    }
    Ok(())
}

fn branch_choice_list(children: &[CommandBranch]) -> Arc<str> {
    Arc::from(
        children
            .iter()
            .filter(|branch| !branch.hidden)
            .map(|branch| branch.name.as_ref())
            .collect::<Vec<_>>()
            .join(", "),
    )
}

fn unknown_subcommand(
    children: &[CommandBranch],
    value: &str,
    case_sensitive: bool,
) -> CommandParseError {
    let normalized = if case_sensitive {
        value.to_owned()
    } else {
        value.to_ascii_lowercase()
    };
    let suggestion = children
        .iter()
        .flat_map(|branch| {
            std::iter::once(branch.name.as_ref())
                .chain(branch.aliases.iter().map(AsRef::as_ref))
                .map(move |candidate| (branch.name.as_ref(), candidate))
        })
        .map(|(canonical, candidate)| {
            let candidate = if case_sensitive {
                candidate.to_owned()
            } else {
                candidate.to_ascii_lowercase()
            };
            (edit_distance(&normalized, &candidate), canonical)
        })
        .filter(|(distance, _)| *distance <= 3)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, canonical)| Arc::from(canonical));
    CommandParseError::UnknownSubcommand {
        value: Arc::from(value),
        suggestion: CompletionSuggestion::new(suggestion),
        choices: branch_choice_list(children),
    }
}

fn unknown_option(schema: &CommandSchema, option: String) -> CommandParseError {
    let candidates = schema.arguments.iter().flat_map(|argument| {
        argument
            .long
            .as_ref()
            .map(|long| format!("--{long}"))
            .into_iter()
            .chain(argument.short.map(|short| format!("-{short}")))
    });
    let suggestion = candidates
        .map(|candidate| (edit_distance(&option, &candidate), candidate))
        .filter(|(distance, _)| *distance <= 3)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, value)| Arc::from(value));
    CommandParseError::UnknownOption {
        option: Arc::from(option),
        suggestion: CompletionSuggestion::new(suggestion),
    }
}

fn edit_distance(left: &str, right: &str) -> usize {
    let mut previous = (0..=right.chars().count()).collect::<Vec<_>>();
    let mut current = vec![0; previous.len()];
    for (left_index, left_char) in left.chars().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_char) in right.chars().enumerate() {
            let substitution = previous[right_index] + usize::from(left_char != right_char);
            let insertion = current[right_index] + 1;
            let deletion = previous[right_index + 1] + 1;
            current[right_index + 1] = substitution.min(insertion).min(deletion);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.chars().count()]
}

/// Tokenizes canonical message segments while preserving mentions and files.
pub fn tokenize_segments(
    segments: &[MessageSegment],
) -> Result<Vec<CommandValue>, CommandParseError> {
    let mut output = Vec::new();
    for segment in segments {
        match segment {
            MessageSegment::Text { content } => {
                output.extend(tokenize_text(content)?.into_iter().map(CommandValue::Text));
            }
            MessageSegment::RichText(content) => {
                output.extend(
                    tokenize_text(&content.text)?
                        .into_iter()
                        .map(CommandValue::Text),
                );
            }
            MessageSegment::At { user_id } => output.push(CommandValue::Mention(user_id.clone())),
            MessageSegment::Media { media, .. } => {
                output.push(CommandValue::File(media.file.clone()));
            }
            other => output.push(CommandValue::Segment(other.clone())),
        }
    }
    Ok(output)
}

/// Shell-like tokenizer used by command text. It supports single and double
/// quotes, backslash escapes, and empty quoted values.
pub fn tokenize_text(input: &str) -> Result<Vec<String>, CommandParseError> {
    let mut output = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut started = false;

    for character in input.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            started = true;
            continue;
        }
        if character == '\\' && quote != Some('\'') {
            escaped = true;
            started = true;
            continue;
        }
        if let Some(active_quote) = quote {
            if character == active_quote {
                quote = None;
            } else {
                current.push(character);
            }
            started = true;
            continue;
        }
        match character {
            '\'' | '"' => {
                quote = Some(character);
                started = true;
            }
            value if value.is_whitespace() => {
                if started {
                    output.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            value => {
                current.push(value);
                started = true;
            }
        }
    }
    if escaped {
        current.push('\\');
    }
    if quote.is_some() {
        return Err(CommandParseError::UnterminatedQuote);
    }
    if started {
        output.push(current);
    }
    Ok(output)
}

/// Renderer-neutral command output. Help, diagnostics, and completion are kept
/// structured until the last step so they can become plain text, rich layout,
/// buttons, or platform-native UI without changing the parser.
// `Message` remains inline to preserve the ergonomic public output API.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum CommandOutput {
    Catalog {
        commands: Arc<[Command]>,
    },
    Help {
        command: Command,
        branch_names: Arc<[Arc<str>]>,
    },
    NotFound {
        query: Arc<str>,
        commands: Arc<[Command]>,
    },
    ParseError {
        command: Command,
        branch_names: Arc<[Arc<str>]>,
        error: CommandParseError,
    },
    Suggestions {
        items: Arc<[CompletionItem]>,
    },
    Message(Message),
}

/// Converts structured command output into the canonical message IR.
pub trait CommandRenderer: Send + Sync + 'static {
    fn render(&self, output: &CommandOutput, locale: Option<&str>) -> Message;
}

impl<F> CommandRenderer for F
where
    F: Fn(&CommandOutput, Option<&str>) -> Message + Send + Sync + 'static,
{
    fn render(&self, output: &CommandOutput, locale: Option<&str>) -> Message {
        (self)(output, locale)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultCommandRenderer;

impl CommandRenderer for DefaultCommandRenderer {
    fn render(&self, output: &CommandOutput, locale: Option<&str>) -> Message {
        match output {
            CommandOutput::Catalog { commands } => {
                Message::text(render_catalog_text(commands, locale))
            }
            CommandOutput::Help {
                command,
                branch_names,
            } => Message::text(render_command_help(command, branch_names, locale)),
            CommandOutput::NotFound { query, commands } => {
                let text = if is_chinese(locale) {
                    format!(
                        "没有找到命令 `{query}`。\n\n{}",
                        render_catalog_text(commands, locale)
                    )
                } else {
                    format!(
                        "Command `{query}` was not found.\n\n{}",
                        render_catalog_text(commands, locale)
                    )
                };
                Message::text(text)
            }
            CommandOutput::ParseError {
                command,
                branch_names,
                error,
            } => {
                let usage = command.usage_for(branch_names);
                let label = if is_chinese(locale) {
                    "用法"
                } else {
                    "Usage"
                };
                Message::text(format!(
                    "{}\n\n{label}：{usage}",
                    error.localized_message(locale)
                ))
            }
            CommandOutput::Suggestions { items } => {
                let heading = if is_chinese(locale) {
                    "可继续输入："
                } else {
                    "Suggestions:"
                };
                let mut text = String::from(heading);
                for item in items.iter() {
                    text.push_str("\n  ");
                    text.push_str(&item.display.get_raw_text());
                    if let Some(description) = &item.description {
                        let description = description.get_raw_text();
                        if !description.is_empty() {
                            text.push_str(" — ");
                            text.push_str(&description);
                        }
                    }
                }
                Message::text(text)
            }
            CommandOutput::Message(message) => message.clone(),
        }
    }
}

/// Resource-backed renderer that lets applications wrap or replace the
/// framework's structured command output with normal message templates.
/// Missing or invalid resource entries fall back to [`DefaultCommandRenderer`]
/// instead of making help/error delivery fallible.
#[derive(Clone, Debug)]
pub struct CatalogCommandRenderer {
    catalog: TranslationCatalog,
    prefix: Arc<str>,
}

impl CatalogCommandRenderer {
    #[must_use]
    pub fn new(catalog: TranslationCatalog) -> Self {
        Self {
            catalog,
            prefix: Arc::from("oxidebot.command"),
        }
    }

    #[must_use]
    pub fn prefix(mut self, value: impl Into<Arc<str>>) -> Self {
        self.prefix = value.into();
        self
    }

    fn key(&self, suffix: &str) -> String {
        format!("{}.{}", self.prefix.trim_end_matches('.'), suffix)
    }
}

impl CommandRenderer for CatalogCommandRenderer {
    fn render(&self, output: &CommandOutput, locale: Option<&str>) -> Message {
        if let CommandOutput::Message(message) = output {
            return message.clone();
        }
        let fallback = DefaultCommandRenderer.render(output, locale);
        let (suffix, mut values) = match output {
            CommandOutput::Catalog { .. } => ("catalog", BTreeMap::new()),
            CommandOutput::Help {
                command,
                branch_names,
            } => {
                let mut values = BTreeMap::new();
                values.insert(Arc::from("command"), TemplateValue::text(command.name()));
                values.insert(
                    Arc::from("usage"),
                    TemplateValue::text(command.usage_for(branch_names)),
                );
                ("help", values)
            }
            CommandOutput::NotFound { query, .. } => {
                let mut values = BTreeMap::new();
                values.insert(Arc::from("query"), TemplateValue::text(query.as_ref()));
                ("not_found", values)
            }
            CommandOutput::ParseError {
                command,
                branch_names,
                error,
            } => {
                let mut values = BTreeMap::new();
                values.insert(Arc::from("command"), TemplateValue::text(command.name()));
                values.insert(
                    Arc::from("usage"),
                    TemplateValue::text(command.usage_for(branch_names)),
                );
                values.insert(
                    Arc::from("error"),
                    TemplateValue::text(error.localized_message(locale)),
                );
                ("parse_error", values)
            }
            CommandOutput::Suggestions { .. } => ("suggestions", BTreeMap::new()),
            CommandOutput::Message(_) => unreachable!(),
        };
        values.insert(Arc::from("body"), TemplateValue::from(fallback.clone()));
        self.catalog
            .render(locale, &self.key(suffix), &values)
            .unwrap_or(fallback)
    }
}

fn is_chinese(locale: Option<&str>) -> bool {
    locale.is_none_or(|locale| locale.starts_with("zh"))
}

/// Immutable help/catalog view assembled from all registered commands.
#[derive(Clone, Debug, Default)]
pub struct CommandCatalog {
    commands: Arc<[Command]>,
    scopes: Arc<[RouteScope]>,
}

impl CommandCatalog {
    #[must_use]
    pub fn new(commands: impl IntoIterator<Item = Command>) -> Self {
        let commands = commands.into_iter().collect::<Vec<_>>();
        let scopes = vec![RouteScope::global(); commands.len()];
        Self {
            commands: commands.into(),
            scopes: scopes.into(),
        }
    }

    pub(crate) fn scoped(commands: impl IntoIterator<Item = (Command, RouteScope)>) -> Self {
        let (commands, scopes): (Vec<_>, Vec<_>) = commands.into_iter().unzip();
        Self {
            commands: commands.into(),
            scopes: scopes.into(),
        }
    }

    pub(crate) fn for_identity(&self, identity: &BotIdentity) -> Self {
        Self::scoped(
            self.commands
                .iter()
                .cloned()
                .zip(self.scopes.iter().cloned())
                .filter(|(_, scope)| scope.matches(identity)),
        )
    }

    #[must_use]
    pub fn commands(&self) -> &[Command] {
        &self.commands
    }

    #[must_use]
    pub fn find_by_id(&self, id: &str) -> Option<&Command> {
        self.commands
            .iter()
            .find(|command| id == format!("oxidebot:{:016x}", command.id().0))
    }

    #[must_use]
    pub fn find(&self, name: &str) -> Option<&Command> {
        let name = name.trim().trim_start_matches('/');
        self.commands.iter().find(|command| {
            command.name.eq_ignore_ascii_case(name)
                || normalize_native_name(&command.name).eq_ignore_ascii_case(name)
                || command.aliases.iter().any(|alias| {
                    alias.eq_ignore_ascii_case(name)
                        || normalize_native_name(alias).eq_ignore_ascii_case(name)
                })
        })
    }

    #[must_use]
    pub fn enabled(&self, registry: &crate::CommandRegistry) -> Self {
        let mut commands = Vec::new();
        let mut scopes = Vec::new();
        for (command, scope) in self.commands.iter().zip(self.scopes.iter()) {
            if registry.is_enabled(command.id()) {
                commands.push(command.clone());
                scopes.push(scope.clone());
            }
        }
        Self {
            commands: commands.into(),
            scopes: scopes.into(),
        }
    }

    #[must_use]
    pub fn definitions(&self) -> Vec<CommandDefinition> {
        self.commands
            .iter()
            .filter(|command| !command.hidden)
            .map(Command::definition)
            .collect()
    }

    #[must_use]
    pub fn output(&self, query: Option<&str>) -> CommandOutput {
        let Some(query) = query.filter(|value| !value.trim().is_empty()) else {
            return CommandOutput::Catalog {
                commands: Arc::clone(&self.commands),
            };
        };
        let normalized = query.trim().trim_start_matches('/');
        if let Some(command) = self.find(normalized) {
            return CommandOutput::Help {
                command: command.clone(),
                branch_names: Arc::from([]),
            };
        }
        let tokens = normalized.split_whitespace().collect::<Vec<_>>();
        let Some((command, consumed)) = self.commands.iter().find_map(|command| {
            command_name_consumed(command, &tokens).map(|consumed| (command, consumed))
        }) else {
            return CommandOutput::NotFound {
                query: Arc::from(query),
                commands: Arc::clone(&self.commands),
            };
        };
        let mut children = command.branches.as_slice();
        let mut branches = Vec::new();
        for token in tokens.iter().skip(consumed) {
            let Some(branch) = children
                .iter()
                .find(|branch| branch.matches(token, command.case_sensitive))
            else {
                return CommandOutput::NotFound {
                    query: Arc::from(query),
                    commands: Arc::clone(&self.commands),
                };
            };
            branches.push(Arc::clone(&branch.name));
            children = branch.children.as_slice();
        }
        CommandOutput::Help {
            command: command.clone(),
            branch_names: branches.into(),
        }
    }

    #[must_use]
    pub fn render_message(&self, query: Option<&str>, locale: Option<&str>) -> Message {
        DefaultCommandRenderer.render(&self.output(query), locale)
    }

    /// Renders the catalog as plain text.
    #[must_use]
    pub fn render_text(&self, query: Option<&str>) -> String {
        self.render_message(query, None).get_raw_text()
    }

    #[must_use]
    pub fn suggest(&self, input: &str, cursor: usize, locale: Option<&str>) -> Vec<CompletionItem> {
        let (prefix, replace) = cursor_prefix(input, cursor);
        let first = prefix.split_whitespace().next().unwrap_or(prefix);
        let has_complete_root = self.commands.iter().any(|command| {
            command.prefixes.iter().any(|command_prefix| {
                first
                    .strip_prefix(command_prefix.as_ref())
                    .is_some_and(|root| {
                        std::iter::once(&command.name)
                            .chain(command.aliases.iter())
                            .any(|name| {
                                name.split_whitespace().next().is_some_and(|expected| {
                                    same_word(root, expected, command.case_sensitive)
                                })
                            })
                    })
            })
        });
        if !has_complete_root {
            let mut output = Vec::new();
            for command in self.commands.iter().filter(|command| !command.hidden) {
                let display = command.display_name();
                if !display.starts_with(first) {
                    continue;
                }
                let mut item = CompletionItem::new(display, CompletionKind::Command, replace);
                let description = command.localized_description(locale);
                if !description.is_empty() {
                    item.description = Some(Message::text(description));
                }
                output.push(item);
            }
            return output;
        }
        self.commands
            .iter()
            .flat_map(|command| command.suggest(input, cursor, locale))
            .collect()
    }

    #[must_use]
    pub fn native_suggestions(&self, request: &SuggestionRequest) -> Vec<Suggestion> {
        let limit = request.limit.unwrap_or(25).min(100) as usize;
        let locale = request.locale.as_deref();
        let command = request
            .command_id
            .as_deref()
            .and_then(|id| self.find_by_id(id))
            .or_else(|| {
                request
                    .command_name
                    .as_deref()
                    .and_then(|name| self.find(name))
            });
        let mut items = if let Some(field) = request.field_id.as_deref() {
            self.find_argument_for_request(command, &request.command_path, field)
                .map_or_else(Vec::new, |argument| {
                    argument
                        .choices
                        .iter()
                        .filter(|choice| {
                            request.query.is_empty()
                                || choice
                                    .value
                                    .to_ascii_lowercase()
                                    .contains(&request.query.to_ascii_lowercase())
                                || choice
                                    .name
                                    .resolve(locale)
                                    .to_ascii_lowercase()
                                    .contains(&request.query.to_ascii_lowercase())
                        })
                        .map(|choice| {
                            let mut item = CompletionItem::new(
                                choice.value.to_string(),
                                CompletionKind::Choice,
                                SourceSpan::default(),
                            )
                            .field(argument.id);
                            item.display = Message::text(choice.name.resolve(locale));
                            item
                        })
                        .collect()
                })
        } else if let Some(command) = command {
            command.suggest(&request.query, request.query.len(), locale)
        } else {
            self.suggest(&request.query, request.query.len(), locale)
        };
        items.truncate(limit);
        items
            .into_iter()
            .enumerate()
            .map(|(index, item)| item.into_suggestion(index))
            .collect()
    }

    fn find_argument_for_request<'a>(
        &'a self,
        command: Option<&'a Command>,
        path: &[String],
        field: &str,
    ) -> Option<&'a ArgumentSpec> {
        let field_id = field
            .trim_start_matches("oxidebot:")
            .parse::<u32>()
            .ok()
            .map(CommandFieldId);
        if let Some(command) = command {
            let mut schema = command.schema.as_ref();
            let mut children = command.branches.as_slice();
            for component in path {
                let branch = children
                    .iter()
                    .find(|branch| branch.matches(component, command.case_sensitive))?;
                schema = branch.schema.as_ref().or(schema);
                children = branch.children.as_slice();
            }
            if let Some(argument) = find_argument_in_schema(schema, field, field_id) {
                return Some(argument);
            }
            return find_argument_in_branches(children, field, field_id);
        }
        for command in self.commands.iter() {
            if let Some(argument) =
                find_argument_in_schema(command.schema.as_ref(), field, field_id)
            {
                return Some(argument);
            }
            if let Some(argument) = find_argument_in_branches(&command.branches, field, field_id) {
                return Some(argument);
            }
        }
        None
    }
}

fn command_name_consumed(command: &Command, tokens: &[&str]) -> Option<usize> {
    std::iter::once(&command.name)
        .chain(command.aliases.iter())
        .filter_map(|name| {
            let path = name.split_whitespace().collect::<Vec<_>>();
            (path.len() <= tokens.len()
                && path
                    .iter()
                    .zip(tokens)
                    .all(|(expected, actual)| same_word(actual, expected, command.case_sensitive)))
            .then_some(path.len())
        })
        .max()
}

fn find_argument_in_schema<'a>(
    schema: Option<&'a CommandSchema>,
    field: &str,
    field_id: Option<CommandFieldId>,
) -> Option<&'a ArgumentSpec> {
    schema.and_then(|schema| {
        schema.arguments.iter().find(|argument| {
            argument.name.as_ref() == field
                || argument.long.as_deref() == Some(field)
                || field_id == Some(argument.id)
                || field == format!("oxidebot:{}", argument.id.0)
        })
    })
}

fn find_argument_in_branches<'a>(
    branches: &'a [CommandBranch],
    field: &str,
    field_id: Option<CommandFieldId>,
) -> Option<&'a ArgumentSpec> {
    for branch in branches {
        if let Some(argument) = find_argument_in_schema(branch.schema.as_ref(), field, field_id) {
            return Some(argument);
        }
        if let Some(argument) = find_argument_in_branches(&branch.children, field, field_id) {
            return Some(argument);
        }
    }
    None
}

fn render_catalog_text(commands: &[Command], locale: Option<&str>) -> String {
    let visible = commands.iter().filter(|command| !command.hidden);
    let mut groups = HashMap::<Option<Arc<str>>, Vec<&Command>>::new();
    for command in visible {
        groups
            .entry(command.category.clone())
            .or_default()
            .push(command);
    }
    let mut categories = groups.into_iter().collect::<Vec<_>>();
    categories.sort_by(|(left, _), (right, _)| left.cmp(right));
    let mut output = String::from(if is_chinese(locale) {
        "可用命令\n"
    } else {
        "Available commands\n"
    });
    for (category, mut commands) in categories {
        commands.sort_by(|left, right| left.name.cmp(&right.name));
        if let Some(category) = category {
            output.push('\n');
            output.push_str(&category);
            output.push('\n');
        }
        for command in commands {
            output.push_str("  ");
            output.push_str(&command.display_name());
            let description = command.localized_description(locale);
            if !description.is_empty() {
                output.push_str(" — ");
                output.push_str(description);
            }
            if !command.branches.is_empty() {
                output.push_str(if is_chinese(locale) {
                    "（含子命令）"
                } else {
                    " (subcommands)"
                });
            }
            output.push('\n');
        }
    }
    output.push_str(if is_chinese(locale) {
        "\n使用 /help <命令> 查看详细说明。"
    } else {
        "\nUse /help <command> for detailed help."
    });
    output
}

fn resolve_branch<'a>(command: &'a Command, path: &[Arc<str>]) -> Option<&'a CommandBranch> {
    let mut children = command.branches.as_slice();
    let mut selected = None;
    for name in path {
        let branch = children.iter().find(|branch| branch.name == *name)?;
        selected = Some(branch);
        children = branch.children.as_slice();
    }
    selected
}

fn render_command_help(
    command: &Command,
    branch_names: &[Arc<str>],
    locale: Option<&str>,
) -> String {
    let usage = command.usage_for(branch_names);
    let selected = resolve_branch(command, branch_names);
    let description = selected.map_or_else(
        || command.localized_description(locale),
        |branch| branch.localized_description(locale),
    );
    let schema = command.schema_for_branch_names(branch_names);
    let children = selected.map_or(command.branches.as_slice(), |branch| {
        branch.children.as_slice()
    });
    let title = if branch_names.is_empty() {
        command.display_name()
    } else {
        format!(
            "{} {}",
            command.display_name(),
            branch_names
                .iter()
                .map(AsRef::as_ref)
                .collect::<Vec<_>>()
                .join(" ")
        )
    };
    let usage_label = if is_chinese(locale) {
        "用法"
    } else {
        "Usage"
    };
    let mut output = format!("{title}\n\n{usage_label}：{usage}");
    if !description.is_empty() {
        output.push_str("\n\n");
        output.push_str(description);
    }
    if branch_names.is_empty() && !command.aliases.is_empty() {
        output.push_str(if is_chinese(locale) {
            "\n\n别名："
        } else {
            "\n\nAliases: "
        });
        output.push_str(
            &command
                .aliases
                .iter()
                .map(AsRef::as_ref)
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    if !children.is_empty() {
        output.push_str(if is_chinese(locale) {
            "\n\n子命令：\n"
        } else {
            "\n\nSubcommands:\n"
        });
        for branch in children.iter().filter(|branch| !branch.hidden) {
            output.push_str("  ");
            output.push_str(&branch.name);
            let branch_description = branch.localized_description(locale);
            if !branch_description.is_empty() {
                output.push_str(" — ");
                output.push_str(branch_description);
            }
            output.push('\n');
        }
    }
    if !schema.arguments.is_empty() {
        output.push_str(if is_chinese(locale) {
            "\n参数：\n"
        } else {
            "\nArguments:\n"
        });
        for argument in &schema.arguments {
            output.push_str("  ");
            output.push_str(&argument.usage_fragment());
            let help = argument.help_text(locale);
            if !help.is_empty() {
                output.push_str(" — ");
                output.push_str(help);
            }
            if let Some(default) = &argument.default {
                if is_chinese(locale) {
                    output.push_str(&format!("（默认：{default}）"));
                } else {
                    output.push_str(&format!(" (default: {default})"));
                }
            }
            if !argument.choices.is_empty() {
                output.push_str(if is_chinese(locale) {
                    "；可选："
                } else {
                    "; choices: "
                });
                output.push_str(
                    &argument
                        .choices
                        .iter()
                        .map(|choice| choice.value.as_ref())
                        .collect::<Vec<_>>()
                        .join(", "),
                );
            }
            output.push('\n');
        }
    }
    if branch_names.is_empty() && !command.examples.is_empty() {
        output.push_str(if is_chinese(locale) {
            "\n示例：\n"
        } else {
            "\nExamples:\n"
        });
        for example in &command.examples {
            output.push_str("  ");
            output.push_str(example);
            output.push('\n');
        }
    }
    output
}
