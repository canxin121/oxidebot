use std::{sync::Arc, time::Duration};

use thiserror::Error;

pub mod platform;

use crate::interaction::{
    BotCommand, BotCommandQuery, BotCommandSet, ChatMenu, InteractionResponse,
    InteractionResponseHandle, MessageComponents, UnsupportedInteractionError,
};
use crate::{
    application::{
        AppSurface, BotProfile, CommandDefinition, MiniAppEvent, Suggestion, SuggestionRequest,
    },
    capability::{BotCapabilities, UnsupportedFeatureError},
    collaboration::{
        CallOptions, CallSession, ChatActivity, PinOptions, PinnedMessage, Reaction,
        ReactionOptions,
    },
    commerce::{CheckoutRequest, Invoice, InvoiceOptions, Payment, ShippingOption},
    content::{
        BatchMessage, BatchSendResult, Checklist, ForwardOptions, MessageEnvelope, MessageQuery,
        Poll,
    },
    conversation::{
        ConversationMember, ConversationProfile, ConversationRef, InviteLink, InviteLinkOptions,
        MessageRef, MessageTarget, Page, PageRequest, PermissionSet, Thread, ThreadOptions,
    },
    event::RequestDecision,
    source::message::{DeliveryItemResult, DeliveryPlan, DeliveryReport, FallbackPolicy, Message},
};
use platform::{PlatformApiRequest, PlatformApiResponse, UnsupportedPlatformApiError};

/// Result returned by every portable or platform-native adapter call.
pub type CallResult<T> = std::result::Result<T, CallError>;

/// Typed adapter-call failure used by the runtime scheduler and applications.
///
/// Adapters must classify transport failures explicitly. This prevents an
/// unknown erased error from silently becoming a permanent failure and makes
/// retry, rate-limit, partial-delivery, and unsupported-feature handling
/// deterministic across every API method.
#[derive(Debug, Error)]
pub enum CallError {
    #[error("temporary adapter failure: {message}")]
    Temporary { message: Arc<str> },
    #[error("adapter rate limited the call: {message}")]
    RateLimited {
        message: Arc<str>,
        retry_after: Option<Duration>,
    },
    #[error("adapter call timed out: {message}")]
    Timeout { message: Arc<str> },
    #[error("adapter rejected the request: {message}")]
    InvalidRequest { message: Arc<str> },
    #[error("adapter could not find the requested resource: {message}")]
    NotFound { message: Arc<str> },
    #[error("adapter does not support {feature}")]
    Unsupported { feature: Arc<str> },
    #[error("adapter call failed permanently: {message}")]
    Permanent { message: Arc<str> },
    #[error(transparent)]
    Planning(#[from] crate::source::message::DeliveryPlanningError),
    #[error(transparent)]
    PartialDelivery(#[from] crate::source::message::PartialDeliveryError),
}

impl CallError {
    #[must_use]
    pub fn temporary(message: impl Into<Arc<str>>) -> Self {
        Self::Temporary {
            message: message.into(),
        }
    }

    #[must_use]
    pub fn rate_limited(message: impl Into<Arc<str>>, retry_after: Option<Duration>) -> Self {
        Self::RateLimited {
            message: message.into(),
            retry_after,
        }
    }

    #[must_use]
    pub fn timeout(message: impl Into<Arc<str>>) -> Self {
        Self::Timeout {
            message: message.into(),
        }
    }

    #[must_use]
    pub fn invalid_request(message: impl Into<Arc<str>>) -> Self {
        Self::InvalidRequest {
            message: message.into(),
        }
    }

    #[must_use]
    pub fn not_found(message: impl Into<Arc<str>>) -> Self {
        Self::NotFound {
            message: message.into(),
        }
    }

    #[must_use]
    pub fn unsupported(feature: impl Into<Arc<str>>) -> Self {
        Self::Unsupported {
            feature: feature.into(),
        }
    }

    #[must_use]
    pub fn permanent(message: impl Into<Arc<str>>) -> Self {
        Self::Permanent {
            message: message.into(),
        }
    }

    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Temporary { .. } | Self::RateLimited { .. } | Self::Timeout { .. }
        )
    }

    #[must_use]
    pub const fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::RateLimited { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

impl From<UnsupportedFeatureError> for CallError {
    fn from(value: UnsupportedFeatureError) -> Self {
        Self::unsupported(value.feature)
    }
}

impl From<UnsupportedInteractionError> for CallError {
    fn from(value: UnsupportedInteractionError) -> Self {
        Self::unsupported(value.feature)
    }
}

impl From<UnsupportedPlatformApiError> for CallError {
    fn from(value: UnsupportedPlatformApiError) -> Self {
        Self::unsupported(format!("platform-native API method {:?}", value.method))
    }
}

/// Incrementally builds a complete [`DeliveryReport`] while executing one
/// previously planned logical delivery.
///
/// An adapter must report every physical message in plan order, including the
/// successful prefix when a later send fails. This helper centralizes that
/// otherwise easy-to-get-wrong invariant and produces a typed
/// [`CallError::PartialDelivery`] when appropriate.
#[derive(Debug)]
pub struct DeliveryReportBuilder {
    degradations: Vec<crate::source::message::DeliveryDegradation>,
    messages: Vec<MessageRef>,
    items: Vec<DeliveryItemResult>,
    expected: usize,
}

impl DeliveryReportBuilder {
    /// Starts a report for exactly the physical messages in `plan`.
    #[must_use]
    pub fn new(plan: &DeliveryPlan) -> Self {
        Self {
            degradations: plan.degradations.clone(),
            messages: Vec::new(),
            items: Vec::with_capacity(plan.messages.len()),
            expected: plan.messages.len(),
        }
    }

    /// Records successful delivery of one planned physical message.
    pub fn delivered(&mut self, messages: impl IntoIterator<Item = MessageRef>) {
        let messages = messages.into_iter().collect::<Vec<_>>();
        self.messages.extend(messages.iter().cloned());
        self.items.push(DeliveryItemResult {
            index: self.items.len(),
            messages,
            error: None,
        });
    }

    /// Records a failed planned physical message and returns the typed partial
    /// result containing every earlier successful reference.
    #[must_use]
    pub fn failed(mut self, error: impl Into<String>) -> CallError {
        self.items.push(DeliveryItemResult {
            index: self.items.len(),
            messages: Vec::new(),
            error: Some(error.into()),
        });
        CallError::PartialDelivery(crate::source::message::PartialDeliveryError {
            report: DeliveryReport {
                messages: self.messages,
                degradations: self.degradations,
                items: self.items,
            },
        })
    }

    /// Finishes an all-successful delivery. A missing or extra physical item
    /// is an adapter contract error rather than a silently malformed report.
    pub fn finish(self) -> CallResult<DeliveryReport> {
        if self.items.len() != self.expected {
            return Err(CallError::permanent(format!(
                "delivery report contains {} physical results for {} planned messages",
                self.items.len(),
                self.expected
            )));
        }
        Ok(DeliveryReport {
            messages: self.messages,
            degradations: self.degradations,
            items: self.items,
        })
    }
}

/// Canonical cross-platform operations implemented by a bot adapter.
#[async_trait::async_trait]
#[allow(unused_variables)]
pub trait CallApiTrait: Send + Sync {
    /// Calls a platform-native API without losing platform-specific fields.
    ///
    /// Adapters should implement this once to expose their complete native API.
    /// The common methods below remain the ergonomic cross-platform layer.
    async fn call_platform_api(
        &self,
        request: PlatformApiRequest,
    ) -> CallResult<PlatformApiResponse> {
        Err(UnsupportedPlatformApiError {
            method: request.method,
        }
        .into())
    }

    /// Returns the platform-native methods explicitly known by this adapter.
    /// Calls are not required to be limited to this list, which keeps adapters
    /// open to newly released platform methods.
    fn platform_api_methods(&self) -> &'static [&'static str] {
        &[]
    }

    fn supports_platform_api_method(&self, method: &str) -> bool {
        self.platform_api_methods().contains(&method)
    }

    /// Returns granular capabilities and platform limits for this adapter.
    fn bot_capabilities(&self) -> BotCapabilities {
        BotCapabilities::default()
    }

    /// Builds the exact physical message plan that would be sent by
    /// [`CallApiTrait::send_outgoing_message_with`]. Planning is pure and can be
    /// used by previews, tests, observability, and strict-delivery workflows.
    ///
    /// `target` is part of the planning input because platforms commonly vary
    /// permissions, limits, and supported presentation features by direct
    /// message, conversation, thread, or interaction context. Implementations
    /// that only expose bot-wide capabilities may intentionally ignore it.
    fn plan_outgoing_message(
        &self,
        _target: &MessageTarget,
        message: &Message,
        policy: FallbackPolicy,
    ) -> CallResult<DeliveryPlan> {
        Ok(message.plan_for(&self.bot_capabilities(), policy)?)
    }

    /// Sends the canonical message IR through the capability-aware delivery
    /// planner and returns both physical message references and every
    /// degradation that occurred.
    ///
    async fn send_outgoing_message_with(
        &self,
        target: MessageTarget,
        message: Message,
        policy: FallbackPolicy,
    ) -> CallResult<DeliveryReport> {
        let plan = self.plan_outgoing_message(&target, &message, policy)?;
        self.send_delivery_plan(target, plan).await
    }

    /// Executes a previously inspected or middleware-transformed delivery plan.
    /// This is the shared transport boundary used by the runtime delivery
    /// pipeline, previews, and strict-delivery workflows.
    async fn send_delivery_plan(
        &self,
        target: MessageTarget,
        plan: DeliveryPlan,
    ) -> CallResult<DeliveryReport> {
        Err(UnsupportedFeatureError::new("sending messages").into())
    }

    /// Sends a fully modeled message with the safe automatic fallback policy.
    async fn send_outgoing_message(
        &self,
        target: MessageTarget,
        message: Message,
    ) -> CallResult<Vec<MessageRef>> {
        Ok(self
            .send_outgoing_message_with(target, message, FallbackPolicy::Auto)
            .await?
            .messages)
    }

    async fn edit_outgoing_message(
        &self,
        message: MessageRef,
        new_message: Message,
    ) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("editing messages").into())
    }

    async fn delete_message_ref(&self, message: MessageRef) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("deleting messages").into())
    }

    async fn delete_messages(&self, messages: Vec<MessageRef>) -> CallResult<()> {
        for message in messages {
            self.delete_message_ref(message).await?;
        }
        Ok(())
    }

    async fn edit_message_components(
        &self,
        message_id: String,
        components: Option<MessageComponents>,
    ) -> CallResult<()> {
        Err(UnsupportedInteractionError::new("editing message components").into())
    }

    async fn answer_interaction(
        &self,
        interaction_id: String,
        response: InteractionResponse,
    ) -> CallResult<()> {
        Err(UnsupportedInteractionError::new("answering interactions").into())
    }

    async fn set_bot_commands(&self, commands: BotCommandSet) -> CallResult<()> {
        Err(UnsupportedInteractionError::new("bot commands").into())
    }

    async fn get_bot_commands(&self, query: BotCommandQuery) -> CallResult<Vec<BotCommand>> {
        Err(UnsupportedInteractionError::new("bot commands").into())
    }

    async fn delete_bot_commands(&self, query: BotCommandQuery) -> CallResult<()> {
        Err(UnsupportedInteractionError::new("bot commands").into())
    }

    async fn set_chat_menu(&self, chat_id: Option<String>, menu: ChatMenu) -> CallResult<()> {
        Err(UnsupportedInteractionError::new("chat menu").into())
    }

    async fn get_chat_menu(&self, chat_id: Option<String>) -> CallResult<ChatMenu> {
        Err(UnsupportedInteractionError::new("chat menu").into())
    }

    async fn defer_interaction(
        &self,
        handle: InteractionResponseHandle,
        visibility: crate::interaction::InteractionVisibility,
    ) -> CallResult<()> {
        self.answer_interaction(handle.id, InteractionResponse::Defer { visibility })
            .await
    }

    async fn send_interaction_followup(
        &self,
        handle: InteractionResponseHandle,
        message: Message,
    ) -> CallResult<Vec<MessageRef>> {
        Err(UnsupportedInteractionError::new("interaction follow-up messages").into())
    }

    async fn edit_interaction_response(
        &self,
        handle: InteractionResponseHandle,
        message: Message,
    ) -> CallResult<()> {
        Err(UnsupportedInteractionError::new("editing the original interaction response").into())
    }

    async fn delete_interaction_response(
        &self,
        handle: InteractionResponseHandle,
    ) -> CallResult<()> {
        Err(UnsupportedInteractionError::new("deleting the original interaction response").into())
    }

    async fn set_message_reactions(
        &self,
        message: MessageRef,
        reactions: Vec<Reaction>,
    ) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("setting message reactions").into())
    }

    async fn add_message_reaction(
        &self,
        message: MessageRef,
        reaction: Reaction,
        options: ReactionOptions,
    ) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("adding message reactions").into())
    }

    async fn remove_message_reaction(
        &self,
        message: MessageRef,
        reaction: Reaction,
    ) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("removing message reactions").into())
    }

    async fn clear_message_reactions(&self, message: MessageRef) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("clearing message reactions").into())
    }

    async fn list_reaction_users(
        &self,
        message: MessageRef,
        reaction: Reaction,
        page: PageRequest,
    ) -> CallResult<Page<crate::source::user::User>> {
        Err(UnsupportedFeatureError::new("listing reaction users").into())
    }

    async fn pin_message(&self, message: MessageRef, options: PinOptions) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("pinning messages").into())
    }

    async fn unpin_message(&self, message: MessageRef) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("unpinning messages").into())
    }

    async fn clear_pins(&self, conversation: ConversationRef) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("clearing pinned messages").into())
    }

    async fn list_pins(
        &self,
        conversation: ConversationRef,
        page: PageRequest,
    ) -> CallResult<Page<PinnedMessage>> {
        Err(UnsupportedFeatureError::new("listing pinned messages").into())
    }

    async fn set_chat_activity(
        &self,
        conversation: ConversationRef,
        activity: ChatActivity,
        duration: Option<Duration>,
    ) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("chat activity indicators").into())
    }

    async fn mark_read(
        &self,
        conversation: ConversationRef,
        through_message: Option<MessageRef>,
    ) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("read receipts").into())
    }

    async fn create_call(
        &self,
        conversation: ConversationRef,
        options: CallOptions,
    ) -> CallResult<CallSession> {
        Err(UnsupportedFeatureError::new("creating calls or meetings").into())
    }

    async fn end_call(&self, call: CallSession) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("ending calls or meetings").into())
    }

    async fn list_call_participants(
        &self,
        call: CallSession,
        page: PageRequest,
    ) -> CallResult<Page<crate::source::user::User>> {
        Err(UnsupportedFeatureError::new("listing call participants").into())
    }

    async fn stop_poll(&self, message: MessageRef) -> CallResult<Poll> {
        Err(UnsupportedFeatureError::new("stopping polls").into())
    }

    async fn edit_checklist(&self, message: MessageRef, checklist: Checklist) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("editing checklists").into())
    }

    async fn create_thread(
        &self,
        parent: ConversationRef,
        options: ThreadOptions,
    ) -> CallResult<Thread> {
        Err(UnsupportedFeatureError::new("creating threads or topics").into())
    }

    async fn edit_thread(
        &self,
        thread: ConversationRef,
        options: ThreadOptions,
    ) -> CallResult<Thread> {
        Err(UnsupportedFeatureError::new("editing threads or topics").into())
    }

    async fn close_thread(&self, thread: ConversationRef) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("closing threads or topics").into())
    }

    async fn reopen_thread(&self, thread: ConversationRef) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("reopening threads or topics").into())
    }

    async fn delete_thread(&self, thread: ConversationRef) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("deleting threads or topics").into())
    }

    async fn list_threads(
        &self,
        parent: ConversationRef,
        page: PageRequest,
    ) -> CallResult<Page<Thread>> {
        Err(UnsupportedFeatureError::new("listing threads or topics").into())
    }

    async fn get_conversation_profile(
        &self,
        conversation: ConversationRef,
    ) -> CallResult<ConversationProfile> {
        Err(UnsupportedFeatureError::new("conversation profiles").into())
    }

    async fn set_conversation_profile(
        &self,
        conversation: ConversationRef,
        profile: ConversationProfile,
    ) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("conversation profiles").into())
    }

    async fn get_conversation_member(
        &self,
        conversation: ConversationRef,
        user_id: String,
    ) -> CallResult<ConversationMember> {
        Err(UnsupportedFeatureError::new("conversation members").into())
    }

    async fn list_conversation_members(
        &self,
        conversation: ConversationRef,
        page: PageRequest,
    ) -> CallResult<Page<ConversationMember>> {
        Err(UnsupportedFeatureError::new("conversation members").into())
    }

    async fn set_member_permissions(
        &self,
        conversation: ConversationRef,
        user_id: String,
        permissions: PermissionSet,
    ) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("member permissions").into())
    }

    async fn set_default_permissions(
        &self,
        conversation: ConversationRef,
        permissions: PermissionSet,
    ) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("default conversation permissions").into())
    }

    async fn set_member_tag(
        &self,
        conversation: ConversationRef,
        user_id: String,
        tag: Option<String>,
    ) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("member tags or labels").into())
    }

    async fn remove_conversation_member(
        &self,
        conversation: ConversationRef,
        user_id: String,
    ) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("removing conversation members").into())
    }

    async fn ban_conversation_member(
        &self,
        conversation: ConversationRef,
        user_id: String,
        duration: Option<Duration>,
        delete_history: bool,
    ) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("banning conversation members").into())
    }

    async fn unban_conversation_member(
        &self,
        conversation: ConversationRef,
        user_id: String,
    ) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("unbanning conversation members").into())
    }

    async fn approve_conversation_join_request(
        &self,
        conversation: ConversationRef,
        user_id: String,
    ) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("approving conversation join requests").into())
    }

    async fn decline_conversation_join_request(
        &self,
        conversation: ConversationRef,
        user_id: String,
    ) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("declining conversation join requests").into())
    }

    async fn leave_conversation(&self, conversation: ConversationRef) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("leaving conversations").into())
    }

    async fn create_invite_link(
        &self,
        conversation: ConversationRef,
        options: InviteLinkOptions,
    ) -> CallResult<InviteLink> {
        Err(UnsupportedFeatureError::new("conversation invite links").into())
    }

    async fn edit_invite_link(
        &self,
        invite: InviteLink,
        options: InviteLinkOptions,
    ) -> CallResult<InviteLink> {
        Err(UnsupportedFeatureError::new("conversation invite links").into())
    }

    async fn revoke_invite_link(&self, invite: InviteLink) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("conversation invite links").into())
    }

    async fn list_messages(&self, query: MessageQuery) -> CallResult<Page<MessageEnvelope>> {
        Err(UnsupportedFeatureError::new("message history").into())
    }

    async fn forward_messages(
        &self,
        messages: Vec<MessageRef>,
        target: MessageTarget,
        options: ForwardOptions,
    ) -> CallResult<Vec<MessageRef>> {
        Err(UnsupportedFeatureError::new("forwarding messages").into())
    }

    async fn copy_messages(
        &self,
        messages: Vec<MessageRef>,
        target: MessageTarget,
        options: ForwardOptions,
    ) -> CallResult<Vec<MessageRef>> {
        Err(UnsupportedFeatureError::new("copying messages").into())
    }

    async fn send_batch(&self, messages: Vec<BatchMessage>) -> CallResult<BatchSendResult> {
        let mut result = BatchSendResult::default();
        for (index, item) in messages.into_iter().enumerate() {
            match self.send_outgoing_message(item.target, item.message).await {
                Ok(messages) => {
                    for message in messages {
                        result.items.push(crate::content::BatchItemResult {
                            index,
                            message: Some(message),
                            error: None,
                        });
                    }
                }
                Err(error) => result.items.push(crate::content::BatchItemResult {
                    index,
                    message: None,
                    error: Some(error.to_string()),
                }),
            }
        }
        Ok(result)
    }

    async fn set_command_definitions(&self, commands: Vec<CommandDefinition>) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("structured application commands").into())
    }

    async fn get_command_definitions(
        &self,
        conversation: Option<ConversationRef>,
    ) -> CallResult<Vec<CommandDefinition>> {
        Err(UnsupportedFeatureError::new("structured application commands").into())
    }

    async fn delete_command_definitions(
        &self,
        conversation: Option<ConversationRef>,
    ) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("structured application commands").into())
    }

    async fn answer_suggestion_request(
        &self,
        request: SuggestionRequest,
        suggestions: Vec<Suggestion>,
        next_cursor: Option<String>,
    ) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("dynamic suggestions or inline queries").into())
    }

    async fn publish_surface(&self, surface: AppSurface) -> CallResult<AppSurface> {
        Err(UnsupportedFeatureError::new("application surfaces").into())
    }

    async fn delete_surface(&self, surface: AppSurface) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("application surfaces").into())
    }

    async fn answer_mini_app_query(
        &self,
        event: MiniAppEvent,
        message: Message,
    ) -> CallResult<Vec<MessageRef>> {
        Err(UnsupportedFeatureError::new("answering mini-app queries").into())
    }

    async fn set_bot_profile(&self, profile: BotProfile) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("localized bot profiles").into())
    }

    async fn get_bot_profile(&self) -> CallResult<BotProfile> {
        Err(UnsupportedFeatureError::new("localized bot profiles").into())
    }

    async fn respond_to_request(
        &self,
        request_id: String,
        decision: RequestDecision,
    ) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("responding to requests").into())
    }

    async fn send_invoice(
        &self,
        target: MessageTarget,
        invoice: Invoice,
        options: InvoiceOptions,
    ) -> CallResult<MessageRef> {
        Err(UnsupportedFeatureError::new("invoices").into())
    }

    async fn answer_shipping_request(
        &self,
        request_id: String,
        options: Vec<ShippingOption>,
        error: Option<String>,
    ) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("shipping requests").into())
    }

    async fn answer_checkout_request(
        &self,
        request: CheckoutRequest,
        approved: bool,
        error: Option<String>,
    ) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("checkout requests").into())
    }

    async fn refund_payment(&self, payment: Payment) -> CallResult<()> {
        Err(UnsupportedFeatureError::new("payment refunds").into())
    }
}

#[cfg(test)]
mod partial_delivery_tests {
    use super::*;
    use crate::source::message::{DeliveryItemResult, Message, PartialDeliveryError};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct FailSecondApi(AtomicUsize);

    #[async_trait::async_trait]
    impl CallApiTrait for FailSecondApi {
        async fn send_delivery_plan(
            &self,
            target: MessageTarget,
            plan: DeliveryPlan,
        ) -> CallResult<DeliveryReport> {
            let mut messages = Vec::new();
            let mut items = Vec::with_capacity(plan.messages.len());
            for (index, _) in plan.messages.into_iter().enumerate() {
                let attempt = self.0.fetch_add(1, Ordering::AcqRel);
                if attempt == 1 {
                    items.push(DeliveryItemResult {
                        index,
                        messages: Vec::new(),
                        error: Some("second physical message failed".to_owned()),
                    });
                    return Err(PartialDeliveryError {
                        report: DeliveryReport {
                            messages,
                            degradations: plan.degradations,
                            items,
                        },
                    }
                    .into());
                }
                let sent = MessageRef::new(format!("message-{attempt}"))
                    .in_conversation(target.conversation.clone());
                messages.push(sent.clone());
                items.push(DeliveryItemResult {
                    index,
                    messages: vec![sent],
                    error: None,
                });
            }
            Ok(DeliveryReport {
                messages,
                degradations: plan.degradations,
                items,
            })
        }
    }

    #[tokio::test]
    async fn partial_delivery_error_retains_successful_message_refs() {
        let api = FailSecondApi::default();
        let target = MessageTarget::new(ConversationRef::group("room"));
        let plan = DeliveryPlan {
            messages: vec![Message::text("first"), Message::text("second")],
            degradations: Vec::new(),
        };
        let error = api
            .send_delivery_plan(target, plan)
            .await
            .expect_err("second physical send fails");
        let CallError::PartialDelivery(partial) = error else {
            panic!("partial report is retained in the typed call error");
        };
        assert_eq!(partial.report.messages.len(), 1);
        assert_eq!(partial.report.items.len(), 2);
        assert!(partial.report.items[0].succeeded());
        assert!(!partial.report.items[1].succeeded());
    }
}
