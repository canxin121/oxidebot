use std::time::Duration;

use anyhow::Result;

pub mod payload;
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

#[async_trait::async_trait]
pub trait CallApiTrait {
    async fn send_message(
        &self,
        message: Vec<MessageSegment>,
        target: SendMessageTarget,
    ) -> Result<SendMessageResponse>;

    async fn delete_message(&self, message_id: String) -> Result<()>;

    async fn get_message_detail(&self, message_id: String) -> Result<GetMessageDetailResponse>;

    async fn set_message_reaction(&self, message_id: String, reaction_id: String) -> Result<()>;

    async fn get_group_member_list(&self, group_id: String) -> Result<GroupMemberListResponse>;

    async fn kick_group_member(
        &self,
        group_id: String,
        user_id: String,
        reject_add_request: Option<bool>,
    ) -> Result<()>;

    async fn mute_group(
        &self,
        group_id: String,
        duration: Option<Duration>,
        r#type: GroupMuteType,
    ) -> Result<()>;

    async fn mute_group_member(
        &self,
        group_id: String,
        user_id: String,
        r#type: GroupMuteType,
        duration: Option<Duration>,
    ) -> Result<()>;

    async fn change_group_admin(
        &self,
        group_id: String,
        user_id: String,
        r#type: GroupAdminChangeType,
    ) -> Result<()>;

    async fn set_group_member_alias(
        &self,
        group_id: String,
        user_id: String,
        new_alias: String,
    ) -> Result<()>;

    async fn get_group_profile(&self, group_id: String) -> Result<GroupGetProfileResponse>;

    async fn set_group_profile(&self, group_id: String, new_profile: GroupProfile) -> Result<()>;

    async fn get_group_file_count(
        &self,
        group_id: String,
        parent_folder_id: Option<String>,
    ) -> Result<GroupGetFileCountResponse>;

    async fn get_group_fs_list(
        &self,
        group_id: String,
        start_index: u64,
        count: u64,
    ) -> Result<GroupGetFsListResponse>;

    async fn delete_group_file(&self, group_id: String, file_id: String) -> Result<()>;

    async fn delete_group_folder(&self, group_id: String, folder_id: String) -> Result<()>;

    async fn create_group_folder(
        &self,
        group_id: String,
        folder_name: String,
        parent_folder_id: Option<String>,
    ) -> Result<()>;

    async fn get_user_profile(&self, user_id: String) -> Result<UserGetProfileResponse>;

    async fn set_bot_profile(&self, new_profile: UserProfile) -> Result<()>;

    async fn get_bot_profile(&self) -> Result<BotGetProfileResponse>;

    async fn get_bot_friend_list(&self) -> Result<BotGetFriendListResponse>;

    async fn get_bot_group_list(&self) -> Result<BotGetGroupListResponse>;

    async fn handle_add_friend_request(&self, id: String, response: RequestResponse) -> Result<()>;

    async fn handle_add_group_request(&self, id: String, response: RequestResponse) -> Result<()>;

    async fn handle_invite_group_request(
        &self,
        id: String,
        response: RequestResponse,
    ) -> Result<()>;

    async fn get_file_info(&self, file_id: String) -> Result<File>;
}
