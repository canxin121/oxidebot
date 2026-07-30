//! Command completion configuration, candidates, and scanning.
//!
//! Completion stays independent of command catalog rendering and text parsing
//! while sharing the immutable command schema through the parent module.

use super::*;

/// Interactive recovery policy for missing required arguments.
#[derive(Clone, Debug)]
pub struct CompletionConfig {
    /// Maximum time to wait for one interactive answer.
    pub timeout: Duration,
    /// Maximum retry rounds for one incomplete invocation.
    pub max_rounds: usize,
    /// Maximum invalid replies accepted for one field before the form stops.
    pub max_attempts_per_field: usize,
    /// Whether invalid values are explained and the same field is asked again.
    pub retry_invalid: bool,
    /// Case-insensitive words that cancel interactive recovery.
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

    /// Sets the maximum invalid replies accepted for one field.
    #[must_use]
    pub const fn max_attempts_per_field(mut self, attempts: usize) -> Self {
        self.max_attempts_per_field = attempts;
        self
    }

    /// Enables or disables retrying the same field after a recoverable parse
    /// error such as an invalid choice or an out-of-range number.
    #[must_use]
    pub const fn retry_invalid(mut self, enabled: bool) -> Self {
        self.retry_invalid = enabled;
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
            max_rounds: 12,
            max_attempts_per_field: 3,
            retry_invalid: true,
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

/// Semantic category used to render a completion item.
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
    /// Creates a completion with text display equal to its inserted value.
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

    /// Attaches rich explanatory text to this completion.
    #[must_use]
    pub fn description(mut self, description: impl Into<Message>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Associates this completion with the field that requested it.
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

pub(super) fn cursor_prefix(input: &str, cursor: usize) -> (&str, SourceSpan) {
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

pub(super) fn branch_children<'a>(command: &'a Command, path: &[Arc<str>]) -> &'a [CommandBranch] {
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

pub(super) fn suggest_for_command(
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
