//! Structured command matches shared by text and native invocation paths.

use super::*;

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
    pub(in crate::command) command: Command,
    pub(in crate::command) invoked_as: Arc<str>,
    pub(in crate::command) prefix: Arc<str>,
    pub(in crate::command) branch_path: Arc<[CommandNodeId]>,
    pub(in crate::command) branch_names: Arc<[Arc<str>]>,
    pub(in crate::command) values: Arc<[CommandValue]>,
    pub(in crate::command) schema: Arc<CommandSchema>,
    pub(in crate::command) parsed: Option<Arc<ParsedArguments>>,
    pub(in crate::command) completion: Option<CompletionConfig>,
    pub(in crate::command) source: CommandSource,
    pub(in crate::command) locale: Option<Arc<str>>,
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
