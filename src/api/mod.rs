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

use crate::source::{
    group::GroupProfile,
    message::{File, MessageSegment},
    user::UserProfile,
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
