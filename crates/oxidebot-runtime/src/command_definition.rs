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
    pub(super) fn native_kind(&self) -> CommandOptionType {
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

    pub(super) fn matches(&self, value: &str) -> bool {
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
    pub(super) name: Arc<str>,
    pub(super) arguments: Vec<Arc<str>>,
    pub(super) required: bool,
    pub(super) multiple: bool,
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

/// A true node in the command grammar. Unlike a Web route mount, this models
/// user-visible subcommand syntax and is reused by parsing, help, completion,
/// and native command publication.
#[derive(Clone, Debug)]
pub struct CommandBranch {
    pub(super) id: CommandNodeId,
    pub(super) name: Arc<str>,
    pub(super) description: LocalizedText,
    pub(super) aliases: Vec<Arc<str>>,
    pub(super) schema: Option<CommandSchema>,
    pub(super) children: Vec<CommandBranch>,
    pub(super) completion: Option<CompletionConfig>,
    pub(super) hidden: bool,
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

    pub(super) fn matches(&self, value: &str, case_sensitive: bool) -> bool {
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
            if completion.timeout.is_zero()
                || completion.max_rounds == 0
                || completion.max_attempts_per_field == 0
            {
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

pub(super) fn stable_hash(bytes: &[u8]) -> u64 {
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
    pub(super) id: CommandId,
    pub(super) name: Arc<str>,
    pub(super) description: LocalizedText,
    pub(super) aliases: Vec<Arc<str>>,
    pub(super) prefixes: Vec<Arc<str>>,
    pub(super) category: Option<Arc<str>>,
    pub(super) examples: Vec<Arc<str>>,
    pub(super) case_sensitive: bool,
    pub(super) hidden: bool,
    pub(super) global_schema: Option<CommandSchema>,
    pub(super) schema: Option<CommandSchema>,
    pub(super) branches: Vec<CommandBranch>,
    pub(super) completion: Option<CompletionConfig>,
    pub(super) shortcuts: Vec<Shortcut>,
}

/// Creates a command using `/` as its prefix.
#[must_use]
pub fn command(name: impl Into<Arc<str>>) -> Command {
    Command::new(name)
}

impl Command {
    /// Creates a command with the default `/` prefix.
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
            global_schema: None,
            schema: None,
            branches: Vec::new(),
            completion: None,
            shortcuts: Vec::new(),
        }
    }

    /// Sets the default localized command description.
    #[must_use]
    pub fn description(mut self, description: impl Into<LocalizedText>) -> Self {
        self.description = description.into();
        self
    }

    /// Adds a locale-specific command description.
    #[must_use]
    pub fn description_translation(
        mut self,
        locale: impl Into<Arc<str>>,
        description: impl Into<Arc<str>>,
    ) -> Self {
        self.description = self.description.translation(locale, description);
        self
    }

    /// Adds a parseable command alias.
    #[must_use]
    pub fn alias(mut self, alias: impl Into<Arc<str>>) -> Self {
        self.aliases.push(alias.into());
        self
    }

    /// Adds several parseable command aliases.
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

    /// Replaces accepted prefixes with one prefix.
    #[must_use]
    pub fn prefix(self, prefix: impl Into<Arc<str>>) -> Self {
        self.prefixes([prefix])
    }

    /// Enables matching command text without a prefix.
    #[must_use]
    pub fn no_prefix(self) -> Self {
        self.prefix(Arc::<str>::from(""))
    }

    /// Enables case-insensitive matching for names and aliases.
    #[must_use]
    pub const fn case_insensitive(mut self) -> Self {
        self.case_sensitive = false;
        self
    }

    /// Hides this command from help and completion while retaining parsing.
    #[must_use]
    pub const fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }

    /// Assigns a presentation category to the command.
    #[must_use]
    pub fn category(mut self, category: impl Into<Arc<str>>) -> Self {
        self.category = Some(category.into());
        self
    }

    /// Adds one usage example for help renderers.
    #[must_use]
    pub fn example(mut self, example: impl Into<Arc<str>>) -> Self {
        self.examples.push(example.into());
        self
    }

    /// Attaches an argument schema to the root command.
    #[must_use]
    pub fn schema(mut self, mut schema: CommandSchema) -> Self {
        schema.rebase(&self.name);
        self.schema = Some(schema);
        self
    }

    /// Attaches arguments inherited by every selected subcommand branch.
    ///
    /// Global arguments are deliberately separate from root arguments: a
    /// command tree can have global options and subcommands, while a leaf
    /// command continues to use [`Self::schema`].
    #[must_use]
    pub fn global_schema(mut self, mut schema: CommandSchema) -> Self {
        schema.rebase(&format!("{}.__global", self.name));
        self.global_schema = Some(schema);
        self
    }

    /// Attaches global arguments generated by [`CommandArgs`].
    #[must_use]
    pub fn global_args<T>(self) -> Self
    where
        T: CommandArgs,
    {
        self.global_schema(T::schema())
    }

    /// Attaches the schema generated by `CommandArgs`.
    #[must_use]
    pub fn args<T>(self) -> Self
    where
        T: CommandArgs,
    {
        self.schema(T::schema())
    }

    /// Enables interactive recovery for missing required root arguments.
    #[must_use]
    pub fn completion(mut self, completion: CompletionConfig) -> Self {
        self.completion = Some(completion);
        self
    }

    /// Adds a static shortcut that expands to this command.
    #[must_use]
    pub fn shortcut(mut self, shortcut: Shortcut) -> Self {
        self.shortcuts.push(shortcut);
        self
    }

    /// Returns static shortcuts declared for this command.
    #[must_use]
    pub fn shortcuts(&self) -> &[Shortcut] {
        &self.shortcuts
    }

    /// Adds a user-visible subcommand branch.
    #[must_use]
    pub fn subcommand(mut self, mut branch: CommandBranch) -> Self {
        branch.rebase(&format!("{}.{}", self.name, branch.name));
        self.branches.push(branch);
        self
    }

    /// Adds a subcommand branch with schema generated by `CommandArgs`.
    #[must_use]
    pub fn subcommand_args<T>(self, name: impl Into<Arc<str>>) -> Self
    where
        T: CommandArgs,
    {
        self.subcommand(CommandBranch::new(name).schema(T::schema()))
    }

    /// Returns this command's stable deterministic ID.
    #[must_use]
    pub const fn id(&self) -> CommandId {
        self.id
    }

    /// Returns the canonical root command name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the default command description.
    #[must_use]
    pub fn description_text(&self) -> &str {
        self.description.resolve(None)
    }

    /// Resolves the command description for `locale`.
    #[must_use]
    pub fn localized_description(&self, locale: Option<&str>) -> &str {
        self.description.resolve(locale)
    }

    /// Returns all parseable aliases.
    #[must_use]
    pub fn aliases_list(&self) -> &[Arc<str>] {
        &self.aliases
    }

    /// Returns all accepted command prefixes.
    #[must_use]
    pub fn prefixes_list(&self) -> &[Arc<str>] {
        &self.prefixes
    }

    /// Returns the root argument schema, if any.
    #[must_use]
    pub fn schema_ref(&self) -> Option<&CommandSchema> {
        self.schema.as_ref()
    }

    /// Returns arguments inherited by every subcommand branch, if configured.
    #[must_use]
    pub fn global_schema_ref(&self) -> Option<&CommandSchema> {
        self.global_schema.as_ref()
    }

    /// Returns direct user-visible subcommand branches.
    #[must_use]
    pub fn branches(&self) -> &[CommandBranch] {
        &self.branches
    }

    /// Returns the root interactive recovery policy, if configured.
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

    /// Returns whether this command is hidden from discovery surfaces.
    #[must_use]
    pub fn is_hidden(&self) -> bool {
        self.hidden
    }

    /// Returns the display name formed from the first prefix and canonical name.
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
            "aliases={:?};prefixes={:?};case={};global_schema={:?};schema={:?};branches={:?};completion={:?};shortcuts={:?}",
            self.aliases,
            self.prefixes,
            self.case_sensitive,
            self.global_schema,
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

    /// Builds root command usage text from its active schema.
    #[must_use]
    pub fn usage(&self) -> String {
        let mut usage = self.display_name();
        if let Some(schema) = &self.schema {
            for argument in schema.arguments() {
                if argument.is_hidden() {
                    continue;
                }
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
            if completion.max_rounds == 0 || completion.max_attempts_per_field == 0 {
                return Err(format!(
                    "command `{}` has an invalid completion attempt limit",
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
        if let Some(schema) = &self.global_schema {
            schema.validate()?;
            if schema.arguments().iter().any(ArgumentSpec::is_positional) {
                return Err(format!(
                    "command `{}` global arguments must be named options or flags",
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
            if let Some(global_schema) = &self.global_schema {
                validate_global_schema_for_branch(global_schema, branch, &self.name)?;
            }
        }

        let mut node_ids = HashMap::<CommandNodeId, String>::new();
        let mut field_ids = HashMap::<CommandFieldId, String>::new();
        if let Some(schema) = &self.global_schema {
            collect_schema_field_ids(schema, &format!("{}.__global", self.name), &mut field_ids)?;
        }
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

    pub(super) fn match_tokens(&self, tokens: Vec<CommandValue>) -> Option<CommandMatch> {
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
        let mut schema = self.global_schema.clone().unwrap_or_default();
        if let Some(root_schema) = &self.schema {
            schema = schema.merged(root_schema);
        }
        let mut branch_path = Vec::new();
        let mut branch_names = Vec::new();
        let mut completion = self.completion.clone();
        let mut children = self.branches.as_slice();
        let mut values = Vec::new();

        while let Some(actual) = tokens.get(consumed).and_then(CommandValue::as_text) {
            if let Some(branch) = children
                .iter()
                .find(|branch| branch.matches(actual, self.case_sensitive))
            {
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
                continue;
            }
            let Some(global_schema) = &self.global_schema else {
                break;
            };
            let Some(width) = global_option_width(global_schema, &tokens, consumed) else {
                break;
            };
            let end = consumed.saturating_add(width).min(tokens.len());
            values.extend(tokens[consumed..end].iter().cloned());
            consumed = end;
        }
        values.extend(tokens.into_iter().skip(consumed));

        Some(CommandMatch {
            command: self.clone(),
            invoked_as,
            prefix,
            branch_path: branch_path.into(),
            branch_names: branch_names.into(),
            values: values.into(),
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

        let mut schema = self.global_schema.clone().unwrap_or_default();
        if let Some(root_schema) = &self.schema {
            schema = schema.merged(root_schema);
        }
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

    /// Converts this command grammar into a portable native command definition.
    #[must_use]
    pub fn definition(&self) -> CommandDefinition {
        let mut options = self
            .global_schema
            .as_ref()
            .map_or_else(Vec::new, CommandSchema::native_options);
        if let Some(schema) = &self.schema {
            options.extend(schema.native_options());
        }
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

    /// Suggests branch, option, argument, and choice completions for input.
    #[must_use]
    pub fn suggest(&self, input: &str, cursor: usize, locale: Option<&str>) -> Vec<CompletionItem> {
        suggest_for_command(self, input, cursor, locale)
    }

    /// Builds usage text for a selected command-tree branch path.
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
            if argument.is_hidden() {
                continue;
            }
            usage.push(' ');
            usage.push_str(&argument.usage_fragment());
        }
        usage
    }

    pub(super) fn schema_for_branch_names(&self, branch_names: &[Arc<str>]) -> CommandSchema {
        let mut schema = self.global_schema.clone().unwrap_or_default();
        if let Some(root_schema) = &self.schema {
            schema = schema.merged(root_schema);
        }
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

fn validate_global_schema_for_branch(
    global: &CommandSchema,
    branch: &CommandBranch,
    root_name: &str,
) -> Result<(), String> {
    if let Some(schema) = &branch.schema {
        global.merged(schema).validate().map_err(|error| {
            format!(
                "command `{root_name}` global arguments conflict with branch `{}`: {error}",
                branch.name
            )
        })?;
    }
    for child in &branch.children {
        validate_global_schema_for_branch(global, child, root_name)?;
    }
    Ok(())
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

pub(super) fn normalize_native_name(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
        .replace('_', "-")
}
pub(super) fn same_word(actual: &str, expected: &str, case_sensitive: bool) -> bool {
    if case_sensitive {
        actual == expected
    } else {
        actual.eq_ignore_ascii_case(expected)
    }
}

/// Returns the number of tokens occupied by one global named option while a
/// command tree is still looking for its next subcommand. The caller retains
/// those tokens for ordinary argument parsing after the branch path is known.
fn global_option_width(
    schema: &CommandSchema,
    tokens: &[CommandValue],
    index: usize,
) -> Option<usize> {
    let text = tokens.get(index)?.as_text()?;
    if let Some(option) = text.strip_prefix("--").filter(|option| !option.is_empty()) {
        let (name, attached) = option
            .split_once('=')
            .map_or((option, None), |(name, value)| (name, Some(value)));
        let spec = schema
            .arguments()
            .iter()
            .find(|argument| argument.long_name() == Some(name))?;
        return Some(if spec.is_flag() || attached.is_some() {
            1
        } else {
            2
        });
    }
    let shorts = text.strip_prefix('-').filter(|shorts| !shorts.is_empty())?;
    if text.parse::<f64>().is_ok() {
        return None;
    }
    let characters = shorts.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    while let Some(short) = characters.get(index) {
        let spec = schema
            .arguments()
            .iter()
            .find(|argument| argument.short_name() == Some(*short))?;
        if !spec.is_flag() {
            return Some(if index + 1 == characters.len() { 2 } else { 1 });
        }
        index += 1;
    }
    Some(1)
}
