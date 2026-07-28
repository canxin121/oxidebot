use crate::HandlerError;
use oxidebot_core::{
    api::{payload::SendMessageTarget, response::SendMessageResponse},
    source::message::Message,
    BotObject,
};
use std::{ops::Deref, sync::Arc, time::Duration};

/// Complete 0.1.8 API object extracted for the current bot.
#[derive(Clone)]
pub struct Bot(pub BotObject);

impl Bot {
    #[must_use]
    pub fn into_inner(self) -> BotObject {
        self.0
    }
}

impl Deref for Bot {
    type Target = dyn oxidebot_core::CallApiTrait;

    fn deref(&self) -> &Self::Target {
        self.0.as_ref()
    }
}

impl std::fmt::Debug for Bot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Bot").finish_non_exhaustive()
    }
}

/// Immediate sender bound to the event's natural reply target.
#[derive(Clone)]
pub struct Reply {
    api: BotObject,
    target: SendMessageTarget,
    reply_to: Option<String>,
}

impl Reply {
    pub(crate) fn new(api: BotObject, target: SendMessageTarget, reply_to: Option<String>) -> Self {
        Self {
            api,
            target,
            reply_to,
        }
    }

    #[must_use]
    pub fn target(&self) -> &SendMessageTarget {
        &self.target
    }

    pub async fn send(&self, message: impl Into<Message>) -> Result<Receipt, HandlerError> {
        self.send_inner(message.into(), false).await
    }

    pub async fn reply(&self, message: impl Into<Message>) -> Result<Receipt, HandlerError> {
        self.send_inner(message.into(), true).await
    }

    async fn send_inner(
        &self,
        mut message: Message,
        include_reply: bool,
    ) -> Result<Receipt, HandlerError> {
        if include_reply {
            if let Some(message_id) = &self.reply_to {
                message = message.reply_to(message_id.clone());
            }
        }
        let responses = self
            .api
            .send_message(message.into_segments(), self.target.clone())
            .await
            .map_err(|error| HandlerError::Api(error.to_string()))?;
        Ok(Receipt {
            api: Arc::clone(&self.api),
            responses: responses.into(),
        })
    }
}

impl std::fmt::Debug for Reply {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Reply")
            .field("target", &self.target)
            .field("reply_to", &self.reply_to)
            .finish_non_exhaustive()
    }
}

/// Handles returned by one logical send. Platforms may split one message into
/// several physical messages, so operations are applied to every response.
#[derive(Clone)]
pub struct Receipt {
    api: BotObject,
    responses: Arc<[SendMessageResponse]>,
}

impl Receipt {
    #[must_use]
    pub fn responses(&self) -> &[SendMessageResponse] {
        &self.responses
    }

    pub fn ids(&self) -> impl ExactSizeIterator<Item = &str> {
        self.responses
            .iter()
            .map(|response| response.sent_message_id.as_str())
    }

    pub async fn edit(&self, replacement: impl Into<Message>) -> Result<(), HandlerError> {
        let segments = replacement.into().into_segments();
        for response in self.responses.iter() {
            self.api
                .edit_message(response.sent_message_id.clone(), segments.clone())
                .await
                .map_err(|error| HandlerError::Api(error.to_string()))?;
        }
        Ok(())
    }

    pub async fn delete(&self) -> Result<(), HandlerError> {
        for response in self.responses.iter() {
            self.api
                .delete_message(response.sent_message_id.clone())
                .await
                .map_err(|error| HandlerError::Api(error.to_string()))?;
        }
        Ok(())
    }

    pub async fn react(&self, reaction: impl Into<String>) -> Result<(), HandlerError> {
        let reaction = reaction.into();
        for response in self.responses.iter() {
            self.api
                .set_message_reaction(response.sent_message_id.clone(), reaction.clone())
                .await
                .map_err(|error| HandlerError::Api(error.to_string()))?;
        }
        Ok(())
    }

    /// Waits and then deletes every physical message represented by this receipt.
    pub async fn delete_after(&self, delay: Duration) -> Result<(), HandlerError> {
        tokio::time::sleep(delay).await;
        self.delete().await
    }
}

impl std::fmt::Debug for Receipt {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Receipt")
            .field("responses", &self.responses)
            .finish_non_exhaustive()
    }
}
