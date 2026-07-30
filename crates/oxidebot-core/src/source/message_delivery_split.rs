//! Deterministic physical-message splitting for platform limits.

use super::*;

pub(in crate::source::message) fn split_for_media_limit(
    message: Message,
    limit: Option<usize>,
) -> Vec<Message> {
    let Some(limit) = limit.filter(|limit| *limit > 0) else {
        return vec![message];
    };
    if message.segments.iter().map(media_units).sum::<usize>() <= limit {
        return vec![message];
    }

    let mut output = Vec::new();
    let mut current = Message::default();
    let mut units = 0usize;
    let mut first = true;
    let flush = |output: &mut Vec<Message>, current: &mut Message, first: &mut bool| {
        if current.segments.is_empty() {
            return;
        }
        current.options = split_message_options(&message.options, *first);
        if *first {
            current.id = message.id.clone();
            *first = false;
        }
        output.push(std::mem::take(current));
    };

    for segment in &message.segments {
        if let MessageSegment::MediaGallery(items) = segment {
            for chunk in items.chunks(limit) {
                if units > 0 && units.saturating_add(chunk.len()) > limit {
                    flush(&mut output, &mut current, &mut first);
                    units = 0;
                }
                current
                    .segments
                    .push(MessageSegment::MediaGallery(chunk.to_vec()));
                units = units.saturating_add(chunk.len());
                if units == limit {
                    flush(&mut output, &mut current, &mut first);
                    units = 0;
                }
            }
            continue;
        }
        let segment_units = media_units(segment);
        if segment_units > 0 && units > 0 && units.saturating_add(segment_units) > limit {
            flush(&mut output, &mut current, &mut first);
            units = 0;
        }
        current.segments.push(segment.clone());
        units = units.saturating_add(segment_units);
        if units == limit {
            flush(&mut output, &mut current, &mut first);
            units = 0;
        }
    }
    flush(&mut output, &mut current, &mut first);
    output
}

fn media_units(segment: &MessageSegment) -> usize {
    match segment {
        MessageSegment::Media { .. } | MessageSegment::Sticker(_) => 1,
        MessageSegment::Share { image, .. } => {
            if image.is_some() {
                1
            } else {
                0
            }
        }
        MessageSegment::MediaGallery(items) => items.len(),
        _ => 0,
    }
}

pub(in crate::source::message) fn split_for_text_limit(
    message: Message,
    limit: Option<usize>,
) -> Vec<Message> {
    let Some(limit) = limit.filter(|limit| *limit > 0) else {
        return vec![message];
    };
    let total_text = message
        .segments
        .iter()
        .map(text_character_len)
        .sum::<usize>();
    if total_text <= limit {
        return vec![message];
    }

    let mut messages = Vec::new();
    let mut current = Message::default();
    let mut current_text = 0usize;
    let mut first = true;

    let flush = |messages: &mut Vec<Message>, current: &mut Message, first: &mut bool| {
        if current.segments.is_empty() {
            return;
        }
        current.options = split_message_options(&message.options, *first);
        if *first {
            current.id = message.id.clone();
            *first = false;
        }
        messages.push(std::mem::take(current));
    };

    for segment in &message.segments {
        let text_len = text_character_len(segment);
        if text_len == 0 {
            current.segments.push(segment.clone());
            continue;
        }

        let mut offset = 0usize;
        while offset < text_len {
            if current_text == limit {
                flush(&mut messages, &mut current, &mut first);
                current_text = 0;
            }
            let available = limit - current_text;
            let take = available.min(text_len - offset);
            current
                .segments
                .push(slice_text_segment(segment, offset, offset + take));
            current_text += take;
            offset += take;
            if current_text == limit {
                flush(&mut messages, &mut current, &mut first);
                current_text = 0;
            }
        }
    }
    flush(&mut messages, &mut current, &mut first);

    messages
}

fn text_character_len(segment: &MessageSegment) -> usize {
    match segment {
        MessageSegment::Text { content } => content.chars().count(),
        MessageSegment::RichText(content) => content.text.chars().count(),
        _ => 0,
    }
}

fn slice_text_segment(segment: &MessageSegment, start: usize, end: usize) -> MessageSegment {
    match segment {
        MessageSegment::Text { content } => MessageSegment::Text {
            content: slice_chars(content, start, end).to_owned(),
        },
        MessageSegment::RichText(content) => {
            let (byte_start, byte_end) = char_range_to_bytes(&content.text, start, end);
            let spans = content
                .spans
                .iter()
                .filter_map(|span| {
                    let intersection_start = span.range.start.max(byte_start);
                    let intersection_end = span.range.end.min(byte_end);
                    (intersection_start < intersection_end).then(|| crate::content::TextSpan {
                        range: (intersection_start - byte_start)..(intersection_end - byte_start),
                        styles: span.styles.clone(),
                    })
                })
                .collect();
            MessageSegment::RichText(RichText {
                text: content.text[byte_start..byte_end].to_owned(),
                spans,
            })
        }
        _ => segment.clone(),
    }
}

fn slice_chars(text: &str, start: usize, end: usize) -> &str {
    let (byte_start, byte_end) = char_range_to_bytes(text, start, end);
    &text[byte_start..byte_end]
}

fn char_range_to_bytes(text: &str, start: usize, end: usize) -> (usize, usize) {
    let byte_start = if start == 0 {
        0
    } else {
        text.char_indices()
            .nth(start)
            .map_or(text.len(), |(offset, _)| offset)
    };
    let byte_end = if end == 0 {
        0
    } else {
        text.char_indices()
            .nth(end)
            .map_or(text.len(), |(offset, _)| offset)
    };
    (byte_start, byte_end)
}
