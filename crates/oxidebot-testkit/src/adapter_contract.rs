//! Reusable assertions for portable adapter delivery reports.

use oxidebot_core::{CallError, DeliveryPlan, DeliveryReport};

/// Verifies a fully successful delivery report.
pub fn assert_complete(plan: &DeliveryPlan, report: &DeliveryReport) -> Result<(), String> {
    if report.degradations != plan.degradations {
        return Err("delivery report did not preserve planner degradations".into());
    }
    if report.items.len() != plan.messages.len() {
        return Err(format!(
            "delivery report contains {} items for {} planned messages",
            report.items.len(),
            plan.messages.len()
        ));
    }
    for (index, item) in report.items.iter().enumerate() {
        if item.index != index {
            return Err(format!(
                "delivery report item at position {index} claims index {}",
                item.index
            ));
        }
        if item.error.is_some() {
            return Err(format!("delivery report item {index} unexpectedly failed"));
        }
    }
    let item_messages = report
        .items
        .iter()
        .flat_map(|item| item.messages.iter())
        .collect::<Vec<_>>();
    if report.messages.len() != item_messages.len()
        || !report
            .messages
            .iter()
            .zip(item_messages)
            .all(|(a, b)| a == b)
    {
        return Err("delivery report summary differs from successful item references".into());
    }
    Ok(())
}

/// Verifies the structured report carried by a partial-delivery error.
pub fn assert_partial(plan: &DeliveryPlan, error: &CallError) -> Result<(), String> {
    let CallError::PartialDelivery(partial) = error else {
        return Err("expected CallError::PartialDelivery".into());
    };
    let report = &partial.report;
    if report.degradations != plan.degradations {
        return Err("partial report did not preserve planner degradations".into());
    }
    let Some(failed) = report.items.last() else {
        return Err("partial report has no failed item".into());
    };
    if failed.error.is_none() || !failed.messages.is_empty() {
        return Err("partial report terminal item must fail without message references".into());
    }
    if report.items.len() > plan.messages.len() {
        return Err("partial report has more items than the delivery plan".into());
    }
    for (index, item) in report.items.iter().enumerate() {
        if item.index != index {
            return Err(format!(
                "partial report item at position {index} claims index {}",
                item.index
            ));
        }
    }
    Ok(())
}
