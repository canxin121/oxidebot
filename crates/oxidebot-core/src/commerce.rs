//! Portable payment, invoice, checkout, and subscription data.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{conversation::ConversationRef, interaction::PlatformNativeData, source::user::User};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Money {
    /// ISO 4217 code or a platform currency such as Telegram Stars (`XTR`).
    pub currency: String,
    /// Amount in the currency's smallest unit.
    pub amount: i64,
}

impl Money {
    pub fn new(currency: impl Into<String>, amount: i64) -> Self {
        Self {
            currency: currency.into(),
            amount,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineItem {
    pub label: String,
    pub amount: Money,
    pub quantity: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CustomerDetails {
    pub name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub shipping_address: Option<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Invoice {
    pub id: Option<String>,
    pub title: String,
    pub description: String,
    pub payload: String,
    pub items: Vec<LineItem>,
    pub customer_details: CustomerDetails,
    pub flexible_shipping: bool,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InvoiceOptions {
    pub provider_token: Option<String>,
    pub start_parameter: Option<String>,
    pub photo_url: Option<String>,
    pub protect_content: bool,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaymentStatus {
    Pending,
    Authorized,
    Paid,
    Failed,
    Refunded,
    Canceled,
    PlatformNative(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Payment {
    pub id: String,
    pub invoice_id: Option<String>,
    pub payer: Option<User>,
    pub conversation: Option<ConversationRef>,
    pub total: Money,
    pub status: PaymentStatus,
    pub occurred_at: Option<DateTime<Utc>>,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckoutRequest {
    pub id: String,
    pub user: User,
    pub invoice_payload: String,
    pub total: Money,
    pub customer_details: CustomerDetails,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShippingOption {
    pub id: String,
    pub title: String,
    pub prices: Vec<LineItem>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShippingRequest {
    pub id: String,
    pub user: User,
    pub invoice_payload: String,
    pub address: serde_json::Value,
    pub platform_data: Option<PlatformNativeData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Subscription {
    pub id: String,
    pub user: User,
    pub price: Money,
    pub status: PaymentStatus,
    pub period_start: Option<DateTime<Utc>>,
    pub period_end: Option<DateTime<Utc>>,
    pub auto_renewing: Option<bool>,
    pub platform_data: Option<PlatformNativeData>,
}
