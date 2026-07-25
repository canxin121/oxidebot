use crate::{
    api::{payload::SendMessageTarget, response},
    bot::BotObject,
    collaboration::Reaction,
    conversation::MessageRef,
    source::{
        group::Group,
        message::{Message, MessageSegment},
        user::User,
    },
};
use anyhow::Result;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MessageEvent {
    pub id: String,
    pub time: Option<DateTime<Utc>>,
    pub sender: User,
    pub group: Option<Group>,
    pub message: Message,
}

impl MessageEvent {
    pub async fn send_message(
        &self,
        bot: BotObject,
        message: Vec<MessageSegment>,
    ) -> Result<Vec<response::SendMessageResponse>> {
        match &self.group {
            Some(group) => {
                bot.send_message(message, SendMessageTarget::Group(group.id.clone()))
                    .await
            }
            None => {
                bot.send_message(message, SendMessageTarget::Private(self.sender.id.clone()))
                    .await
            }
        }
    }

    pub async fn send_private_message(
        &self,
        bot: BotObject,
        message: Vec<MessageSegment>,
    ) -> Result<Vec<response::SendMessageResponse>> {
        bot.send_message(message, SendMessageTarget::Private(self.sender.id.clone()))
            .await
    }

    pub async fn delete_message(&self, bot: BotObject) -> Result<()> {
        bot.delete_message(self.id.clone()).await
    }

    pub async fn reply_message(
        &self,
        bot: BotObject,
        message: Vec<MessageSegment>,
    ) -> Result<Vec<response::SendMessageResponse>> {
        let mut message = message;
        message.push(MessageSegment::Reply {
            message_id: self.message.id.clone(),
        });
        self.send_message(bot, message).await
    }

    /// Legacy spelling retained for 0.1 compatibility.
    pub async fn replay_message(
        &self,
        bot: BotObject,
        message: Vec<MessageSegment>,
    ) -> Result<Vec<response::SendMessageResponse>> {
        self.reply_message(bot, message).await
    }

    pub async fn set_reactions(&self, bot: BotObject, reaction_ids: Vec<String>) -> Result<()> {
        bot.set_message_reactions(
            MessageRef::new(self.id.clone()),
            reaction_ids
                .into_iter()
                .map(Reaction::UnicodeEmoji)
                .collect(),
        )
        .await
    }
}
