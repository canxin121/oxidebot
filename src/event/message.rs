use crate::{
    api::{payload::SendMessageTarget, response},
    bot::BotObject,
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
    pub platform: &'static str,
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
    ) -> Result<response::SendMessageResponse> {
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
    ) -> Result<response::SendMessageResponse> {
        bot.send_message(message, SendMessageTarget::Private(self.sender.id.clone()))
            .await
    }

    pub async fn delete_message(&self, bot: BotObject) -> Result<()> {
        bot.delete_message(self.id.clone()).await
    }

    pub async fn replay_message(
        &self,
        bot: BotObject,
        message: Vec<MessageSegment>,
    ) -> Result<response::SendMessageResponse> {
        let mut message = message;
        message.push(MessageSegment::Reply {
            message_id: self.message.id.clone(),
        });
        self.send_message(bot, message).await
    }

    pub async fn set_reactions(&self, bot: BotObject, reaction_ids: Vec<String>) -> Result<()> {
        for reaction_id in reaction_ids {
            bot.set_message_reaction(self.id.clone(), reaction_id)
                .await?;
        }
        Ok(())
    }
}
