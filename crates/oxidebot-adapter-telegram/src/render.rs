//! Telegram MarkdownV2 rendering for portable messages.

use oxidebot_core::{
    content::{RichText, TextStyle},
    source::message::{Message, MessageSegment},
};

pub(crate) fn telegram_text(message: &Message) -> (String, Option<&'static str>) {
    let needs_markdown = message.segments.iter().any(
        |segment| matches!(segment, MessageSegment::RichText(value) if !value.spans.is_empty()),
    );
    if !needs_markdown {
        return (message.get_raw_text(), None);
    }
    let mut output = String::new();
    for segment in &message.segments {
        match segment {
            MessageSegment::Text { content } => output.push_str(&escape_markdown_v2(content)),
            MessageSegment::RichText(value) => output.push_str(&render_rich_text_markdown(value)),
            _ => {
                if let Some(text) = segment.fallback_text() {
                    output.push_str(&escape_markdown_v2(&text));
                }
            }
        }
    }
    (output, Some("MarkdownV2"))
}

fn render_rich_text_markdown(value: &RichText) -> String {
    let spans = value
        .spans
        .iter()
        .filter(|span| {
            span.range.start < span.range.end
                && span.range.end <= value.text.len()
                && value.text.is_char_boundary(span.range.start)
                && value.text.is_char_boundary(span.range.end)
        })
        .collect::<Vec<_>>();
    if spans.is_empty() {
        return escape_markdown_v2(&value.text);
    }
    let mut boundaries = vec![0, value.text.len()];
    for span in &spans {
        boundaries.push(span.range.start);
        boundaries.push(span.range.end);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    boundaries
        .windows(2)
        .filter(|range| range[0] != range[1])
        .map(|range| {
            let styles = spans
                .iter()
                .flat_map(|span| {
                    (span.range.start <= range[0] && range[1] <= span.range.end)
                        .then_some(span.styles.as_slice())
                        .into_iter()
                        .flatten()
                })
                .collect::<Vec<_>>();
            render_markdown_chunk(&value.text[range[0]..range[1]], &styles)
        })
        .collect()
}

fn render_markdown_chunk(text: &str, styles: &[&TextStyle]) -> String {
    let preformatted = styles.iter().find_map(|style| match style {
        TextStyle::Preformatted { language } => Some(language.as_deref()),
        _ => None,
    });
    let code = styles.iter().any(|style| matches!(style, TextStyle::Code));
    let mut output = if let Some(language) = preformatted {
        format!(
            "```{}\n{}\n```",
            language.unwrap_or_default(),
            escape_markdown_code(text)
        )
    } else if code {
        format!("`{}`", escape_markdown_code(text))
    } else {
        escape_markdown_v2(text)
    };
    if preformatted.is_some() || code {
        return output;
    }
    for (matches, prefix, suffix) in [
        (
            styles.iter().any(|style| matches!(style, TextStyle::Bold)),
            "*",
            "*",
        ),
        (
            styles
                .iter()
                .any(|style| matches!(style, TextStyle::Italic)),
            "_",
            "_",
        ),
        (
            styles
                .iter()
                .any(|style| matches!(style, TextStyle::Underline)),
            "__",
            "__",
        ),
        (
            styles
                .iter()
                .any(|style| matches!(style, TextStyle::Strikethrough)),
            "~",
            "~",
        ),
        (
            styles
                .iter()
                .any(|style| matches!(style, TextStyle::Spoiler)),
            "||",
            "||",
        ),
    ] {
        if matches {
            output = format!("{prefix}{output}{suffix}");
        }
    }
    if let Some(url) = styles.iter().find_map(|style| match style {
        TextStyle::Link { url } => Some(url),
        _ => None,
    }) {
        output = format!("[{output}]({})", escape_markdown_link(url));
    }
    output
}

fn escape_markdown_v2(text: &str) -> String {
    text.chars()
        .flat_map(|character| {
            if matches!(
                character,
                '_' | '*'
                    | '['
                    | ']'
                    | '('
                    | ')'
                    | '~'
                    | '`'
                    | '>'
                    | '#'
                    | '+'
                    | '-'
                    | '='
                    | '|'
                    | '{'
                    | '}'
                    | '.'
                    | '!'
                    | '\\'
            ) {
                ['\\', character]
            } else {
                ['\0', character]
            }
        })
        .filter(|character| *character != '\0')
        .collect()
}
fn escape_markdown_code(text: &str) -> String {
    text.replace('\\', "\\\\").replace('`', "\\`")
}
fn escape_markdown_link(url: &str) -> String {
    url.replace('\\', "\\\\").replace(')', "\\)")
}
