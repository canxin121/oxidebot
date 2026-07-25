use std::time::Duration;

use anyhow::Result;

pub mod payload;
pub mod platform;
pub mod response;

use payload::{GroupAdminChangeType, GroupMuteType, RequestResponse, SendMessageTarget};
pub use response::{
    BotGetFriendListResponse, BotGetGroupListResponse, BotGetProfileResponse,
    GetMessageDetailResponse, GroupGetFileCountResponse, GroupGetFsListResponse,
    GroupGetProfileResponse, GroupMemberListResponse, SendMessageResponse, UserGetProfileResponse,
};

use crate::interaction::{
    BotCommand, BotCommandQuery, BotCommandSet, ChatMenu, InteractionCapabilities,
    InteractionResponse, InteractionResponseHandle, MessageComponents, MessageOptions,
    UnsupportedInteractionError,
};
use crate::source::{
    group::GroupProfile,
    message::{File, MessageSegment},
    user::UserProfile,
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
        OutgoingMessage, Poll,
    },
    conversation::{
        ConversationMember, ConversationProfile, ConversationRef, InviteLink, InviteLinkOptions,
        MessageRef, MessageTarget, Page, PageRequest, PermissionSet, Thread, ThreadOptions,
    },
};
use platform::{PlatformApiRequest, PlatformApiResponse, UnsupportedPlatformApiError};

/// CallApiTrait is a trait that defines the methods that a bot should implement to interact with the API.
/// If the bot does not implement the method, it will return an error.
#[async_trait::async_trait]
#[allow(unused_variables)]
pub trait CallApiTrait {
    /// Calls a platform-native API without losing platform-specific fields.
    ///
    /// Adapters should implement this once to expose their complete native API.
    /// The common methods below remain the ergonomic cross-platform layer.
    async fn call_platform_api(&self, request: PlatformApiRequest) -> Result<PlatformApiResponse> {
        Err(UnsupportedPlatformApiError {
            method: request.method,
        }
        .into())
    }

    /// Returns the platform-native methods explicitly known by this adapter.
    /// Calls are not required to be limited to this list, which keeps adapters
    /// forward-compatible with newly released platform methods.
    fn platform_api_methods(&self) -> &'static [&'static str] {
        &[]
    }

    fn supports_platform_api_method(&self, method: &str) -> bool {
        self.platform_api_methods().contains(&method)
    }

    /// Returns the high-level interactive capabilities implemented by this
    /// adapter. Platform-native APIs can expose additional capabilities.
    fn interaction_capabilities(&self) -> InteractionCapabilities {
        InteractionCapabilities::default()
    }

    /// Returns granular capabilities and platform limits. This is the
    /// preferred capability API for new adapters.
    fn bot_capabilities(&self) -> BotCapabilities {
        BotCapabilities::default()
    }

    /// Sends a fully modeled message. The default converts portable basic
    /// content to the legacy API, keeping existing adapters source-compatible.
    async fn send_outgoing_message(
        &self,
        target: MessageTarget,
        message: OutgoingMessage,
    ) -> Result<Vec<MessageRef>> {
        let legacy_target = legacy_send_target(&target)?;
        let conversation = target.conversation;
        let (segments, options) = message.try_into_legacy()?;
        Ok(self
            .send_message_with_options(segments, legacy_target, options)
            .await?
            .into_iter()
            .map(|response| {
                MessageRef::new(response.sent_message_id).in_conversation(conversation.clone())
            })
            .collect())
    }

    async fn edit_outgoing_message(
        &self,
        message: MessageRef,
        new_message: OutgoingMessage,
    ) -> Result<()> {
        let (segments, options) = new_message.try_into_legacy()?;
        if !options.is_empty() {
            return Err(UnsupportedFeatureError::new("editing message options atomically").into());
        }
        self.edit_message(message.id, segments).await
    }

    async fn delete_message_ref(&self, message: MessageRef) -> Result<()> {
        self.delete_message(message.id).await
    }

    async fn delete_messages(&self, messages: Vec<MessageRef>) -> Result<()> {
        for message in messages {
            self.delete_message_ref(message).await?;
        }
        Ok(())
    }

    /// Sends a message with interactive components while keeping the original
    /// `send_message` API source-compatible for existing adapters.
    async fn send_message_with_options(
        &self,
        message: Vec<MessageSegment>,
        target: SendMessageTarget,
        options: MessageOptions,
    ) -> Result<Vec<SendMessageResponse>> {
        if options.is_empty() {
            self.send_message(message, target).await
        } else {
            Err(UnsupportedInteractionError::new("message components").into())
        }
    }

    async fn edit_message_components(
        &self,
        message_id: String,
        components: Option<MessageComponents>,
    ) -> Result<()> {
        Err(UnsupportedInteractionError::new("editing message components").into())
    }

    async fn answer_interaction(
        &self,
        interaction_id: String,
        response: InteractionResponse,
    ) -> Result<()> {
        Err(UnsupportedInteractionError::new("answering interactions").into())
    }

    async fn set_bot_commands(&self, commands: BotCommandSet) -> Result<()> {
        Err(UnsupportedInteractionError::new("bot commands").into())
    }

    async fn get_bot_commands(&self, query: BotCommandQuery) -> Result<Vec<BotCommand>> {
        Err(UnsupportedInteractionError::new("bot commands").into())
    }

    async fn delete_bot_commands(&self, query: BotCommandQuery) -> Result<()> {
        Err(UnsupportedInteractionError::new("bot commands").into())
    }

    async fn set_chat_menu(&self, chat_id: Option<String>, menu: ChatMenu) -> Result<()> {
        Err(UnsupportedInteractionError::new("chat menu").into())
    }

    async fn get_chat_menu(&self, chat_id: Option<String>) -> Result<ChatMenu> {
        Err(UnsupportedInteractionError::new("chat menu").into())
    }

    async fn defer_interaction(
        &self,
        handle: InteractionResponseHandle,
        visibility: crate::interaction::InteractionVisibility,
    ) -> Result<()> {
        self.answer_interaction(handle.id, InteractionResponse::Defer { visibility })
            .await
    }

    async fn send_interaction_followup(
        &self,
        handle: InteractionResponseHandle,
        message: OutgoingMessage,
    ) -> Result<Vec<MessageRef>> {
        Err(UnsupportedInteractionError::new("interaction follow-up messages").into())
    }

    async fn edit_interaction_response(
        &self,
        handle: InteractionResponseHandle,
        message: OutgoingMessage,
    ) -> Result<()> {
        Err(UnsupportedInteractionError::new("editing the original interaction response").into())
    }

    async fn delete_interaction_response(&self, handle: InteractionResponseHandle) -> Result<()> {
        Err(UnsupportedInteractionError::new("deleting the original interaction response").into())
    }

    async fn set_message_reactions(
        &self,
        message: MessageRef,
        reactions: Vec<Reaction>,
    ) -> Result<()> {
        if let [reaction] = reactions.as_slice() {
            self.set_message_reaction(message.id, legacy_reaction_id(reaction)?)
                .await
        } else {
            Err(UnsupportedFeatureError::new("setting multiple message reactions").into())
        }
    }

    async fn add_message_reaction(
        &self,
        message: MessageRef,
        reaction: Reaction,
        options: ReactionOptions,
    ) -> Result<()> {
        self.set_message_reaction(message.id, legacy_reaction_id(&reaction)?)
            .await
    }

    async fn remove_message_reaction(&self, message: MessageRef, reaction: Reaction) -> Result<()> {
        Err(UnsupportedFeatureError::new("removing message reactions").into())
    }

    async fn clear_message_reactions(&self, message: MessageRef) -> Result<()> {
        Err(UnsupportedFeatureError::new("clearing message reactions").into())
    }

    async fn list_reaction_users(
        &self,
        message: MessageRef,
        reaction: Reaction,
        page: PageRequest,
    ) -> Result<Page<crate::source::user::User>> {
        Err(UnsupportedFeatureError::new("listing reaction users").into())
    }

    async fn pin_message(&self, message: MessageRef, options: PinOptions) -> Result<()> {
        Err(UnsupportedFeatureError::new("pinning messages").into())
    }

    async fn unpin_message(&self, message: MessageRef) -> Result<()> {
        Err(UnsupportedFeatureError::new("unpinning messages").into())
    }

    async fn clear_pins(&self, conversation: ConversationRef) -> Result<()> {
        Err(UnsupportedFeatureError::new("clearing pinned messages").into())
    }

    async fn list_pins(
        &self,
        conversation: ConversationRef,
        page: PageRequest,
    ) -> Result<Page<PinnedMessage>> {
        Err(UnsupportedFeatureError::new("listing pinned messages").into())
    }

    async fn set_chat_activity(
        &self,
        conversation: ConversationRef,
        activity: ChatActivity,
        duration: Option<Duration>,
    ) -> Result<()> {
        Err(UnsupportedFeatureError::new("chat activity indicators").into())
    }

    async fn mark_read(
        &self,
        conversation: ConversationRef,
        through_message: Option<MessageRef>,
    ) -> Result<()> {
        Err(UnsupportedFeatureError::new("read receipts").into())
    }

    async fn create_call(
        &self,
        conversation: ConversationRef,
        options: CallOptions,
    ) -> Result<CallSession> {
        Err(UnsupportedFeatureError::new("creating calls or meetings").into())
    }

    async fn end_call(&self, call: CallSession) -> Result<()> {
        Err(UnsupportedFeatureError::new("ending calls or meetings").into())
    }

    async fn list_call_participants(
        &self,
        call: CallSession,
        page: PageRequest,
    ) -> Result<Page<crate::source::user::User>> {
        Err(UnsupportedFeatureError::new("listing call participants").into())
    }

    async fn stop_poll(&self, message: MessageRef) -> Result<Poll> {
        Err(UnsupportedFeatureError::new("stopping polls").into())
    }

    async fn edit_checklist(&self, message: MessageRef, checklist: Checklist) -> Result<()> {
        Err(UnsupportedFeatureError::new("editing checklists").into())
    }

    async fn create_thread(
        &self,
        parent: ConversationRef,
        options: ThreadOptions,
    ) -> Result<Thread> {
        Err(UnsupportedFeatureError::new("creating threads or topics").into())
    }

    async fn edit_thread(&self, thread: ConversationRef, options: ThreadOptions) -> Result<Thread> {
        Err(UnsupportedFeatureError::new("editing threads or topics").into())
    }

    async fn close_thread(&self, thread: ConversationRef) -> Result<()> {
        Err(UnsupportedFeatureError::new("closing threads or topics").into())
    }

    async fn reopen_thread(&self, thread: ConversationRef) -> Result<()> {
        Err(UnsupportedFeatureError::new("reopening threads or topics").into())
    }

    async fn delete_thread(&self, thread: ConversationRef) -> Result<()> {
        Err(UnsupportedFeatureError::new("deleting threads or topics").into())
    }

    async fn list_threads(
        &self,
        parent: ConversationRef,
        page: PageRequest,
    ) -> Result<Page<Thread>> {
        Err(UnsupportedFeatureError::new("listing threads or topics").into())
    }

    async fn get_conversation_profile(
        &self,
        conversation: ConversationRef,
    ) -> Result<ConversationProfile> {
        Err(UnsupportedFeatureError::new("conversation profiles").into())
    }

    async fn set_conversation_profile(
        &self,
        conversation: ConversationRef,
        profile: ConversationProfile,
    ) -> Result<()> {
        Err(UnsupportedFeatureError::new("conversation profiles").into())
    }

    async fn get_conversation_member(
        &self,
        conversation: ConversationRef,
        user_id: String,
    ) -> Result<ConversationMember> {
        Err(UnsupportedFeatureError::new("conversation members").into())
    }

    async fn list_conversation_members(
        &self,
        conversation: ConversationRef,
        page: PageRequest,
    ) -> Result<Page<ConversationMember>> {
        Err(UnsupportedFeatureError::new("conversation members").into())
    }

    async fn set_member_permissions(
        &self,
        conversation: ConversationRef,
        user_id: String,
        permissions: PermissionSet,
    ) -> Result<()> {
        Err(UnsupportedFeatureError::new("member permissions").into())
    }

    async fn set_default_permissions(
        &self,
        conversation: ConversationRef,
        permissions: PermissionSet,
    ) -> Result<()> {
        Err(UnsupportedFeatureError::new("default conversation permissions").into())
    }

    async fn set_member_tag(
        &self,
        conversation: ConversationRef,
        user_id: String,
        tag: Option<String>,
    ) -> Result<()> {
        Err(UnsupportedFeatureError::new("member tags or labels").into())
    }

    async fn remove_conversation_member(
        &self,
        conversation: ConversationRef,
        user_id: String,
    ) -> Result<()> {
        Err(UnsupportedFeatureError::new("removing conversation members").into())
    }

    async fn ban_conversation_member(
        &self,
        conversation: ConversationRef,
        user_id: String,
        duration: Option<Duration>,
        delete_history: bool,
    ) -> Result<()> {
        Err(UnsupportedFeatureError::new("banning conversation members").into())
    }

    async fn unban_conversation_member(
        &self,
        conversation: ConversationRef,
        user_id: String,
    ) -> Result<()> {
        Err(UnsupportedFeatureError::new("unbanning conversation members").into())
    }

    async fn approve_conversation_join_request(
        &self,
        conversation: ConversationRef,
        user_id: String,
    ) -> Result<()> {
        Err(UnsupportedFeatureError::new("approving conversation join requests").into())
    }

    async fn decline_conversation_join_request(
        &self,
        conversation: ConversationRef,
        user_id: String,
    ) -> Result<()> {
        Err(UnsupportedFeatureError::new("declining conversation join requests").into())
    }

    async fn leave_conversation(&self, conversation: ConversationRef) -> Result<()> {
        Err(UnsupportedFeatureError::new("leaving conversations").into())
    }

    async fn create_invite_link(
        &self,
        conversation: ConversationRef,
        options: InviteLinkOptions,
    ) -> Result<InviteLink> {
        Err(UnsupportedFeatureError::new("conversation invite links").into())
    }

    async fn edit_invite_link(
        &self,
        invite: InviteLink,
        options: InviteLinkOptions,
    ) -> Result<InviteLink> {
        Err(UnsupportedFeatureError::new("conversation invite links").into())
    }

    async fn revoke_invite_link(&self, invite: InviteLink) -> Result<()> {
        Err(UnsupportedFeatureError::new("conversation invite links").into())
    }

    async fn list_messages(&self, query: MessageQuery) -> Result<Page<MessageEnvelope>> {
        Err(UnsupportedFeatureError::new("message history").into())
    }

    async fn forward_messages(
        &self,
        messages: Vec<MessageRef>,
        target: MessageTarget,
        options: ForwardOptions,
    ) -> Result<Vec<MessageRef>> {
        Err(UnsupportedFeatureError::new("forwarding messages").into())
    }

    async fn copy_messages(
        &self,
        messages: Vec<MessageRef>,
        target: MessageTarget,
        options: ForwardOptions,
    ) -> Result<Vec<MessageRef>> {
        Err(UnsupportedFeatureError::new("copying messages").into())
    }

    async fn send_batch(&self, messages: Vec<BatchMessage>) -> Result<BatchSendResult> {
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

    async fn set_command_definitions(&self, commands: Vec<CommandDefinition>) -> Result<()> {
        Err(UnsupportedFeatureError::new("structured application commands").into())
    }

    async fn get_command_definitions(
        &self,
        conversation: Option<ConversationRef>,
    ) -> Result<Vec<CommandDefinition>> {
        Err(UnsupportedFeatureError::new("structured application commands").into())
    }

    async fn delete_command_definitions(
        &self,
        conversation: Option<ConversationRef>,
    ) -> Result<()> {
        Err(UnsupportedFeatureError::new("structured application commands").into())
    }

    async fn answer_suggestion_request(
        &self,
        request: SuggestionRequest,
        suggestions: Vec<Suggestion>,
        next_cursor: Option<String>,
    ) -> Result<()> {
        Err(UnsupportedFeatureError::new("dynamic suggestions or inline queries").into())
    }

    async fn publish_surface(&self, surface: AppSurface) -> Result<AppSurface> {
        Err(UnsupportedFeatureError::new("application surfaces").into())
    }

    async fn delete_surface(&self, surface: AppSurface) -> Result<()> {
        Err(UnsupportedFeatureError::new("application surfaces").into())
    }

    async fn answer_mini_app_query(
        &self,
        event: MiniAppEvent,
        message: OutgoingMessage,
    ) -> Result<Vec<MessageRef>> {
        Err(UnsupportedFeatureError::new("answering mini-app queries").into())
    }

    async fn set_bot_profile_v2(&self, profile: BotProfile) -> Result<()> {
        Err(UnsupportedFeatureError::new("localized bot profiles").into())
    }

    async fn get_bot_profile_v2(&self) -> Result<BotProfile> {
        Err(UnsupportedFeatureError::new("localized bot profiles").into())
    }

    async fn send_invoice(
        &self,
        target: MessageTarget,
        invoice: Invoice,
        options: InvoiceOptions,
    ) -> Result<MessageRef> {
        Err(UnsupportedFeatureError::new("invoices").into())
    }

    async fn answer_shipping_request(
        &self,
        request_id: String,
        options: Vec<ShippingOption>,
        error: Option<String>,
    ) -> Result<()> {
        Err(UnsupportedFeatureError::new("shipping requests").into())
    }

    async fn answer_checkout_request(
        &self,
        request: CheckoutRequest,
        approved: bool,
        error: Option<String>,
    ) -> Result<()> {
        Err(UnsupportedFeatureError::new("checkout requests").into())
    }

    async fn refund_payment(&self, payment: Payment) -> Result<()> {
        Err(UnsupportedFeatureError::new("payment refunds").into())
    }

    async fn send_message(
        &self,
        message: Vec<MessageSegment>,
        target: SendMessageTarget,
    ) -> Result<Vec<SendMessageResponse>> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn delete_message(&self, message_id: String) -> Result<()> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    /// Edits an existing message.
    ///
    /// The default delegates to the legacy misspelled `edit_messagee` method
    /// so existing 0.1 adapters remain source-compatible.
    async fn edit_message(
        &self,
        message_id: String,
        new_message: Vec<MessageSegment>,
    ) -> Result<()> {
        self.edit_messagee(message_id, new_message).await
    }

    /// Legacy spelling retained for 0.1 adapter compatibility.
    async fn edit_messagee(
        &self,
        message_id: String,
        new_message: Vec<MessageSegment>,
    ) -> Result<()> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn get_message_detail(&self, message_id: String) -> Result<GetMessageDetailResponse> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn set_message_reaction(&self, message_id: String, reaction_id: String) -> Result<()> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn get_group_member_list(&self, group_id: String) -> Result<GroupMemberListResponse> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn kick_group_member(
        &self,
        group_id: String,
        user_id: String,
        reject_add_request: Option<bool>,
    ) -> Result<()> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn mute_group(
        &self,
        group_id: String,
        duration: Option<Duration>,
        r#type: GroupMuteType,
    ) -> Result<()> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn mute_group_member(
        &self,
        group_id: String,
        user_id: String,
        r#type: GroupMuteType,
        duration: Option<Duration>,
    ) -> Result<()> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn change_group_admin(
        &self,
        group_id: String,
        user_id: String,
        r#type: GroupAdminChangeType,
    ) -> Result<()> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn set_group_member_alias(
        &self,
        group_id: String,
        user_id: String,
        new_alias: String,
    ) -> Result<()> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn get_group_profile(&self, group_id: String) -> Result<GroupGetProfileResponse> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn set_group_profile(&self, group_id: String, new_profile: GroupProfile) -> Result<()> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn get_group_file_count(
        &self,
        group_id: String,
        parent_folder_id: Option<String>,
    ) -> Result<GroupGetFileCountResponse> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn get_group_fs_list(
        &self,
        group_id: String,
        start_index: u64,
        count: u64,
    ) -> Result<GroupGetFsListResponse> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn delete_group_file(&self, group_id: String, file_id: String) -> Result<()> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn delete_group_folder(&self, group_id: String, folder_id: String) -> Result<()> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn create_group_folder(
        &self,
        group_id: String,
        folder_name: String,
        parent_folder_id: Option<String>,
    ) -> Result<()> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn get_user_profile(&self, user_id: String) -> Result<UserGetProfileResponse> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn set_bot_profile(&self, new_profile: UserProfile) -> Result<()> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn get_bot_profile(&self) -> Result<BotGetProfileResponse> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn get_bot_friend_list(&self) -> Result<BotGetFriendListResponse> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn get_bot_group_list(&self) -> Result<BotGetGroupListResponse> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn handle_add_friend_request(&self, id: String, response: RequestResponse) -> Result<()> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn handle_add_group_request(&self, id: String, response: RequestResponse) -> Result<()> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn handle_invite_group_request(
        &self,
        id: String,
        response: RequestResponse,
    ) -> Result<()> {
        Err(anyhow::anyhow!("Not implemented"))
    }

    async fn get_file_info(&self, file_id: String) -> Result<File> {
        Err(anyhow::anyhow!("Not implemented"))
    }
}

fn legacy_send_target(target: &MessageTarget) -> Result<SendMessageTarget> {
    anyhow::ensure!(
        target.conversation.parent.is_none()
            && target.conversation.platform_data.is_none()
            && target.platform_data.is_none()
            && target.recipients.is_empty(),
        "nested or platform-specific message targets require the v2 adapter API"
    );
    Ok(match target.conversation.kind {
        crate::conversation::ConversationKind::Direct => {
            SendMessageTarget::Private(target.conversation.id.clone())
        }
        crate::conversation::ConversationKind::Group
        | crate::conversation::ConversationKind::Channel => {
            SendMessageTarget::Group(target.conversation.id.clone())
        }
        _ => {
            return Err(UnsupportedFeatureError::new(
                "this conversation kind in the legacy message API",
            )
            .into())
        }
    })
}

fn legacy_reaction_id(reaction: &Reaction) -> Result<String> {
    Ok(match reaction {
        Reaction::UnicodeEmoji(emoji) => emoji.clone(),
        Reaction::CustomEmoji { id, .. } => id.clone(),
        Reaction::Paid => "paid".to_owned(),
        Reaction::PlatformNative { .. } => {
            return Err(UnsupportedFeatureError::new(
                "platform-native reactions in the legacy reaction API",
            )
            .into())
        }
    })
}
