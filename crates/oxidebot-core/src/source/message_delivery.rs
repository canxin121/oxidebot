//! Capability-aware message delivery planning.
//!
//! The planner transforms canonical messages into physical delivery plans
//! without I/O. It centralizes fallback, capability adaptation, degradation
//! accounting, and deterministic limit splitting.

use super::*;

pub(super) fn adapt_segment(
    segment: &MessageSegment,
    index: usize,
    capabilities: &BotCapabilities,
    policy: FallbackPolicy,
    output: &mut Vec<MessageSegment>,
    degradations: &mut Vec<DeliveryDegradation>,
) -> Result<(), DeliveryPlanningError> {
    let limit_violation = segment_limit_violation(segment, capabilities);
    let support = if limit_violation.is_some() {
        SupportLevel::Unsupported
    } else {
        segment_support(segment, capabilities)
    };
    let path = format!("segments[{index}]");
    let feature = limit_violation.unwrap_or_else(|| segment.kind_name().to_owned());
    match support {
        SupportLevel::Native => {
            output.push(segment.clone());
            return Ok(());
        }
        SupportLevel::Emulated => {
            output.push(segment.clone());
            degradations.push(DeliveryDegradation {
                path,
                feature,
                kind: DegradationKind::Emulated,
                detail: "feature will use the adapter's portable representation".to_owned(),
            });
            return Ok(());
        }
        SupportLevel::Unsupported => {}
    }

    let fallback = segment.fallback_text();
    match policy {
        FallbackPolicy::Strict => Err(DeliveryPlanningError { path, feature }),
        FallbackPolicy::DropUnsupported => {
            degradations.push(DeliveryDegradation {
                path,
                feature,
                kind: DegradationKind::Dropped,
                detail: "unsupported segment was removed".to_owned(),
            });
            Ok(())
        }
        FallbackPolicy::ToText | FallbackPolicy::Flatten | FallbackPolicy::Auto => {
            if let Some(text) = fallback.filter(|value| !value.is_empty()) {
                output.push(MessageSegment::text(text));
                degradations.push(DeliveryDegradation {
                    path,
                    feature,
                    kind: if matches!(policy, FallbackPolicy::Flatten) {
                        DegradationKind::Flattened
                    } else {
                        DegradationKind::ConvertedToText
                    },
                    detail: "unsupported segment was represented as text".to_owned(),
                });
                Ok(())
            } else if matches!(policy, FallbackPolicy::Auto) {
                Err(DeliveryPlanningError { path, feature })
            } else {
                degradations.push(DeliveryDegradation {
                    path,
                    feature,
                    kind: DegradationKind::Dropped,
                    detail: "unsupported segment had no portable fallback".to_owned(),
                });
                Ok(())
            }
        }
    }
}

fn option_is_supported(
    support: SupportLevel,
    path: &str,
    feature: &str,
    degradations: &mut Vec<DeliveryDegradation>,
) -> bool {
    match support {
        SupportLevel::Native => true,
        SupportLevel::Emulated => {
            degradations.push(DeliveryDegradation {
                path: path.to_owned(),
                feature: feature.to_owned(),
                kind: DegradationKind::Emulated,
                detail: "delivery option will use the adapter's portable representation".to_owned(),
            });
            true
        }
        SupportLevel::Unsupported => false,
    }
}

pub(super) fn adapt_options(
    message: &mut Message,
    capabilities: &BotCapabilities,
    policy: FallbackPolicy,
    degradations: &mut Vec<DeliveryDegradation>,
) -> Result<(), DeliveryPlanningError> {
    if let Some(components) = message.options.components.take() {
        let within_limit = capabilities
            .limits
            .max_components
            .is_none_or(|limit| component_count(&components) <= limit);
        let support = match &components {
            MessageComponents::InlineKeyboard(_) => capabilities.components.inline_keyboard,
            MessageComponents::ReplyKeyboard(_)
            | MessageComponents::RemoveReplyKeyboard { .. }
            | MessageComponents::ForceReply { .. } => capabilities.components.reply_keyboard,
            MessageComponents::PlatformNative(_) => capabilities.components.platform_native,
        };
        let supported = within_limit
            && option_is_supported(
                support,
                "options.components",
                "message components",
                degradations,
            );
        if supported {
            message.options.components = Some(components);
        } else {
            let fallback = render_components(&components);
            if matches!(policy, FallbackPolicy::Strict)
                || (matches!(policy, FallbackPolicy::Auto) && fallback.is_empty())
            {
                return Err(DeliveryPlanningError {
                    path: "options.components".to_owned(),
                    feature: "message components".to_owned(),
                });
            }
            if !fallback.is_empty()
                && matches!(
                    policy,
                    FallbackPolicy::ToText | FallbackPolicy::Flatten | FallbackPolicy::Auto
                )
            {
                message.push(format!("\n{fallback}"));
            }
            degradations.push(DeliveryDegradation {
                path: "options.components".to_owned(),
                feature: "message components".to_owned(),
                kind: if fallback.is_empty() {
                    DegradationKind::Dropped
                } else {
                    DegradationKind::ConvertedToText
                },
                detail: "interactive components are unsupported by the target adapter".to_owned(),
            });
        }
    }

    if message.options.reply.is_some() {
        if !option_is_supported(
            capabilities.delivery.replies,
            "options.reply",
            "reply",
            degradations,
        ) {
            remove_option_or_error(policy, "options.reply", "reply", true, degradations)?;
            message.options.reply = None;
        } else if let Some(reply) = message.options.reply.as_mut() {
            if reply.quote.is_some()
                && !option_is_supported(
                    capabilities.delivery.quoted_replies,
                    "options.reply.quote",
                    "quoted reply",
                    degradations,
                )
            {
                remove_option_or_error(
                    policy,
                    "options.reply.quote",
                    "quoted reply",
                    false,
                    degradations,
                )?;
                reply.quote = None;
                reply.quote_position = None;
            }
            if reply.platform_data.is_some()
                && !option_is_supported(
                    capabilities.content.platform_native,
                    "options.reply.platform_data",
                    "platform-native reply data",
                    degradations,
                )
            {
                remove_option_or_error(
                    policy,
                    "options.reply.platform_data",
                    "platform-native reply data",
                    true,
                    degradations,
                )?;
                reply.platform_data = None;
            }
        }
    }

    match message.options.visibility.clone() {
        MessageVisibility::Public => {}
        MessageVisibility::Ephemeral => {
            if !option_is_supported(
                capabilities.delivery.ephemeral,
                "options.visibility",
                "ephemeral visibility",
                degradations,
            ) {
                remove_option_or_error(
                    policy,
                    "options.visibility",
                    "ephemeral visibility",
                    true,
                    degradations,
                )?;
                message.options.visibility = MessageVisibility::Public;
            }
        }
        MessageVisibility::PrivateTo(_) => {
            if !option_is_supported(
                capabilities.delivery.private_to_users,
                "options.visibility",
                "private message visibility",
                degradations,
            ) {
                remove_option_or_error(
                    policy,
                    "options.visibility",
                    "private message visibility",
                    true,
                    degradations,
                )?;
                message.options.visibility = MessageVisibility::Public;
            }
        }
    }

    match message.options.delivery_time.clone() {
        DeliveryTime::Immediate => {}
        DeliveryTime::Scheduled(_) => {
            if !option_is_supported(
                capabilities.delivery.scheduling,
                "options.delivery_time",
                "scheduled delivery",
                degradations,
            ) {
                remove_option_or_error(
                    policy,
                    "options.delivery_time",
                    "scheduled delivery",
                    true,
                    degradations,
                )?;
                message.options.delivery_time = DeliveryTime::Immediate;
            }
        }
        DeliveryTime::Draft => {
            if !option_is_supported(
                capabilities.delivery.drafts,
                "options.delivery_time",
                "draft delivery",
                degradations,
            ) {
                remove_option_or_error(
                    policy,
                    "options.delivery_time",
                    "draft delivery",
                    true,
                    degradations,
                )?;
                message.options.delivery_time = DeliveryTime::Immediate;
            }
        }
    }

    let optional_features = [
        (
            message.options.notification == NotificationPolicy::Silent,
            capabilities.delivery.silent,
            "silent notification",
            false,
        ),
        (
            message.options.notification == NotificationPolicy::Force,
            capabilities.delivery.forced_notification,
            "forced notification",
            false,
        ),
        (
            message.options.protect_content,
            capabilities.delivery.protected_content,
            "protected content",
            true,
        ),
        (
            message.options.link_preview.is_some(),
            capabilities.delivery.link_preview_control,
            "link preview control",
            false,
        ),
        (
            message.options.mentions.is_some(),
            capabilities.delivery.mention_control,
            "mention policy",
            false,
        ),
        (
            message.options.idempotency_key.is_some(),
            capabilities.delivery.idempotency_keys,
            "idempotency key",
            false,
        ),
        (
            message.options.client_message_id.is_some(),
            capabilities.delivery.client_message_ids,
            "client message id",
            false,
        ),
        (
            !message.options.metadata.is_empty(),
            capabilities.delivery.metadata,
            "message metadata",
            false,
        ),
        (
            message.options.platform_data.is_some(),
            capabilities.content.platform_native,
            "platform-native message options",
            true,
        ),
    ];
    for (present, support, feature, semantic_loss) in optional_features {
        if present && !option_is_supported(support, "options", feature, degradations) {
            remove_option_or_error(policy, "options", feature, semantic_loss, degradations)?;
        }
    }
    if (!capabilities.delivery.silent.is_supported()
        && message.options.notification == NotificationPolicy::Silent)
        || (!capabilities.delivery.forced_notification.is_supported()
            && message.options.notification == NotificationPolicy::Force)
    {
        message.options.notification = NotificationPolicy::Default;
    }
    if !capabilities.delivery.protected_content.is_supported() {
        message.options.protect_content = false;
    }
    if !capabilities.delivery.link_preview_control.is_supported() {
        message.options.link_preview = None;
    }
    if !capabilities.delivery.mention_control.is_supported() {
        message.options.mentions = None;
    }
    if !capabilities.delivery.idempotency_keys.is_supported() {
        message.options.idempotency_key = None;
    }
    if !capabilities.delivery.client_message_ids.is_supported() {
        message.options.client_message_id = None;
    }
    if !capabilities.delivery.metadata.is_supported() {
        message.options.metadata.clear();
    }
    if !capabilities.content.platform_native.is_supported() {
        message.options.platform_data = None;
    }
    Ok(())
}

fn remove_option_or_error(
    policy: FallbackPolicy,
    path: &str,
    feature: &str,
    semantic_loss: bool,
    degradations: &mut Vec<DeliveryDegradation>,
) -> Result<(), DeliveryPlanningError> {
    if matches!(policy, FallbackPolicy::Strict)
        || (semantic_loss && matches!(policy, FallbackPolicy::Auto))
    {
        return Err(DeliveryPlanningError {
            path: path.to_owned(),
            feature: feature.to_owned(),
        });
    }
    degradations.push(DeliveryDegradation {
        path: path.to_owned(),
        feature: feature.to_owned(),
        kind: DegradationKind::OptionRemoved,
        detail: "unsupported delivery option was reset to the platform default".to_owned(),
    });
    Ok(())
}

fn segment_limit_violation(
    segment: &MessageSegment,
    capabilities: &BotCapabilities,
) -> Option<String> {
    let limits = &capabilities.limits;
    if let Some(max) = limits.max_rich_text_length {
        if matches!(segment, MessageSegment::RichText(text) if text.text.chars().count() > max) {
            return Some(format!(
                "rich text exceeds the platform limit of {max} characters"
            ));
        }
    }
    if let Some(max) = limits.max_caption_length {
        if matches!(segment, MessageSegment::Media { media, .. } if media.caption.as_ref().is_some_and(|caption| caption.text.chars().count() > max))
        {
            return Some(format!(
                "media caption exceeds the platform limit of {max} characters"
            ));
        }
    }
    if let Some(max) = limits.max_poll_options {
        if matches!(segment, MessageSegment::Poll(poll) if poll.options.len() > max) {
            return Some(format!("poll exceeds the platform limit of {max} options"));
        }
    }
    for file in segment_files(segment) {
        if let Some(max) = limits.max_file_bytes {
            if file.size.is_some_and(|size| size > max) {
                return Some(format!(
                    "file `{}` exceeds the platform limit of {max} bytes",
                    file.name
                ));
            }
        }
        if !limits.supported_mime_types.is_empty() {
            if let Some(mime) = file.mime.as_deref() {
                if !limits
                    .supported_mime_types
                    .iter()
                    .any(|allowed| mime_matches(allowed, mime))
                {
                    return Some(format!(
                        "MIME type `{mime}` is not accepted by the platform"
                    ));
                }
            }
        }
    }
    None
}

fn segment_files(segment: &MessageSegment) -> Vec<&File> {
    match segment {
        MessageSegment::Share { image, .. } => image.iter().collect(),
        MessageSegment::Media { media, .. } => vec![&media.file],
        MessageSegment::MediaGallery(items) => items.iter().map(|item| &item.media.file).collect(),
        MessageSegment::Emoji(emoji) => emoji.file.iter().collect(),
        MessageSegment::Sticker(sticker) => sticker.file.iter().collect(),
        _ => Vec::new(),
    }
}

fn mime_matches(allowed: &str, actual: &str) -> bool {
    allowed == actual
        || allowed == "*/*"
        || allowed.strip_suffix("/*").is_some_and(|prefix| {
            actual.starts_with(prefix) && actual.as_bytes().get(prefix.len()) == Some(&b'/')
        })
}

fn segment_support(segment: &MessageSegment, capabilities: &BotCapabilities) -> SupportLevel {
    let content = &capabilities.content;
    match segment {
        MessageSegment::Text { .. } => content.plain_text,
        MessageSegment::RichText(_) => content.rich_text,
        MessageSegment::Media { kind, media } => {
            let support = match kind {
                MediaType::Image => content.images,
                MediaType::Video => content.video,
                MediaType::Audio => content.audio,
                MediaType::Document => content.files,
                MediaType::Animation => content.animation,
                MediaType::VoiceNote => content.voice_notes,
                MediaType::VideoNote => content.video_notes,
                MediaType::PlatformNative(_) => content.platform_native,
            };
            if support == SupportLevel::Native && media_requires_emulation(media) {
                SupportLevel::Emulated
            } else {
                support
            }
        }
        MessageSegment::MediaGallery(_) => content.media_galleries,
        MessageSegment::Location(location) => {
            if content.location == SupportLevel::Native
                && (location.horizontal_accuracy.is_some()
                    || location.live_period.is_some()
                    || location.heading.is_some()
                    || location.proximity_alert_radius.is_some()
                    || location.platform_data.is_some())
            {
                SupportLevel::Emulated
            } else {
                content.location
            }
        }
        MessageSegment::Contact(_) => content.contacts,
        MessageSegment::Emoji(_) => content.custom_emoji,
        MessageSegment::Sticker(_) => content.stickers,
        MessageSegment::Poll(poll) => {
            if matches!(poll.kind, crate::content::PollType::Quiz) {
                content.quizzes
            } else {
                content.polls
            }
        }
        MessageSegment::Checklist(_) => content.checklists,
        MessageSegment::Layout(_) => content.rich_layout,
        MessageSegment::PlatformNative(_) => content.platform_native,
        MessageSegment::Reference { .. }
        | MessageSegment::ForwardNode { .. }
        | MessageSegment::ForwardCustomNode { .. } => capabilities.collaboration.forwarding,
        MessageSegment::At { .. } => content.user_mentions,
        MessageSegment::AtRole { .. } => content.role_mentions,
        MessageSegment::AtChannel { .. } => content.channel_mentions,
        MessageSegment::AtAll => content.everyone_mentions,
        MessageSegment::Share { .. } => content.shares,
    }
}

fn media_requires_emulation(media: &Media) -> bool {
    media.caption.is_some()
        || media.thumbnail.is_some()
        || media.width.is_some()
        || media.height.is_some()
        || media.spoiler
        || media.alt_text.is_some()
        || media.waveform.is_some()
        || media.platform_data.is_some()
}

fn component_count(components: &MessageComponents) -> usize {
    match components {
        MessageComponents::InlineKeyboard(keyboard) => {
            keyboard.rows.iter().map(|row| row.components.len()).sum()
        }
        MessageComponents::ReplyKeyboard(keyboard) => {
            keyboard.rows.iter().map(|row| row.buttons.len()).sum()
        }
        MessageComponents::RemoveReplyKeyboard { .. }
        | MessageComponents::ForceReply { .. }
        | MessageComponents::PlatformNative(_) => 1,
    }
}

fn split_message_options(options: &MessageOptions, first: bool) -> MessageOptions {
    if first {
        return options.clone();
    }
    let mut continuation = options.clone();
    continuation.components = None;
    continuation.reply = None;
    continuation.idempotency_key = None;
    continuation.client_message_id = None;
    continuation
}

pub(super) fn split_for_media_limit(message: Message, limit: Option<usize>) -> Vec<Message> {
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

pub(super) fn split_for_text_limit(message: Message, limit: Option<usize>) -> Vec<Message> {
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

pub(super) fn render_poll(poll: &Poll) -> String {
    let mut output = poll.question.text.clone();
    for (index, option) in poll.options.iter().enumerate() {
        output.push_str(&format!("\n{}. {}", index + 1, option.text.text));
    }
    output
}

pub(super) fn render_checklist(checklist: &Checklist) -> String {
    let mut output = checklist.title.text.clone();
    for task in &checklist.tasks {
        output.push_str(&format!(
            "\n{} {}",
            if task.completed { "[x]" } else { "[ ]" },
            task.text.text
        ));
    }
    output
}

fn render_components(components: &MessageComponents) -> String {
    use crate::interaction::{InteractionComponent, MessageComponents};
    match components {
        MessageComponents::InlineKeyboard(keyboard) => keyboard
            .rows
            .iter()
            .flat_map(|row| row.components.iter())
            .filter_map(|component| match component {
                InteractionComponent::Button(button) => Some(format!("[{}]", button.label)),
                InteractionComponent::Select(select) => select
                    .placeholder
                    .as_ref()
                    .map(|placeholder| format!("[{placeholder}]")),
                InteractionComponent::Input(_) | InteractionComponent::PlatformNative(_) => None,
            })
            .collect::<Vec<_>>()
            .join(" "),
        MessageComponents::ReplyKeyboard(keyboard) => keyboard
            .rows
            .iter()
            .flat_map(|row| row.buttons.iter())
            .map(|button| format!("[{}]", button.label))
            .collect::<Vec<_>>()
            .join(" "),
        MessageComponents::ForceReply { .. } => "[reply requested]".to_owned(),
        MessageComponents::RemoveReplyKeyboard { .. } | MessageComponents::PlatformNative(_) => {
            String::new()
        }
    }
}
