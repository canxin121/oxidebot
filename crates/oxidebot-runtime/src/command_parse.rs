//! Text and segment tokenization for command invocations.

use super::{form_value_label, ArgumentSpec, CommandParseError, CommandValue};
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
