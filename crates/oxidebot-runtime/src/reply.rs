use crate::{authoring::ErasedDeliveryPipeline, HandlerError};
use oxidebot_core::{
    collaboration::{Reaction, ReactionOptions},
    conversation::{MessageRef, MessageTarget},
    source::message::{DeliveryDegradation, DeliveryReport, FallbackPolicy, Message},
    BotCapabilities, BotObject,
};
use std::{ops::Deref, sync::Arc, time::Duration};

/// Complete unified API object extracted for the current bot.
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
    target: MessageTarget,
    reply_to: Option<String>,
    fallback: FallbackPolicy,
    pipeline: Option<Arc<dyn ErasedDeliveryPipeline>>,
}

impl Reply {
    pub(crate) fn new(
        api: BotObject,
        target: MessageTarget,
        reply_to: Option<String>,
        pipeline: Option<Arc<dyn ErasedDeliveryPipeline>>,
    ) -> Self {
        Self {
            api,
            target,
            reply_to,
            fallback: FallbackPolicy::Auto,
            pipeline,
        }
    }

    #[must_use]
    pub fn target(&self) -> &MessageTarget {
        &self.target
    }

    pub(crate) fn retarget(&self, target: MessageTarget) -> Self {
        Self {
            api: Arc::clone(&self.api),
            target,
            reply_to: None,
            fallback: self.fallback,
            pipeline: self.pipeline.clone(),
        }
    }

    pub(crate) fn rebind(&self, api: BotObject, target: MessageTarget) -> Self {
        Self {
            api: Arc::new(api),
            target,
            reply_to: None,
            fallback: self.fallback,
            pipeline: self.pipeline.clone(),
        }
    }

    /// Overrides capability fallback behavior for subsequent sends.
    #[must_use]
    pub const fn fallback(mut self, fallback: FallbackPolicy) -> Self {
        self.fallback = fallback;
        self
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
        let report = if let Some(pipeline) = &self.pipeline {
            pipeline
                .deliver(&self.api, self.target.clone(), message, self.fallback)
                .await?
        } else {
            self.api
                .send_outgoing_message_with(self.target.clone(), message, self.fallback)
                .await
                .map_err(|error| HandlerError::Api(error.to_string()))?
        };
        Ok(Receipt {
            api: Arc::clone(&self.api),
            target: self.target.clone(),
            report: Arc::new(report),
            pipeline: self.pipeline.clone(),
        })
    }
}

impl std::fmt::Debug for Reply {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Reply")
            .field("target", &self.target)
            .field("reply_to", &self.reply_to)
            .field("fallback", &self.fallback)
            .finish_non_exhaustive()
    }
}

/// Handles returned by one logical send. Platforms may split one message into
/// several physical messages, so bulk methods operate on every reference and
/// indexed methods operate on one physical message.
#[derive(Clone)]
pub struct Receipt {
    api: BotObject,
    target: MessageTarget,
    report: Arc<DeliveryReport>,
    pipeline: Option<Arc<dyn ErasedDeliveryPipeline>>,
}

impl Receipt {
    #[must_use]
    pub fn report(&self) -> &DeliveryReport {
        &self.report
    }

    #[must_use]
    pub fn references(&self) -> &[MessageRef] {
        &self.report.messages
    }

    pub fn ids(&self) -> impl ExactSizeIterator<Item = &str> {
        self.report
            .messages
            .iter()
            .map(|message| message.id.as_str())
    }

    #[must_use]
    pub fn degraded(&self) -> bool {
        self.report.degraded()
    }

    #[must_use]
    pub fn degradations(&self) -> &[DeliveryDegradation] {
        &self.report.degradations
    }

    #[must_use]
    pub fn capabilities(&self) -> BotCapabilities {
        self.api.bot_capabilities()
    }

    #[must_use]
    pub fn is_editable(&self) -> bool {
        self.capabilities().delivery.edit_messages.is_supported()
    }

    #[must_use]
    pub fn is_deletable(&self) -> bool {
        self.capabilities().delivery.delete_messages.is_supported()
    }

    #[must_use]
    pub fn is_reactionable(&self) -> bool {
        self.capabilities().collaboration.reactions.is_supported()
    }

    pub async fn send(&self, message: impl Into<Message>) -> Result<Receipt, HandlerError> {
        let message = message.into();
        let report = if let Some(pipeline) = &self.pipeline {
            pipeline
                .deliver(
                    &self.api,
                    self.target.clone(),
                    message,
                    FallbackPolicy::Auto,
                )
                .await?
        } else {
            self.api
                .send_outgoing_message_with(self.target.clone(), message, FallbackPolicy::Auto)
                .await
                .map_err(|error| HandlerError::Api(error.to_string()))?
        };
        Ok(Self {
            api: Arc::clone(&self.api),
            target: self.target.clone(),
            report: Arc::new(report),
            pipeline: self.pipeline.clone(),
        })
    }

    pub async fn reply(&self, message: impl Into<Message>) -> Result<Receipt, HandlerError> {
        let Some(reference) = self.references().last() else {
            return Err(HandlerError::Api(
                "receipt contains no message reference".into(),
            ));
        };
        self.send(message.into().reply_to(reference.id.clone()))
            .await
    }

    pub async fn edit(&self, replacement: impl Into<Message>) -> Result<(), HandlerError> {
        let replacement = replacement.into();
        for reference in self.references() {
            self.api
                .edit_outgoing_message(reference.clone(), replacement.clone())
                .await
                .map_err(|error| HandlerError::Api(error.to_string()))?;
        }
        Ok(())
    }

    pub async fn edit_at(
        &self,
        index: usize,
        replacement: impl Into<Message>,
    ) -> Result<(), HandlerError> {
        let reference = self.reference_at(index)?;
        self.api
            .edit_outgoing_message(reference.clone(), replacement.into())
            .await
            .map_err(|error| HandlerError::Api(error.to_string()))
    }

    pub async fn delete(&self) -> Result<(), HandlerError> {
        for reference in self.references() {
            self.api
                .delete_message_ref(reference.clone())
                .await
                .map_err(|error| HandlerError::Api(error.to_string()))?;
        }
        Ok(())
    }

    pub async fn delete_at(&self, index: usize) -> Result<(), HandlerError> {
        let reference = self.reference_at(index)?;
        self.api
            .delete_message_ref(reference.clone())
            .await
            .map_err(|error| HandlerError::Api(error.to_string()))
    }

    pub async fn react(&self, reaction: impl Into<String>) -> Result<(), HandlerError> {
        let reaction = Reaction::UnicodeEmoji(reaction.into());
        for reference in self.references() {
            self.api
                .add_message_reaction(
                    reference.clone(),
                    reaction.clone(),
                    ReactionOptions::default(),
                )
                .await
                .map_err(|error| HandlerError::Api(error.to_string()))?;
        }
        Ok(())
    }

    pub async fn react_at(
        &self,
        index: usize,
        reaction: impl Into<String>,
    ) -> Result<(), HandlerError> {
        let reference = self.reference_at(index)?;
        self.api
            .add_message_reaction(
                reference.clone(),
                Reaction::UnicodeEmoji(reaction.into()),
                ReactionOptions::default(),
            )
            .await
            .map_err(|error| HandlerError::Api(error.to_string()))
    }

    /// Waits and then deletes every physical message represented by this receipt.
    pub async fn delete_after(&self, delay: Duration) -> Result<(), HandlerError> {
        tokio::time::sleep(delay).await;
        self.delete().await
    }

    fn reference_at(&self, index: usize) -> Result<&MessageRef, HandlerError> {
        self.references().get(index).ok_or_else(|| {
            HandlerError::Api(format!(
                "receipt contains {} physical messages; index {index} is out of range",
                self.references().len()
            ))
        })
    }
}

impl std::fmt::Debug for Receipt {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Receipt")
            .field("target", &self.target)
            .field("report", &self.report)
            .finish_non_exhaustive()
    }
}
