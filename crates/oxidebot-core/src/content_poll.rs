//! Portable poll and checklist models.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::RichText;
use crate::{conversation::MessageRef, interaction::PlatformNativeData, source::user::User};

/// Poll behavior supported by the platform.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum PollType {
    /// Ordinary poll.
    #[default]
    Regular,
    /// Quiz poll.
    Quiz,
    /// Platform-native poll type.
    PlatformNative(String),
}

/// One selectable answer in a poll.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PollOption {
    /// Platform option identifier, if available.
    pub id: Option<String>,
    /// User-visible option text.
    pub text: RichText,
    /// Current vote count, if exposed.
    pub voter_count: Option<u64>,
    /// Lossless platform-specific option metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// Portable poll state and configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Poll {
    /// Platform poll identifier, if available.
    pub id: Option<String>,
    /// Poll question.
    pub question: RichText,
    /// Available answers.
    pub options: Vec<PollOption>,
    /// Whether multiple answers may be selected.
    pub allows_multiple_answers: bool,
    /// Whether users may change their vote.
    pub allows_revoting: Option<bool>,
    /// Whether only conversation members may vote.
    pub members_only: Option<bool>,
    /// Country availability restrictions.
    pub country_codes: Vec<String>,
    /// Whether votes are anonymous.
    pub anonymous: Option<bool>,
    /// Poll behavior.
    pub kind: PollType,
    /// Duration for which the poll remains open.
    pub open_for: Option<Duration>,
    /// Absolute closing time.
    pub closes_at: Option<DateTime<Utc>>,
    /// Total vote count, if exposed.
    pub total_voter_count: Option<u64>,
    /// Retained as a convenience for platforms limited to one correct answer.
    pub correct_option: Option<usize>,
    /// Correct answer indexes for platforms that support multiple correct answers.
    pub correct_options: Vec<usize>,
    /// Optional answer explanation.
    pub explanation: Option<RichText>,
    /// Whether voting is closed.
    pub closed: bool,
    /// Lossless platform-specific poll metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// One task in a portable checklist.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChecklistTask {
    /// Platform task identifier, if available.
    pub id: Option<String>,
    /// User-visible task text.
    pub text: RichText,
    /// Whether the task is complete.
    pub completed: bool,
    /// User that completed the task, if known.
    pub completed_by: Option<User>,
    /// Completion time, if known.
    pub completed_at: Option<DateTime<Utc>>,
    /// Lossless platform-specific task metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// Portable checklist content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Checklist {
    /// Platform checklist identifier, if available.
    pub id: Option<String>,
    /// Checklist title.
    pub title: RichText,
    /// Ordered tasks.
    pub tasks: Vec<ChecklistTask>,
    /// Whether users may add tasks.
    pub can_add_tasks: bool,
    /// Whether users may mark tasks complete.
    pub can_mark_tasks_done: bool,
    /// Lossless platform-specific checklist metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// Incremental change to a checklist.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ChecklistChange {
    /// Message containing the checklist, if known.
    pub message: Option<MessageRef>,
    /// Tasks added by the change.
    pub added_tasks: Vec<ChecklistTask>,
    /// Task identifiers marked complete.
    pub completed_task_ids: Vec<String>,
    /// Task identifiers reopened by the change.
    pub reopened_task_ids: Vec<String>,
    /// User that made the change, if known.
    pub actor: Option<User>,
    /// Lossless platform-specific change metadata.
    pub platform_data: Option<PlatformNativeData>,
}
