use super::super::{IdempotencyGuarantee, RuntimeCapabilities};

use oxidebot_core::{
    application::CommandDefinition,
    collaboration::{Reaction, ReactionOptions},
    conversation::{
        ConversationKind, ConversationRef as PublicConversationRef, MessageRef as PublicMessageRef,
        MessageTarget as PublicMessageTarget,
    },
    interaction::{InteractionResponse, InteractionResponseHandle, InteractionVisibility},
    source::message::{
        DeliveryPlan as PublicDeliveryPlan, DeliveryReport as PublicDeliveryReport,
        Message as PublicMessage,
    },
};
use std::sync::Arc;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) enum CommandKey {
    PublicConversation(Arc<str>),
    Interaction(Arc<str>),
    Independent(u64),
}

pub(in crate::bot) enum CommandOperation {
    Deliver {
        target: PublicMessageTarget,
        plan: PublicDeliveryPlan,
    },
    EditPublic {
        message: PublicMessageRef,
        replacement: PublicMessage,
    },
    DeletePublic {
        message: PublicMessageRef,
    },
    ReactPublic {
        message: PublicMessageRef,
        reaction: Reaction,
        options: ReactionOptions,
    },
    AnswerInteraction {
        handle: InteractionResponseHandle,
        response: InteractionResponse,
    },
    DeferInteraction {
        handle: InteractionResponseHandle,
        visibility: InteractionVisibility,
    },
    InteractionFollowup {
        handle: InteractionResponseHandle,
        message: PublicMessage,
    },
    EditInteraction {
        handle: InteractionResponseHandle,
        message: PublicMessage,
    },
    PublishCommands {
        definitions: Vec<CommandDefinition>,
    },
}

impl CommandOperation {
    pub(super) fn key(&self, sequence: u64) -> CommandKey {
        match self {
            Self::Deliver { target, .. } => {
                CommandKey::PublicConversation(public_target_key(target))
            }
            Self::EditPublic { message, .. }
            | Self::DeletePublic { message }
            | Self::ReactPublic { message, .. } => message
                .conversation
                .as_ref()
                .map(public_conversation_key)
                .map(CommandKey::PublicConversation)
                .unwrap_or(CommandKey::Independent(sequence)),
            Self::AnswerInteraction { handle, .. }
            | Self::DeferInteraction { handle, .. }
            | Self::InteractionFollowup { handle, .. }
            | Self::EditInteraction { handle, .. } => {
                CommandKey::Interaction(Arc::from(handle.id.as_str()))
            }
            Self::PublishCommands { .. } => CommandKey::Independent(sequence),
        }
    }

    pub(super) fn retained_bytes(&self) -> usize {
        match self {
            Self::Deliver { target, plan } => public_target_bytes(target)
                .saturating_add(
                    plan.messages
                        .iter()
                        .map(PublicMessage::estimated_bytes)
                        .sum::<usize>(),
                )
                .saturating_add(
                    plan.degradations
                        .iter()
                        .map(|item| {
                            item.path
                                .len()
                                .saturating_add(item.feature.len())
                                .saturating_add(item.detail.len())
                                .saturating_add(64)
                        })
                        .sum::<usize>(),
                )
                .saturating_add(256),
            Self::EditPublic {
                message,
                replacement,
            } => public_message_ref_bytes(message)
                .saturating_add(replacement.estimated_bytes())
                .saturating_add(256),
            Self::DeletePublic { message } => public_message_ref_bytes(message).saturating_add(128),
            Self::ReactPublic {
                message,
                reaction,
                options,
            } => public_message_ref_bytes(message)
                .saturating_add(serialized_size(reaction))
                .saturating_add(serialized_size(options))
                .saturating_add(128),
            Self::AnswerInteraction { handle, response } => interaction_handle_bytes(handle)
                .saturating_add(serialized_size(response))
                .saturating_add(256),
            Self::DeferInteraction { handle, .. } => {
                interaction_handle_bytes(handle).saturating_add(128)
            }
            Self::InteractionFollowup { handle, message }
            | Self::EditInteraction { handle, message } => interaction_handle_bytes(handle)
                .saturating_add(message.estimated_bytes())
                .saturating_add(256),
            Self::PublishCommands { definitions } => serialized_size(definitions)
                .saturating_add(definitions.capacity() * std::mem::size_of::<CommandDefinition>())
                .saturating_add(256),
        }
    }

    pub(super) fn priority(&self) -> CommandPriority {
        match self {
            Self::AnswerInteraction { .. }
            | Self::DeferInteraction { .. }
            | Self::InteractionFollowup { .. }
            | Self::EditInteraction { .. } => CommandPriority::High,
            Self::Deliver { .. }
            | Self::EditPublic { .. }
            | Self::DeletePublic { .. }
            | Self::ReactPublic { .. }
            | Self::PublishCommands { .. } => CommandPriority::Normal,
        }
    }

    pub(super) fn idempotent(&self, capabilities: RuntimeCapabilities) -> bool {
        match self {
            Self::Deliver { plan, .. } => {
                capabilities.send_idempotency != IdempotencyGuarantee::Unsupported
                    && !plan.messages.is_empty()
                    && plan
                        .messages
                        .iter()
                        .all(|message| message.options.idempotency_key.is_some())
            }
            Self::DeletePublic { .. } => capabilities.delete_idempotent,
            // Publishing replaces the complete definition set and is safe to repeat.
            Self::PublishCommands { .. } => true,
            Self::EditPublic { .. }
            | Self::ReactPublic { .. }
            | Self::AnswerInteraction { .. }
            | Self::DeferInteraction { .. }
            | Self::InteractionFollowup { .. }
            | Self::EditInteraction { .. } => false,
        }
    }
}

fn public_conversation_key(conversation: &PublicConversationRef) -> Arc<str> {
    let mut key = String::new();
    append_public_conversation_key(conversation, &mut key);
    Arc::from(key)
}

fn public_target_key(target: &PublicMessageTarget) -> Arc<str> {
    public_conversation_key(&target.conversation)
}

fn append_public_conversation_key(conversation: &PublicConversationRef, output: &mut String) {
    if let Some(parent) = &conversation.parent {
        append_public_conversation_key(parent, output);
        output.push('/');
    }
    output.push_str(match conversation.kind {
        ConversationKind::Direct => "direct:",
        ConversationKind::Group => "group:",
        ConversationKind::Channel => "channel:",
        ConversationKind::Thread => "thread:",
        ConversationKind::Topic => "topic:",
        ConversationKind::Forum => "forum:",
        ConversationKind::Unknown => "unknown:",
        ConversationKind::PlatformNative(_) => "native:",
    });
    output.push_str(&conversation.id.to_string());
}

fn public_conversation_bytes(conversation: &PublicConversationRef) -> usize {
    conversation
        .id
        .estimated_bytes()
        .saturating_add(
            conversation
                .parent
                .as_deref()
                .map_or(0, public_conversation_bytes),
        )
        .saturating_add(serialized_size(&conversation.platform_data))
        .saturating_add(64)
}

fn public_target_bytes(target: &PublicMessageTarget) -> usize {
    public_conversation_bytes(&target.conversation)
        .saturating_add(
            target
                .recipients
                .iter()
                .map(oxidebot_core::UserId::estimated_bytes)
                .sum::<usize>(),
        )
        .saturating_add(target.recipients.capacity() * std::mem::size_of::<oxidebot_core::UserId>())
        .saturating_add(serialized_size(&target.platform_data))
        .saturating_add(64)
}

fn public_message_ref_bytes(message: &PublicMessageRef) -> usize {
    message
        .id
        .estimated_bytes()
        .saturating_add(
            message
                .conversation
                .as_ref()
                .map_or(0, public_conversation_bytes),
        )
        .saturating_add(serialized_size(&message.platform_data))
        .saturating_add(64)
}

fn interaction_handle_bytes(handle: &InteractionResponseHandle) -> usize {
    handle
        .id
        .len()
        .saturating_add(serialized_size(&handle.platform_data))
        .saturating_add(128)
}

fn serialized_size(value: &impl serde::Serialize) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CommandPriority {
    High,
    Normal,
}
pub(in crate::bot) enum ApiCommandResult {
    Messages(Vec<PublicMessageRef>),
    Delivery(PublicDeliveryReport),
    Unit,
}
