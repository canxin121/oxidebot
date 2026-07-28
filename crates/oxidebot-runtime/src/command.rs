use oxidebot_core::{
    event::MessageEvent,
    source::message::{File, MessageSegment},
};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::Arc,
    time::Duration,
};
use thiserror::Error;

/// Declares one user-facing command.
#[derive(Clone, Debug)]
pub struct Command {
    name: Arc<str>,
    description: Arc<str>,
    aliases: Vec<Arc<str>>,
    prefixes: Vec<Arc<str>>,
    category: Option<Arc<str>>,
    examples: Vec<Arc<str>>,
    case_sensitive: bool,
    hidden: bool,
    schema: Option<CommandSchema>,
    completion: Option<CompletionConfig>,
}

/// Creates a command using `/` as its prefix.
#[must_use]
pub fn command(name: impl Into<Arc<str>>) -> Command {
    Command::new(name)
}

impl Command {
    #[must_use]
    pub fn new(name: impl Into<Arc<str>>) -> Self {
        Self {
            name: name.into(),
            description: Arc::from(""),
            aliases: Vec::new(),
            prefixes: vec![Arc::from("/")],
            category: None,
            examples: Vec::new(),
            case_sensitive: true,
            hidden: false,
            schema: None,
            completion: None,
        }
    }

    #[must_use]
    pub fn description(mut self, description: impl Into<Arc<str>>) -> Self {
        self.description = description.into();
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
    pub fn schema(mut self, schema: CommandSchema) -> Self {
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
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn description_text(&self) -> &str {
        &self.description
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
    pub fn completion_ref(&self) -> Option<&CompletionConfig> {
        self.completion.as_ref()
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

    pub(crate) fn match_event(&self, event: &MessageEvent) -> Option<CommandResult> {
        let tokens = tokenize_segments(&event.message.segments).ok()?;
        self.match_tokens(tokens)
    }

    fn match_tokens(&self, tokens: Vec<CommandValue>) -> Option<CommandResult> {
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

        let (consumed, invoked_as, prefix) = matched?;
        Some(CommandResult {
            command: self.clone(),
            invoked_as,
            prefix,
            values: tokens.into_iter().skip(consumed).collect::<Vec<_>>().into(),
        })
    }
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
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub const fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    #[must_use]
    pub const fn max_rounds(mut self, max_rounds: usize) -> Self {
        self.max_rounds = max_rounds;
        self
    }

    #[must_use]
    pub fn cancel_words<I, T>(mut self, words: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<Arc<str>>,
    {
        self.cancel_words = words.into_iter().map(Into::into).collect::<Vec<_>>().into();
        self
    }

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

/// Runtime-neutral schema generated by `#[derive(CommandArgs)]` or built by
/// hand for help, parsing, completion, and platform command publication.
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

    pub(crate) fn validate(&self) -> Result<(), String> {
        let mut names = HashSet::new();
        let mut longs = HashSet::new();
        let mut shorts = HashSet::new();
        let mut seen_optional_positional = false;
        let mut seen_variadic = false;
        for argument in &self.arguments {
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
            if argument.flag && argument.multiple {
                return Err(format!("flag `{}` cannot be multiple", argument.name));
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
        }
        Ok(())
    }
}

/// One command argument or option.
#[derive(Clone, Debug)]
pub struct ArgumentSpec {
    name: Arc<str>,
    help: Arc<str>,
    prompt: Option<Arc<str>>,
    value_name: Option<Arc<str>>,
    long: Option<Arc<str>>,
    short: Option<char>,
    required: bool,
    multiple: bool,
    rest: bool,
    flag: bool,
    default: Option<Arc<str>>,
}

impl ArgumentSpec {
    #[must_use]
    pub fn new(name: impl Into<Arc<str>>) -> Self {
        Self {
            name: name.into(),
            help: Arc::from(""),
            prompt: None,
            value_name: None,
            long: None,
            short: None,
            required: true,
            multiple: false,
            rest: false,
            flag: false,
            default: None,
        }
    }

    #[must_use]
    pub fn help(mut self, help: impl Into<Arc<str>>) -> Self {
        self.help = help.into();
        self
    }

    #[must_use]
    pub fn prompt(mut self, prompt: impl Into<Arc<str>>) -> Self {
        self.prompt = Some(prompt.into());
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
        self.multiple || self.rest
    }

    #[must_use]
    pub const fn is_flag(&self) -> bool {
        self.flag
    }

    #[must_use]
    pub fn prompt_text(&self) -> String {
        self.prompt
            .as_ref()
            .map_or_else(|| format!("请输入 {}：", self.name), ToString::to_string)
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
}

/// One lossless command token. Non-text message segments remain typed.
#[derive(Clone, Debug, PartialEq)]
pub enum CommandValue {
    Text(String),
    Mention(String),
    File(File),
    Segment(MessageSegment),
}

impl CommandValue {
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(value) => Some(value),
            Self::Mention(_) | Self::File(_) | Self::Segment(_) => None,
        }
    }

    #[must_use]
    pub fn into_segment(self) -> MessageSegment {
        match self {
            Self::Text(value) => MessageSegment::text(value),
            Self::Mention(user_id) => MessageSegment::at(user_id),
            Self::File(file) => MessageSegment::file(file),
            Self::Segment(segment) => segment,
        }
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

impl FromCommandValue for String {
    fn from_command_value(value: CommandValue) -> Result<Self, CommandParseError> {
        match value {
            CommandValue::Text(value) => Ok(value),
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
            CommandValue::File(file)
            | CommandValue::Segment(MessageSegment::File { file: Some(file) })
            | CommandValue::Segment(MessageSegment::Image { file: Some(file) })
            | CommandValue::Segment(MessageSegment::Video {
                file: Some(file), ..
            })
            | CommandValue::Segment(MessageSegment::Audio {
                file: Some(file), ..
            }) => Ok(file),
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
            other => Err(CommandParseError::UnexpectedValue {
                expected: "mention",
                actual: value_kind(&other),
            }),
        }
    }
}

macro_rules! from_text {
    ($($type:ty),+ $(,)?) => {$ (
        impl FromCommandValue for $type {
            fn from_command_value(value: CommandValue) -> Result<Self, CommandParseError> {
                let text = String::from_command_value(value)?;
                text.parse::<$type>().map_err(|error| CommandParseError::InvalidValue {
                    value: text,
                    expected: std::any::type_name::<$type>(),
                    reason: error.to_string(),
                })
            }
        }
    )+};
}

from_text!(bool, char, i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64);

fn value_kind(value: &CommandValue) -> &'static str {
    match value {
        CommandValue::Text(_) => "text",
        CommandValue::Mention(_) => "mention",
        CommandValue::File(_) => "file",
        CommandValue::Segment(_) => "message segment",
    }
}

/// Parsed values indexed by the stable schema field name.
#[derive(Clone, Debug, Default)]
pub struct ParsedArguments {
    values: HashMap<Arc<str>, Vec<CommandValue>>,
    flags: HashSet<Arc<str>>,
}

impl ParsedArguments {
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.flags.contains(name) || self.values.contains_key(name)
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

    #[must_use]
    pub fn flag(&self, name: &str) -> bool {
        self.flags.contains(name)
    }

    #[must_use]
    pub fn values(&self, name: &str) -> &[CommandValue] {
        self.values.get(name).map_or(&[], Vec::as_slice)
    }
}

/// Raw command match attached to the event context before extractors run.
#[derive(Clone, Debug)]
pub struct CommandResult {
    command: Command,
    invoked_as: Arc<str>,
    prefix: Arc<str>,
    values: Arc<[CommandValue]>,
}

impl CommandResult {
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

    pub fn text_values(&self) -> impl Iterator<Item = &str> {
        self.values.iter().filter_map(CommandValue::as_text)
    }

    pub fn parse_with(&self, schema: &CommandSchema) -> Result<ParsedArguments, CommandParseError> {
        parse_arguments(schema, &self.values)
    }

    pub(crate) fn with_appended(&self, mut values: Vec<CommandValue>) -> Self {
        let mut combined = Vec::with_capacity(self.values.len().saturating_add(values.len()));
        combined.extend(self.values.iter().cloned());
        combined.append(&mut values);
        Self {
            command: self.command.clone(),
            invoked_as: Arc::clone(&self.invoked_as),
            prefix: Arc::clone(&self.prefix),
            values: combined.into(),
        }
    }

    pub(crate) fn with_answer(
        &self,
        schema: &CommandSchema,
        name: &str,
        values: Vec<CommandValue>,
    ) -> Self {
        let Some(spec) = schema.find(name) else {
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

/// Implemented by strongly typed command argument structs.
pub trait CommandArgs: Sized + Send + 'static {
    fn schema() -> CommandSchema;
    fn from_arguments(arguments: &ParsedArguments) -> Result<Self, CommandParseError>;

    fn parse(result: &CommandResult) -> Result<Self, CommandParseError> {
        let arguments = result.parse_with(&Self::schema())?;
        Self::from_arguments(&arguments)
    }

    #[must_use]
    fn command(name: impl Into<Arc<str>>) -> Command {
        Command::new(name).schema(Self::schema())
    }
}

/// Command syntax and type conversion failures.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum CommandParseError {
    #[error("missing required argument `{name}`")]
    MissingArgument { name: Arc<str>, prompt: Arc<str> },
    #[error("unknown option `{option}`{suggestion}")]
    UnknownOption {
        option: Arc<str>,
        suggestion: CompletionSuggestion,
    },
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
}

/// Display helper that avoids storing a formatted suggestion in every error.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CompletionSuggestion(Option<Arc<str>>);

impl CompletionSuggestion {
    fn new(value: Option<Arc<str>>) -> Self {
        Self(value)
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
                        output.flags.insert(Arc::clone(&spec.name));
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
                        let option_values =
                            output.values.entry(Arc::clone(&spec.name)).or_default();
                        if !spec.multiple && !option_values.is_empty() {
                            return Err(CommandParseError::DuplicateOption {
                                option: Arc::from(format!("--{name}")),
                            });
                        }
                        option_values.push(option_value);
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
                                output.flags.insert(Arc::clone(&spec.name));
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
                            let option_values =
                                output.values.entry(Arc::clone(&spec.name)).or_default();
                            if !spec.multiple && !option_values.is_empty() {
                                return Err(CommandParseError::DuplicateOption {
                                    option: Arc::from(format!("-{short}")),
                                });
                            }
                            option_values.push(option_value);
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
                output
                    .values
                    .entry(Arc::clone(&spec.name))
                    .or_default()
                    .extend(positional[value_index..].iter().cloned());
                value_index = positional.len();
            }
        } else if let Some(value) = positional.get(value_index) {
            output
                .values
                .entry(Arc::clone(&spec.name))
                .or_default()
                .push(value.clone());
            value_index += 1;
        }
    }
    if let Some(value) = positional.get(value_index) {
        return Err(CommandParseError::ExtraArgument {
            value: Arc::from(value.as_text().unwrap_or("<message segment>")),
        });
    }

    for spec in &schema.arguments {
        if spec.is_required() && !output.contains(&spec.name) {
            return Err(CommandParseError::MissingArgument {
                name: Arc::clone(&spec.name),
                prompt: Arc::from(spec.prompt_text()),
            });
        }
    }
    Ok(output)
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
            MessageSegment::At { user_id } => output.push(CommandValue::Mention(user_id.clone())),
            MessageSegment::File { file: Some(file) }
            | MessageSegment::Image { file: Some(file) }
            | MessageSegment::Video {
                file: Some(file), ..
            }
            | MessageSegment::Audio {
                file: Some(file), ..
            } => {
                output.push(CommandValue::File(file.clone()));
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

/// Immutable help/catalog view assembled from all registered commands.
#[derive(Clone, Debug, Default)]
pub struct CommandCatalog {
    commands: Arc<[Command]>,
}

impl CommandCatalog {
    #[must_use]
    pub fn new(commands: impl IntoIterator<Item = Command>) -> Self {
        Self {
            commands: commands.into_iter().collect::<Vec<_>>().into(),
        }
    }

    #[must_use]
    pub fn commands(&self) -> &[Command] {
        &self.commands
    }

    #[must_use]
    pub fn find(&self, name: &str) -> Option<&Command> {
        self.commands.iter().find(|command| {
            command.name.eq_ignore_ascii_case(name)
                || command
                    .aliases
                    .iter()
                    .any(|alias| alias.eq_ignore_ascii_case(name))
        })
    }

    #[must_use]
    pub fn render(&self, query: Option<&str>) -> String {
        if let Some(query) = query.filter(|value| !value.trim().is_empty()) {
            return self.find(query.trim().trim_start_matches('/')).map_or_else(
                || format!("没有找到命令 `{query}`。\n\n{}", self.render_index()),
                render_command_help,
            );
        }
        self.render_index()
    }

    fn render_index(&self) -> String {
        let visible = self.commands.iter().filter(|command| !command.hidden);
        let mut groups = HashMap::<Option<Arc<str>>, Vec<&Command>>::new();
        for command in visible {
            groups
                .entry(command.category.clone())
                .or_default()
                .push(command);
        }
        let mut categories = groups.into_iter().collect::<Vec<_>>();
        categories.sort_by(|(left, _), (right, _)| left.cmp(right));
        let mut output = String::from("可用命令\n");
        for (category, mut commands) in categories {
            commands.sort_by(|left, right| left.name.cmp(&right.name));
            if let Some(category) = category {
                output.push_str(&format!("\n{category}\n"));
            }
            for command in commands {
                output.push_str("  ");
                output.push_str(&command.display_name());
                if !command.description.is_empty() {
                    output.push_str(" — ");
                    output.push_str(&command.description);
                }
                output.push('\n');
            }
        }
        output.push_str("\n使用 /help <命令> 查看详细说明。");
        output
    }
}

fn render_command_help(command: &Command) -> String {
    let mut output = format!("{}\n\n用法：{}", command.display_name(), command.usage());
    if !command.description.is_empty() {
        output.push_str("\n\n");
        output.push_str(&command.description);
    }
    if !command.aliases.is_empty() {
        output.push_str("\n\n别名：");
        output.push_str(
            &command
                .aliases
                .iter()
                .map(AsRef::as_ref)
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    if let Some(schema) = &command.schema {
        if !schema.arguments.is_empty() {
            output.push_str("\n\n参数：\n");
            for argument in &schema.arguments {
                output.push_str("  ");
                output.push_str(&argument.usage_fragment());
                if !argument.help.is_empty() {
                    output.push_str(" — ");
                    output.push_str(&argument.help);
                }
                if let Some(default) = &argument.default {
                    output.push_str(&format!("（默认：{default}）"));
                }
                output.push('\n');
            }
        }
    }
    if !command.examples.is_empty() {
        output.push_str("\n示例：\n");
        for example in &command.examples {
            output.push_str("  ");
            output.push_str(example);
            output.push('\n');
        }
    }
    output
}
