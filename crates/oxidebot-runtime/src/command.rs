use crate::Shortcut;
use oxidebot_core::{
    application::{
        CommandChoice, CommandContext, CommandDefinition, CommandInvocation, CommandKind,
        CommandOption, CommandOptionType, Localized, Suggestion,
    },
    conversation::{ConversationKind, ConversationRef},
    source::{
        message::{File, Message, MessageSegment},
        user::User,
    },
    FormValue,
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt,
    sync::Arc,
    time::Duration,
};
use thiserror::Error;

#[path = "command_parse.rs"]
mod command_parse;

#[path = "command_render.rs"]
mod command_render;

#[path = "command_catalog.rs"]
mod command_catalog;

#[path = "command_definition.rs"]
mod command_definition;

#[path = "command_completion.rs"]
mod command_completion;

#[path = "command_schema.rs"]
mod command_schema;

pub use command_catalog::CommandCatalog;
use command_completion::{branch_children, cursor_prefix, suggest_for_command};
pub use command_completion::{CompletionConfig, CompletionItem, CompletionKind, SourceSpan};
pub use command_definition::{
    command, ArgumentAction, ArgumentChoice, ArgumentGroup, Command, CommandBranch, CommandEnum,
    CommandValueKind, LocalizedText,
};
use command_definition::{normalize_native_name, same_word, stable_hash};
use command_parse::{
    branch_choice_list, parse_arguments, parse_native_arguments, unknown_subcommand,
    validate_argument_value,
};
pub use command_parse::{tokenize_segments, tokenize_text};
pub use command_render::{
    CatalogCommandRenderer, CommandOutput, CommandRenderer, DefaultCommandRenderer,
};
pub use command_schema::{ArgumentSpec, CommandSchema};

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

/// One lossless command token. Non-text message segments and native form
/// values remain typed until a field requests conversion.
// `MessageSegment` remains inline to preserve the ergonomic public value API.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum CommandValue {
    /// Text token from command input.
    Text(String),
    /// Mention token retaining its user ID.
    Mention(oxidebot_core::UserId),
    /// File token retaining its portable descriptor.
    File(File),
    /// Rich message segment supplied as a command token.
    Segment(MessageSegment),
    /// Typed value supplied by a native platform invocation.
    Form(FormValue),
}

impl CommandValue {
    /// Borrows text when this value is textual.
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(value) | Self::Form(FormValue::Text(value)) => Some(value),
            Self::Mention(_) | Self::File(_) | Self::Segment(_) | Self::Form(_) => None,
        }
    }

    /// Converts this value to a canonical rich message segment.
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
        FormValue::Conversation(value) => value.id.to_string(),
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
    /// Converts one lossless command value into this typed argument value.
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
            CommandValue::Mention(user_id) => Ok(Self::Text(user_id.to_string())),
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
pub struct Mention(pub oxidebot_core::UserId);

impl FromCommandValue for Mention {
    fn from_command_value(value: CommandValue) -> Result<Self, CommandParseError> {
        match value {
            CommandValue::Mention(user_id) => Ok(Self(user_id)),
            CommandValue::Segment(MessageSegment::At { user_id }) => Ok(Self(user_id)),
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
    /// Returns whether the named field was supplied.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.present.contains(name)
    }

    /// Returns whether the field with `id` was supplied.
    #[must_use]
    pub fn contains_id(&self, id: CommandFieldId) -> bool {
        self.present_ids.contains(&id)
    }

    /// Converts the first value of a required named field.
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

    /// Converts the first value of a required field identified by `id`.
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

    /// Converts the first named field value when it is present.
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

    /// Converts the first field value by stable ID when it is present.
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

    /// Converts every value of a named field in input order.
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

    /// Converts every value of a field by stable ID in input order.
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

    /// Returns a named boolean flag's value.
    #[must_use]
    pub fn flag(&self, name: &str) -> bool {
        self.flags.contains(name)
    }

    /// Returns a boolean flag's value by stable field ID.
    #[must_use]
    pub fn flag_id(&self, id: CommandFieldId) -> bool {
        self.flag_ids.contains(&id)
    }

    /// Returns the count action result for a named field.
    #[must_use]
    pub fn count(&self, name: &str) -> u32 {
        self.counts.get(name).copied().unwrap_or_default()
    }

    /// Returns the count action result by stable field ID.
    #[must_use]
    pub fn count_id(&self, id: CommandFieldId) -> u32 {
        self.counts_by_id.get(&id).copied().unwrap_or_default()
    }

    /// Borrows every lossless value of a named field.
    #[must_use]
    pub fn values(&self, name: &str) -> &[CommandValue] {
        self.values.get(name).map_or(&[], Vec::as_slice)
    }

    /// Borrows every lossless value of a field by stable ID.
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
        for group in schema.groups() {
            let present = group
                .arguments_ref()
                .iter()
                .filter(|name| self.contains(name))
                .cloned()
                .collect::<Vec<_>>();
            if group.required && present.is_empty() {
                return Err(CommandParseError::MissingArgumentGroup {
                    group: Arc::clone(&group.name),
                    members: group.arguments_ref().to_vec().into(),
                });
            }
            if !group.multiple && present.len() > 1 {
                return Err(CommandParseError::ArgumentGroupConflict {
                    group: Arc::clone(&group.name),
                    members: present.into(),
                });
            }
        }
        Ok(())
    }
}

/// Where a command match originated.
#[derive(Clone, Debug)]
pub enum CommandSource {
    /// Command text was tokenized from a canonical message.
    Text,
    /// Adapter provided a typed native command invocation.
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
    /// Returns the matched command definition.
    #[must_use]
    pub fn command(&self) -> &Command {
        &self.command
    }

    /// Returns the root name or alias used by the caller.
    #[must_use]
    pub fn invoked_as(&self) -> &str {
        &self.invoked_as
    }

    /// Returns the textual prefix used by the caller.
    #[must_use]
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Returns lossless tokens after command and branch names.
    #[must_use]
    pub fn values(&self) -> &[CommandValue] {
        &self.values
    }

    /// Returns the active argument schema.
    #[must_use]
    pub fn schema(&self) -> &CommandSchema {
        &self.schema
    }

    /// Returns cached parsed arguments when parsing has already happened.
    #[must_use]
    pub fn arguments(&self) -> Option<&ParsedArguments> {
        self.parsed.as_deref()
    }

    /// Returns whether this match came from text or a native invocation.
    #[must_use]
    pub fn source(&self) -> &CommandSource {
        &self.source
    }

    /// Returns the adapter-reported locale, if present.
    #[must_use]
    pub fn locale(&self) -> Option<&str> {
        self.locale.as_deref()
    }

    pub(crate) fn with_locale(mut self, locale: Option<Arc<str>>) -> Self {
        self.locale = locale;
        self
    }

    /// Returns stable IDs for selected command-tree branches.
    #[must_use]
    pub fn branch_path(&self) -> &[CommandNodeId] {
        &self.branch_path
    }

    /// Returns selected command-tree branch names.
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

    /// Returns the deepest selected branch name, if a branch was selected.
    #[must_use]
    pub fn selected_branch_name(&self) -> Option<&str> {
        self.branch_names.last().map(AsRef::as_ref)
    }

    /// Returns whether the selected branch path equals `path` case-insensitively.
    #[must_use]
    pub fn is_branch(&self, path: &[&str]) -> bool {
        self.branch_names.len() == path.len()
            && self
                .branch_names
                .iter()
                .zip(path)
                .all(|(actual, expected)| actual.eq_ignore_ascii_case(expected))
    }

    /// Returns the active interactive recovery policy, if configured.
    #[must_use]
    pub fn completion_ref(&self) -> Option<&CompletionConfig> {
        self.completion.as_ref()
    }

    /// Iterates only textual lossless values in the invocation.
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

    /// Parses invocation values with an explicit schema.
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

    /// Parses invocation values with the selected command or branch schema.
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

    pub(crate) fn with_subcommand_answer(&self, mut values: Vec<CommandValue>) -> Option<Self> {
        let mut tokens = Vec::with_capacity(
            self.values
                .len()
                .saturating_add(values.len())
                .saturating_add(self.branch_names.len())
                .saturating_add(2),
        );
        let mut command_path = self.invoked_as.split_whitespace();
        let root = command_path.next()?;
        tokens.push(CommandValue::Text(format!("{}{}", self.prefix, root)));
        tokens.extend(command_path.map(|name| CommandValue::Text(name.to_owned())));
        tokens.extend(
            self.branch_names
                .iter()
                .map(|name| CommandValue::Text(name.to_string())),
        );
        tokens.append(&mut values);
        tokens.extend(self.values.iter().cloned());
        self.command.match_tokens(tokens)
    }
}

/// Converts the shared structured match into one handler argument.
pub trait FromCommandMatch: Sized + Send + 'static {
    /// Converts one structured command match into this handler argument type.
    fn from_match(result: &CommandMatch) -> Result<Self, CommandParseError>;
}

/// Implemented by strongly typed flat command argument structs.
pub trait CommandArgs: Sized + Send + 'static {
    /// Returns the static schema for this flat argument struct.
    fn schema() -> CommandSchema;
    /// Builds this struct from parsed values.
    fn from_arguments(arguments: &ParsedArguments) -> Result<Self, CommandParseError>;

    /// Parses a command match using this type's static schema.
    fn parse(result: &CommandMatch) -> Result<Self, CommandParseError> {
        let arguments = result.parse_with(&Self::schema())?;
        Self::from_arguments(&arguments)
    }

    /// Creates a command named `name` bound to this type's schema.
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
    /// Returns the complete typed command-tree grammar.
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
    /// Input did not match this command.
    NotMatched {
        /// Canonical command name expected by the parser.
        command: Arc<str>,
    },
    #[error("missing required subcommand; expected one of: {choices}")]
    /// A command tree requires a branch name.
    MissingSubcommand {
        /// Human-readable available branch choices.
        choices: Arc<str>,
    },
    #[error("unknown subcommand `{value}`{suggestion}; expected one of: {choices}")]
    /// A supplied branch name is not part of the command grammar.
    UnknownSubcommand {
        /// Unrecognized branch text.
        value: Arc<str>,
        /// Closest matching branch suggestion, if any.
        suggestion: CompletionSuggestion,
        /// Human-readable available branch choices.
        choices: Arc<str>,
    },
    #[error("missing required argument `{name}`")]
    /// A required argument was absent.
    MissingArgument {
        /// Schema name of the missing argument.
        name: Arc<str>,
        /// Localized prompt suitable for interactive recovery.
        prompt: Arc<str>,
    },
    #[error("one of argument group `{group}` is required; expected one of: {members:?}")]
    /// A required group did not receive any member.
    MissingArgumentGroup {
        /// Group that requires one member.
        group: Arc<str>,
        /// Eligible member fields.
        members: Arc<[Arc<str>]>,
    },
    #[error("argument group `{group}` accepts only one member; received: {members:?}")]
    /// More than one mutually exclusive group member was supplied.
    ArgumentGroupConflict {
        /// Mutually exclusive group.
        group: Arc<str>,
        /// Supplied member fields.
        members: Arc<[Arc<str>]>,
    },
    #[error("missing schema field {id:?}")]
    /// A required field ID was absent.
    MissingFieldId {
        /// Stable ID of the missing schema field.
        id: CommandFieldId,
    },
    #[error("unknown option `{option}`{suggestion}")]
    /// A named text option is unknown to the active schema.
    UnknownOption {
        /// Unrecognized option spelling.
        option: Arc<str>,
        /// Closest matching option suggestion, if any.
        suggestion: CompletionSuggestion,
    },
    #[error("unknown native option `{option}`")]
    /// A native platform option does not map to the active schema.
    UnknownNativeOption {
        /// Unrecognized native option name.
        option: Arc<str>,
    },
    #[error("option `{option}` requires a value")]
    /// A value-taking option was supplied without a value.
    MissingOptionValue {
        /// Option spelling that needs a following value.
        option: Arc<str>,
    },
    #[error("option `{option}` was provided more than once")]
    /// A non-repeatable option was supplied more than once.
    DuplicateOption {
        /// Option spelling that was repeated.
        option: Arc<str>,
    },
    #[error("unexpected extra argument `{value}`")]
    /// Input contained a positional value with no accepting field.
    ExtraArgument {
        /// Unexpected input value.
        value: Arc<str>,
    },
    #[error("expected {expected}, received {actual}")]
    /// A typed command value has the wrong kind.
    UnexpectedValue {
        /// Expected portable kind.
        expected: &'static str,
        /// Actual portable kind.
        actual: &'static str,
    },
    #[error("could not parse `{value}` as {expected}: {reason}")]
    /// Text conversion into a requested Rust type failed.
    InvalidValue {
        /// Input value that could not be converted.
        value: String,
        /// Expected Rust or portable type.
        expected: &'static str,
        /// Conversion failure detail.
        reason: String,
    },
    #[error("`{argument}` must be one of: {choices}")]
    /// A value is not in an argument's declared choices.
    InvalidChoice {
        /// Argument whose choice validation failed.
        argument: Arc<str>,
        /// Human-readable allowed choices.
        choices: Arc<str>,
    },
    #[error("`{argument}` is outside the accepted range")]
    /// A value violates configured numeric bounds.
    OutOfRange {
        /// Argument violating numeric bounds.
        argument: Arc<str>,
    },
    #[error("`{argument}` has length {actual}, expected {expected}")]
    /// A value violates configured text-length bounds.
    InvalidLength {
        /// Argument violating length bounds.
        argument: Arc<str>,
        /// Observed text length.
        actual: usize,
        /// Human-readable accepted length range.
        expected: Arc<str>,
    },
    #[error("`{argument}` requires `{required}`")]
    /// A supplied argument requires another argument.
    Requires {
        /// Supplied argument with an unmet requirement.
        argument: Arc<str>,
        /// Required companion argument.
        required: Arc<str>,
    },
    #[error("`{left}` conflicts with `{right}`")]
    /// Two mutually exclusive arguments were supplied together.
    Conflict {
        /// First conflicting argument.
        left: Arc<str>,
        /// Second conflicting argument.
        right: Arc<str>,
    },
    #[error("unterminated quote in command")]
    /// Text tokenization ended while a quote was still open.
    UnterminatedQuote,
    #[error("interactive command completion was cancelled")]
    /// Interactive recovery was cancelled by the caller.
    Cancelled,
    #[error("interactive command completion exceeded its configured rounds")]
    /// Interactive recovery used all configured rounds without valid input.
    CompletionExhausted,
}

impl CommandParseError {
    /// Returns the localized prompt for a missing required argument.
    #[must_use]
    pub fn missing_prompt(&self) -> Option<&str> {
        match self {
            Self::MissingArgument { prompt, .. } => Some(prompt),
            _ => None,
        }
    }

    /// Returns the missing required argument name, when applicable.
    #[must_use]
    pub fn missing_name(&self) -> Option<&str> {
        match self {
            Self::MissingArgument { name, .. } => Some(name),
            _ => None,
        }
    }

    /// Returns a localized human-readable error message.
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
            Self::MissingArgumentGroup { group, members } => format!(
                "参数组 `{group}` 至少需要提供以下字段之一：{}",
                members
                    .iter()
                    .map(AsRef::as_ref)
                    .collect::<Vec<_>>()
                    .join("、")
            ),
            Self::ArgumentGroupConflict { group, members } => format!(
                "参数组 `{group}` 只能提供一个字段，但同时提供了：{}",
                members
                    .iter()
                    .map(AsRef::as_ref)
                    .collect::<Vec<_>>()
                    .join("、")
            ),
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

    /// Returns the suggested replacement text, if an edit-distance match exists.
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
