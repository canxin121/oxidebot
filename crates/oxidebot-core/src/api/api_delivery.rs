//! Helpers for adapters to report execution of a delivery plan correctly.

use crate::{
    conversation::MessageRef,
    source::message::{
        DeliveryDegradation, DeliveryItemResult, DeliveryPlan, DeliveryReport, PartialDeliveryError,
    },
};

use super::{CallError, CallResult};

/// Incrementally builds a complete [`DeliveryReport`] while executing one
/// previously planned logical delivery.
///
/// An adapter must report every physical message in plan order, including the
/// successful prefix when a later send fails. This helper centralizes that
/// otherwise easy-to-get-wrong invariant and produces a typed
/// [`CallError::PartialDelivery`] when appropriate.
#[derive(Debug)]
pub struct DeliveryReportBuilder {
    degradations: Vec<DeliveryDegradation>,
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
        CallError::PartialDelivery(PartialDeliveryError {
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
