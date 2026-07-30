//! Portable text fallbacks for rich content the target cannot render natively.

use super::*;

pub(in crate::source::message) fn render_poll(poll: &Poll) -> String {
    let mut output = poll.question.text.clone();
    for (index, option) in poll.options.iter().enumerate() {
        output.push_str(&format!("\n{}. {}", index + 1, option.text.text));
    }
    output
}

pub(in crate::source::message) fn render_checklist(checklist: &Checklist) -> String {
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

pub(in crate::source::message) fn render_components(components: &MessageComponents) -> String {
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
