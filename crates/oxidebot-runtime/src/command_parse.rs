//! Text and segment tokenization for command invocations.

use super::{CommandParseError, CommandValue};
use oxidebot_core::source::message::MessageSegment;

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
