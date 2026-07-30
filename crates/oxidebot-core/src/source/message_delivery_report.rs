//! Public contract types for planned and executed message delivery.

use serde::{Deserialize, Serialize};
use std::{error::Error, fmt};

use super::{Message, MessageRef};

/// Policy used when a platform cannot represent portable message semantics.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum FallbackPolicy {
    /// Fail on the first unsupported semantic feature.
    Strict,
    /// Drop unsupported features and record every loss.
    DropUnsupported,
    /// Convert unsupported features to their human-readable text representation.
    ToText,
    /// Prefer child/fallback content, then text, then drop.
    Flatten,
    /// Preserve semantics where possible and refuse silent semantic loss.
    #[default]
    Auto,
}

/// Type of semantic degradation recorded during delivery planning.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DegradationKind {
    /// The adapter can preserve the logical feature through a portable
    /// representation rather than a native platform primitive.
    Emulated,
    /// Content was rendered as a text fallback.
    ConvertedToText,
    /// Content was replaced with a supported child or fallback representation.
    Flattened,
    /// Content could not be represented and was omitted.
    Dropped,
    /// A message option could not be represented and was removed.
    OptionRemoved,
    /// One logical message was split to satisfy platform limits.
    MessageSplit,
}

/// One semantic change made while producing a physical delivery plan.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeliveryDegradation {
    /// Location in the logical portable message.
    pub path: String,
    /// Feature that triggered the degradation.
    pub feature: String,
    /// Class of semantic change.
    pub kind: DegradationKind,
    /// Human-readable explanation of the change.
    pub detail: String,
}

/// Exact physical messages and semantic degradations selected before delivery.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeliveryPlan {
    /// Ordered physical messages to send.
    pub messages: Vec<Message>,
    /// Semantic changes required by platform support or limits.
    pub degradations: Vec<DeliveryDegradation>,
}

impl DeliveryPlan {
    /// Returns whether planning required any semantic degradation.
    #[must_use]
    pub fn degraded(&self) -> bool {
        !self.degradations.is_empty()
    }
}

/// Result of a complete or partially completed physical delivery plan.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeliveryReport {
    /// Successfully created message references.
    pub messages: Vec<MessageRef>,
    /// Semantic changes selected by planning.
    pub degradations: Vec<DeliveryDegradation>,
    /// Per-physical-message outcomes in delivery-plan order. This remains
    /// populated on a partial-delivery error so callers can retry or
    /// compensate without duplicating already successful messages.
    #[serde(default)]
    pub items: Vec<DeliveryItemResult>,
}

impl DeliveryReport {
    /// Returns whether planning required any semantic degradation.
    #[must_use]
    pub fn degraded(&self) -> bool {
        !self.degradations.is_empty()
    }

    /// Returns true when every attempted physical message succeeded.
    #[must_use]
    pub fn completed(&self) -> bool {
        self.items.iter().all(DeliveryItemResult::succeeded)
    }

    /// Iterates the failed physical messages without losing successful refs.
    pub fn failures(&self) -> impl Iterator<Item = &DeliveryItemResult> {
        self.items.iter().filter(|item| !item.succeeded())
    }
}

/// Outcome of one physical message in a logical delivery plan.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeliveryItemResult {
    /// Zero-based physical-message index in the delivery plan.
    pub index: usize,
    /// References created by this physical delivery item.
    pub messages: Vec<MessageRef>,
    /// Failure detail when this physical item did not succeed.
    pub error: Option<String>,
}

impl DeliveryItemResult {
    /// Returns whether this physical delivery item succeeded.
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.error.is_none()
    }
}

/// Error returned after a logical delivery has already produced a structured
/// per-item report. The report may contain successful physical messages.
#[derive(Clone, Debug, PartialEq)]
pub struct PartialDeliveryError {
    /// Complete report preserving every successful and failed physical item.
    pub report: DeliveryReport,
}

impl fmt::Display for PartialDeliveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let completed = self
            .report
            .items
            .iter()
            .filter(|item| item.succeeded())
            .count();
        write!(
            formatter,
            "logical delivery stopped after {completed} of {} physical messages succeeded",
            self.report.items.len()
        )
    }
}

impl Error for PartialDeliveryError {}

/// Error returned when strict delivery planning cannot preserve a feature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveryPlanningError {
    /// Location in the logical portable message.
    pub path: String,
    /// Feature that cannot be represented under the selected fallback policy.
    pub feature: String,
}

impl fmt::Display for DeliveryPlanningError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "message feature {:?} at {} is unsupported and the fallback policy forbids degradation",
            self.feature, self.path
        )
    }
}

impl Error for DeliveryPlanningError {}
