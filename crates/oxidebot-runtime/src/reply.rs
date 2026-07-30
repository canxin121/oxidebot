use crate::{authoring::ErasedDeliveryPipeline, BotHandle, HandlerError};
use oxidebot_core::{
    collaboration::{Reaction, ReactionOptions},
    conversation::{MessageRef, MessageTarget},
    source::message::{DeliveryDegradation, DeliveryReport, FallbackPolicy, Message},
    BotCapabilities, BotObject,
};
use std::{ops::Deref, sync::Arc, time::Duration};

/// One physical-message outcome from a bulk receipt mutation.
#[derive(Clone, Debug, PartialEq)]
pub struct MutationItemResult {
    /// Zero-based position of the physical message in the receipt.
    pub index: usize,
    /// Reference to the physical message that was mutated.
    pub message: MessageRef,
    /// Stringified operation error, or `None` when the mutation succeeded.
    pub error: Option<String>,
}

impl MutationItemResult {
    /// Returns whether this individual physical-message mutation succeeded.
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.error.is_none()
    }
}

/// Complete per-item result for edit, delete, or reaction operations on a
/// logical receipt that contains multiple physical messages.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MutationReport {
    /// One outcome for each physical message in the logical receipt.
    pub items: Vec<MutationItemResult>,
}

impl MutationReport {
    /// Returns whether every physical-message mutation succeeded.
    #[must_use]
    pub fn completed(&self) -> bool {
        self.items.iter().all(MutationItemResult::succeeded)
    }

    /// Iterates over the physical-message mutations that failed.
    pub fn failures(&self) -> impl Iterator<Item = &MutationItemResult> {
        self.items.iter().filter(|item| !item.succeeded())
    }

    fn into_result(self, operation: &'static str) -> Result<(), HandlerError> {
        if self.completed() {
            Ok(())
        } else {
            Err(PartialMutationError {
                operation,
                report: self,
            }
            .into())
        }
    }
}

/// Error retaining all successful and failed physical mutations.
#[derive(Clone, Debug, PartialEq)]
pub struct PartialMutationError {
    /// Operation that yielded a partial outcome.
    pub operation: &'static str,
    /// Complete set of successful and failed per-message outcomes.
    pub report: MutationReport,
}

impl std::fmt::Display for PartialMutationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let succeeded = self
            .report
            .items
            .iter()
            .filter(|item| item.succeeded())
            .count();
        write!(
            formatter,
            "receipt {} completed for {succeeded} of {} physical messages",
            self.operation,
            self.report.items.len()
        )
    }
}

impl std::error::Error for PartialMutationError {}

/// Complete unified API object extracted for the current bot.
#[derive(Clone)]
pub struct Bot(pub BotObject);

impl Bot {
    /// Returns the underlying unified platform API object.
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
    bot: BotHandle,
    target: MessageTarget,
    reply_to: Option<oxidebot_core::MessageId>,
    fallback: FallbackPolicy,
    pipeline: Option<Arc<dyn ErasedDeliveryPipeline>>,
}

impl Reply {
    pub(crate) fn new(
        bot: BotHandle,
        target: MessageTarget,
        reply_to: Option<oxidebot_core::MessageId>,
        pipeline: Option<Arc<dyn ErasedDeliveryPipeline>>,
    ) -> Self {
        Self {
            bot,
            target,
            reply_to,
            fallback: FallbackPolicy::Auto,
            pipeline,
        }
    }

    /// Returns the target to which this sender will deliver messages.
    #[must_use]
    pub fn target(&self) -> &MessageTarget {
        &self.target
    }

    pub(crate) fn retarget(&self, target: MessageTarget) -> Self {
        Self {
            bot: self.bot.clone(),
            target,
            reply_to: None,
            fallback: self.fallback,
            pipeline: self.pipeline.clone(),
        }
    }

    pub(crate) fn rebind(&self, bot: BotHandle, target: MessageTarget) -> Self {
        Self {
            bot,
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

    /// Sends a message to this reply target without attaching a reply reference.
    pub async fn send(&self, message: impl Into<Message>) -> Result<Receipt, HandlerError> {
        self.send_inner(message.into(), false).await
    }

    /// Sends a message as a reply when the triggering event has a message reference.
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
                .deliver(&self.bot, self.target.clone(), message, self.fallback)
                .await?
        } else {
            self.bot
                .send_outgoing_message_with(self.target.clone(), message, self.fallback)
                .await
                .map_err(HandlerError::from)?
        };
        Ok(Receipt {
            bot: self.bot.clone(),
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
    bot: BotHandle,
    target: MessageTarget,
    report: Arc<DeliveryReport>,
    pipeline: Option<Arc<dyn ErasedDeliveryPipeline>>,
}

impl Receipt {
    /// Returns the adapter delivery report for this logical send.
    #[must_use]
    pub fn report(&self) -> &DeliveryReport {
        &self.report
    }

    /// Returns every physical message reference produced by this logical send.
    #[must_use]
    pub fn references(&self) -> &[MessageRef] {
        &self.report.messages
    }

    /// Iterates over the identifiers of every delivered physical message.
    pub fn ids(&self) -> impl ExactSizeIterator<Item = &oxidebot_core::MessageId> {
        self.report.messages.iter().map(|message| &message.id)
    }

    /// Returns whether capability fallback degraded this delivery.
    #[must_use]
    pub fn degraded(&self) -> bool {
        self.report.degraded()
    }

    /// Returns the capability degradations recorded for this delivery.
    #[must_use]
    pub fn degradations(&self) -> &[DeliveryDegradation] {
        &self.report.degradations
    }

    /// Returns the capabilities of the adapter that delivered this receipt.
    #[must_use]
    pub fn capabilities(&self) -> BotCapabilities {
        self.bot.bot_capabilities().unwrap_or_default()
    }

    /// Returns whether this adapter supports editing delivered messages.
    #[must_use]
    pub fn is_editable(&self) -> bool {
        self.capabilities().delivery.edit_messages.is_supported()
    }

    /// Returns whether this adapter supports deleting delivered messages.
    #[must_use]
    pub fn is_deletable(&self) -> bool {
        self.capabilities().delivery.delete_messages.is_supported()
    }

    /// Returns whether this adapter supports adding reactions to delivered messages.
    #[must_use]
    pub fn is_reactionable(&self) -> bool {
        self.capabilities().collaboration.reactions.is_supported()
    }

    /// Sends another message to the same target as this receipt.
    pub async fn send(&self, message: impl Into<Message>) -> Result<Receipt, HandlerError> {
        let message = message.into();
        let report = if let Some(pipeline) = &self.pipeline {
            pipeline
                .deliver(
                    &self.bot,
                    self.target.clone(),
                    message,
                    FallbackPolicy::Auto,
                )
                .await?
        } else {
            self.bot
                .send_outgoing_message_with(self.target.clone(), message, FallbackPolicy::Auto)
                .await
                .map_err(HandlerError::from)?
        };
        Ok(Self {
            bot: self.bot.clone(),
            target: self.target.clone(),
            report: Arc::new(report),
            pipeline: self.pipeline.clone(),
        })
    }

    /// Sends a reply to the last physical message in this receipt.
    pub async fn reply(&self, message: impl Into<Message>) -> Result<Receipt, HandlerError> {
        let Some(reference) = self.references().last() else {
            return Err(HandlerError::Api(
                "receipt contains no message reference".into(),
            ));
        };
        self.send(message.into().reply_to(reference.id.clone()))
            .await
    }

    /// Replaces every physical message, returning an error if any edit fails.
    pub async fn edit(&self, replacement: impl Into<Message>) -> Result<(), HandlerError> {
        self.edit_report(replacement).await.into_result("edit")
    }

    /// Edits every physical message and retains all per-item outcomes instead
    /// of stopping after the first failure.
    pub async fn edit_report(&self, replacement: impl Into<Message>) -> MutationReport {
        let replacement = replacement.into();
        let mut items = Vec::with_capacity(self.references().len());
        for (index, reference) in self.references().iter().enumerate() {
            let error = self
                .bot
                .edit_outgoing_message(reference.clone(), replacement.clone())
                .await
                .err()
                .map(|error| error.to_string());
            items.push(MutationItemResult {
                index,
                message: reference.clone(),
                error,
            });
        }
        MutationReport { items }
    }

    /// Replaces the physical message at `index`.
    pub async fn edit_at(
        &self,
        index: usize,
        replacement: impl Into<Message>,
    ) -> Result<(), HandlerError> {
        let reference = self.reference_at(index)?;
        self.bot
            .edit_outgoing_message(reference.clone(), replacement.into())
            .await
            .map_err(HandlerError::from)
    }

    /// Deletes every physical message, returning an error if any deletion fails.
    pub async fn delete(&self) -> Result<(), HandlerError> {
        self.delete_report().await.into_result("delete")
    }

    /// Deletes every physical message and retains partial success.
    pub async fn delete_report(&self) -> MutationReport {
        let mut items = Vec::with_capacity(self.references().len());
        for (index, reference) in self.references().iter().enumerate() {
            let error = self
                .bot
                .delete_message_ref(reference.clone())
                .await
                .err()
                .map(|error| error.to_string());
            items.push(MutationItemResult {
                index,
                message: reference.clone(),
                error,
            });
        }
        MutationReport { items }
    }

    /// Deletes the physical message at `index`.
    pub async fn delete_at(&self, index: usize) -> Result<(), HandlerError> {
        let reference = self.reference_at(index)?;
        self.bot
            .delete_message_ref(reference.clone())
            .await
            .map_err(HandlerError::from)
    }

    /// Adds a Unicode-emoji reaction to every physical message in the receipt.
    pub async fn react(&self, reaction: impl Into<String>) -> Result<(), HandlerError> {
        self.react_report(reaction).await.into_result("reaction")
    }

    /// Adds a reaction to every physical message and retains partial success.
    pub async fn react_report(&self, reaction: impl Into<String>) -> MutationReport {
        let reaction = Reaction::UnicodeEmoji(reaction.into());
        let mut items = Vec::with_capacity(self.references().len());
        for (index, reference) in self.references().iter().enumerate() {
            let error = self
                .bot
                .add_message_reaction(
                    reference.clone(),
                    reaction.clone(),
                    ReactionOptions::default(),
                )
                .await
                .err()
                .map(|error| error.to_string());
            items.push(MutationItemResult {
                index,
                message: reference.clone(),
                error,
            });
        }
        MutationReport { items }
    }

    /// Adds a Unicode-emoji reaction to the physical message at `index`.
    pub async fn react_at(
        &self,
        index: usize,
        reaction: impl Into<String>,
    ) -> Result<(), HandlerError> {
        let reference = self.reference_at(index)?;
        self.bot
            .add_message_reaction(
                reference.clone(),
                Reaction::UnicodeEmoji(reaction.into()),
                ReactionOptions::default(),
            )
            .await
            .map_err(HandlerError::from)
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
