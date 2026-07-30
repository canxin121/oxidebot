//! Text and segment tokenization for command invocations.

use super::{
    form_value_label, ArgumentSpec, CommandBranch, CommandParseError, CommandSchema, CommandValue,
    CompletionSuggestion,
};
use oxidebot_core::source::message::MessageSegment;
use std::sync::Arc;

pub(super) fn validate_argument_value(
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
    if !spec.choices.is_empty()
        && !text
            .as_deref()
            .is_some_and(|text| spec.choices.iter().any(|choice| choice.matches(text)))
    {
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
    if let Some(validator) = &spec.validator {
        validator(value)?;
    }
    Ok(())
}

pub(super) fn unknown_subcommand(
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

pub(super) fn unknown_option(schema: &CommandSchema, option: String) -> CommandParseError {
    let suggestion = schema
        .arguments
        .iter()
        .flat_map(|argument| {
            argument
                .long
                .as_ref()
                .map(|long| format!("--{long}"))
                .into_iter()
                .chain(argument.short.map(|short| format!("-{short}")))
        })
        .map(|candidate| (edit_distance(&option, &candidate), candidate))
        .filter(|(distance, _)| *distance <= 3)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, value)| Arc::from(value));
    CommandParseError::UnknownOption {
        option: Arc::from(option),
        suggestion: CompletionSuggestion::new(suggestion),
    }
}

pub(super) fn branch_choice_list(children: &[CommandBranch]) -> Arc<str> {
    Arc::from(
        children
            .iter()
            .filter(|branch| !branch.hidden)
            .map(|branch| branch.name.as_ref())
            .collect::<Vec<_>>()
            .join(", "),
    )
}

pub(super) fn edit_distance(left: &str, right: &str) -> usize {
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
