//! Declarative command argument and native-option specification.

use super::*;

/// One command argument or option.
#[derive(Clone)]
pub struct ArgumentSpec {
    pub(in crate::command) id: CommandFieldId,
    pub(in crate::command) name: Arc<str>,
    pub(in crate::command) help: LocalizedText,
    pub(in crate::command) heading: Option<LocalizedText>,
    pub(in crate::command) prompt: Option<LocalizedText>,
    pub(in crate::command) value_name: Option<Arc<str>>,
    pub(in crate::command) long: Option<Arc<str>>,
    pub(in crate::command) short: Option<char>,
    pub(in crate::command) required: bool,
    pub(in crate::command) multiple: bool,
    pub(in crate::command) rest: bool,
    pub(in crate::command) flag: bool,
    pub(in crate::command) default: Option<Arc<str>>,
    pub(in crate::command) kind: CommandValueKind,
    pub(in crate::command) action: ArgumentAction,
    pub(in crate::command) choices: Vec<ArgumentChoice>,
    pub(in crate::command) autocomplete: bool,
    pub(in crate::command) min_value: Option<f64>,
    pub(in crate::command) max_value: Option<f64>,
    pub(in crate::command) min_length: Option<u32>,
    pub(in crate::command) max_length: Option<u32>,
    pub(in crate::command) allowed_conversation_kinds: Vec<ConversationKind>,
    pub(in crate::command) requires: Vec<Arc<str>>,
    pub(in crate::command) conflicts: Vec<Arc<str>>,
    pub(in crate::command) validator: Option<ValueValidator>,
    pub(in crate::command) hidden: bool,
}

pub(in crate::command) type ValueValidator =
    Arc<dyn Fn(&CommandValue) -> Result<(), CommandParseError> + Send + Sync>;

impl fmt::Debug for ArgumentSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ArgumentSpec")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("help", &self.help)
            .field("heading", &self.heading)
            .field("prompt", &self.prompt)
            .field("value_name", &self.value_name)
            .field("long", &self.long)
            .field("short", &self.short)
            .field("required", &self.required)
            .field("multiple", &self.multiple)
            .field("rest", &self.rest)
            .field("flag", &self.flag)
            .field("default", &self.default)
            .field("kind", &self.kind)
            .field("action", &self.action)
            .field("choices", &self.choices)
            .field("autocomplete", &self.autocomplete)
            .field("min_value", &self.min_value)
            .field("max_value", &self.max_value)
            .field("min_length", &self.min_length)
            .field("max_length", &self.max_length)
            .field(
                "allowed_conversation_kinds",
                &self.allowed_conversation_kinds,
            )
            .field("requires", &self.requires)
            .field("conflicts", &self.conflicts)
            .field("has_validator", &self.validator.is_some())
            .field("hidden", &self.hidden)
            .finish()
    }
}

impl ArgumentSpec {
    /// Creates a required positional string argument named `name`.
    #[must_use]
    pub fn new(name: impl Into<Arc<str>>) -> Self {
        let name = name.into();
        Self {
            id: CommandFieldId(stable_hash(name.as_bytes()) as u32),
            name,
            help: LocalizedText::default(),
            heading: None,
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
            validator: None,
            hidden: false,
        }
    }

    pub(in crate::command) fn rebase(&mut self, path: &str) {
        self.id = CommandFieldId(stable_hash(format!("{path}.{}", self.name).as_bytes()) as u32);
    }

    /// Sets localized help text for this argument.
    #[must_use]
    pub fn help(mut self, help: impl Into<LocalizedText>) -> Self {
        self.help = help.into();
        self
    }

    /// Places this field under one localized heading in generated help.
    #[must_use]
    pub fn heading(mut self, heading: impl Into<LocalizedText>) -> Self {
        self.heading = Some(heading.into());
        self
    }

    /// Hides this field from generated help and completion while retaining
    /// parse support for compatibility or internal integrations.
    #[must_use]
    pub const fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }

    /// Adds a locale-specific help translation.
    #[must_use]
    pub fn help_translation(
        mut self,
        locale: impl Into<Arc<str>>,
        help: impl Into<Arc<str>>,
    ) -> Self {
        self.help = self.help.translation(locale, help);
        self
    }

    /// Sets the interactive prompt used when this required argument is missing.
    #[must_use]
    pub fn prompt(mut self, prompt: impl Into<LocalizedText>) -> Self {
        self.prompt = Some(prompt.into());
        self
    }

    /// Adds a locale-specific interactive prompt translation.
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

    /// Sets the placeholder name used in usage and help output.
    #[must_use]
    pub fn value_name(mut self, value_name: impl Into<Arc<str>>) -> Self {
        self.value_name = Some(value_name.into());
        self
    }

    /// Sets the long option name without leading dashes.
    #[must_use]
    pub fn long(mut self, long: impl Into<Arc<str>>) -> Self {
        self.long = Some(long.into());
        self
    }

    /// Sets the short option character without a leading dash.
    #[must_use]
    pub const fn short(mut self, short: char) -> Self {
        self.short = Some(short);
        self
    }

    /// Sets whether this argument must be supplied.
    #[must_use]
    pub const fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    /// Allows this argument to collect multiple values.
    #[must_use]
    pub const fn multiple(mut self, multiple: bool) -> Self {
        self.multiple = multiple;
        self
    }

    /// Makes this positional argument consume the remaining input.
    #[must_use]
    pub const fn rest(mut self, rest: bool) -> Self {
        self.rest = rest;
        self
    }

    /// Makes this option a flag and configures its default action.
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

    /// Supplies a default parsed text value and makes the argument optional.
    #[must_use]
    pub fn default_value(mut self, default: impl Into<Arc<str>>) -> Self {
        self.default = Some(default.into());
        self.required = false;
        self
    }

    /// Sets the portable value kind used by parsing and native publication.
    #[must_use]
    pub fn kind(mut self, kind: CommandValueKind) -> Self {
        self.kind = kind;
        self
    }

    /// Sets behavior when an option appears repeatedly.
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

    /// Adds one enumerated value choice.
    #[must_use]
    pub fn choice(mut self, choice: ArgumentChoice) -> Self {
        self.choices.push(choice);
        self
    }

    /// Adds several enumerated value choices.
    #[must_use]
    pub fn with_choices<I>(mut self, choices: I) -> Self
    where
        I: IntoIterator<Item = ArgumentChoice>,
    {
        self.choices.extend(choices);
        self
    }

    /// Enables or disables completion for this argument.
    #[must_use]
    pub const fn autocomplete(mut self, enabled: bool) -> Self {
        self.autocomplete = enabled;
        self
    }

    /// Sets an inclusive minimum numeric value.
    #[must_use]
    pub const fn min_value(mut self, value: f64) -> Self {
        self.min_value = Some(value);
        self
    }

    /// Sets an inclusive maximum numeric value.
    #[must_use]
    pub const fn max_value(mut self, value: f64) -> Self {
        self.max_value = Some(value);
        self
    }

    /// Sets an inclusive minimum text length.
    #[must_use]
    pub const fn min_length(mut self, value: u32) -> Self {
        self.min_length = Some(value);
        self
    }

    /// Sets an inclusive maximum text length.
    #[must_use]
    pub const fn max_length(mut self, value: u32) -> Self {
        self.max_length = Some(value);
        self
    }

    /// Restricts conversation arguments to one conversation kind.
    #[must_use]
    pub fn allowed_conversation_kind(mut self, kind: ConversationKind) -> Self {
        self.allowed_conversation_kinds.push(kind);
        self
    }

    /// Requires another named argument when this argument is supplied.
    #[must_use]
    pub fn requires(mut self, name: impl Into<Arc<str>>) -> Self {
        self.requires.push(name.into());
        self
    }

    /// Declares another named argument incompatible with this one.
    #[must_use]
    pub fn conflicts_with(mut self, name: impl Into<Arc<str>>) -> Self {
        self.conflicts.push(name.into());
        self
    }

    /// Validates this field after conversion but before a command handler runs.
    ///
    /// The validator is synchronous by design. Validation that needs state,
    /// I/O, or authorization should use [`crate::ResolveCommandValue`] in a handler.
    #[must_use]
    pub fn validate<T, F, E>(mut self, validator: F) -> Self
    where
        T: FromCommandValue,
        F: Fn(&T) -> Result<(), E> + Send + Sync + 'static,
        E: fmt::Display + Send + Sync + 'static,
    {
        let name = Arc::clone(&self.name);
        self.validator = Some(Arc::new(move |value| {
            let parsed = T::from_command_value(value.clone())?;
            validator(&parsed).map_err(|error| CommandParseError::InvalidValue {
                value: value
                    .as_text()
                    .map_or_else(|| "non-text command value".to_owned(), ToOwned::to_owned),
                expected: "a valid command value",
                reason: format!("{name}: {error}"),
            })
        }));
        self
    }

    /// Returns this field's stable schema-local ID.
    #[must_use]
    pub const fn id(&self) -> CommandFieldId {
        self.id
    }

    /// Returns the schema field name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the long option name without dashes, if any.
    #[must_use]
    pub fn long_name(&self) -> Option<&str> {
        self.long.as_deref()
    }

    /// Returns the short option character, if any.
    #[must_use]
    pub const fn short_name(&self) -> Option<char> {
        self.short
    }

    /// Returns whether parsing can yield multiple values for this field.
    #[must_use]
    pub const fn is_multiple(&self) -> bool {
        self.multiple || self.rest || matches!(self.action, ArgumentAction::Append)
    }

    /// Returns whether this field is an option flag.
    #[must_use]
    pub const fn is_flag(&self) -> bool {
        self.flag
    }

    /// Returns how repeated option appearances affect parsed values.
    #[must_use]
    pub const fn action_kind(&self) -> ArgumentAction {
        self.action
    }

    /// Returns the portable value kind expected for this field.
    #[must_use]
    pub fn value_kind(&self) -> &CommandValueKind {
        &self.kind
    }

    /// Returns declared enumerated choices.
    #[must_use]
    pub fn choices(&self) -> &[ArgumentChoice] {
        &self.choices
    }

    /// Returns whether this field requests platform completion support.
    #[must_use]
    pub const fn is_autocomplete(&self) -> bool {
        self.autocomplete
    }

    /// Resolves help text for `locale`.
    #[must_use]
    pub fn help_text(&self, locale: Option<&str>) -> &str {
        self.help.resolve(locale)
    }

    /// Resolves this field's optional help heading.
    #[must_use]
    pub fn heading_text(&self, locale: Option<&str>) -> Option<&str> {
        self.heading.as_ref().map(|heading| heading.resolve(locale))
    }

    /// Returns whether this field is hidden from discovery surfaces.
    #[must_use]
    pub const fn is_hidden(&self) -> bool {
        self.hidden
    }

    /// Returns the default interactive prompt for this field.
    #[must_use]
    pub fn prompt_text(&self) -> String {
        self.prompt_text_for(None)
    }

    /// Returns the localized interactive prompt for this field.
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

    /// Returns whether this field is required after flag and default rules.
    #[must_use]
    pub fn is_required(&self) -> bool {
        self.required && self.default.is_none() && !self.flag
    }

    /// Returns whether this field has neither long nor short option spelling.
    #[must_use]
    pub fn is_positional(&self) -> bool {
        self.long.is_none() && self.short.is_none()
    }

    pub(in crate::command) fn usage_fragment(&self) -> String {
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

    pub(in crate::command) fn native_option(&self) -> CommandOption {
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
