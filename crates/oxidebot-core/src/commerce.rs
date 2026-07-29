//! Portable payment, invoice, checkout, and subscription data.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{conversation::ConversationRef, interaction::PlatformNativeData, source::user::User};

/// Monetary amount expressed in a named currency's smallest unit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Money {
    /// ISO 4217 code or a platform currency such as Telegram Stars (`XTR`).
    pub currency: String,
    /// Amount in the currency's smallest unit.
    pub amount: i64,
}

impl Money {
    /// Creates a monetary amount without applying exchange-rate conversion.
    pub fn new(currency: impl Into<String>, amount: i64) -> Self {
        Self {
            currency: currency.into(),
            amount,
        }
    }
}

/// One priced item in an invoice or shipping option.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineItem {
    /// User-visible line-item label.
    pub label: String,
    /// Price of one unit.
    pub amount: Money,
    /// Number of units, if the platform represents it separately.
    pub quantity: Option<u32>,
}

/// Optional customer details collected by a payment platform.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CustomerDetails {
    /// Customer name.
    pub name: Option<String>,
    /// Customer email address.
    pub email: Option<String>,
    /// Customer phone number.
    pub phone: Option<String>,
    /// Lossless shipping-address payload.
    pub shipping_address: Option<serde_json::Value>,
}

/// Invoice content that a bot can send to a customer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Invoice {
    /// Platform-assigned invoice identifier, if known.
    pub id: Option<String>,
    /// User-visible invoice title.
    pub title: String,
    /// User-visible invoice description.
    pub description: String,
    /// Opaque payload echoed by platform payment events.
    pub payload: String,
    /// Priced invoice items.
    pub items: Vec<LineItem>,
    /// Customer details requested or associated with the invoice.
    pub customer_details: CustomerDetails,
    /// Whether shipping options depend on the customer address.
    pub flexible_shipping: bool,
    /// Lossless platform-specific invoice metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// Options controlling platform delivery of an invoice.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InvoiceOptions {
    /// Platform payment-provider credential or token.
    pub provider_token: Option<String>,
    /// Parameter used when a user opens the invoice via a deep link.
    pub start_parameter: Option<String>,
    /// Optional product-image URL.
    pub photo_url: Option<String>,
    /// Whether the platform should protect invoice content from forwarding.
    pub protect_content: bool,
    /// Lossless platform-specific invoice-delivery metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// Lifecycle state of a payment or subscription.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaymentStatus {
    /// Payment is awaiting completion.
    Pending,
    /// Payment is authorized but not captured.
    Authorized,
    /// Payment completed successfully.
    Paid,
    /// Payment failed.
    Failed,
    /// Payment was refunded.
    Refunded,
    /// Payment was canceled.
    Canceled,
    /// State without a portable OxideBot equivalent.
    PlatformNative(String),
}

/// A payment recorded by a platform.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Payment {
    /// Platform payment identifier.
    pub id: String,
    /// Associated invoice identifier, if exposed.
    pub invoice_id: Option<String>,
    /// User that paid, if exposed.
    pub payer: Option<User>,
    /// Conversation associated with the payment, if any.
    pub conversation: Option<ConversationRef>,
    /// Total amount paid.
    pub total: Money,
    /// Current payment lifecycle state.
    pub status: PaymentStatus,
    /// Platform-reported payment time.
    pub occurred_at: Option<DateTime<Utc>>,
    /// Lossless platform-specific payment metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// A platform request to approve or reject checkout.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckoutRequest {
    /// Platform checkout-request identifier.
    pub id: String,
    /// Customer requesting checkout.
    pub user: User,
    /// Invoice payload echoed by the platform.
    pub invoice_payload: String,
    /// Total amount to collect.
    pub total: Money,
    /// Customer details supplied with the request.
    pub customer_details: CustomerDetails,
    /// Lossless platform-specific checkout metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// One selectable shipping option for an invoice.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShippingOption {
    /// Stable shipping-option identifier.
    pub id: String,
    /// User-visible shipping-option title.
    pub title: String,
    /// Price components of the option.
    pub prices: Vec<LineItem>,
}

/// A platform request to calculate available shipping options.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShippingRequest {
    /// Platform shipping-request identifier.
    pub id: String,
    /// User requesting shipping.
    pub user: User,
    /// Invoice payload echoed by the platform.
    pub invoice_payload: String,
    /// Lossless shipping address supplied by the platform.
    pub address: serde_json::Value,
    /// Lossless platform-specific shipping metadata.
    pub platform_data: Option<PlatformNativeData>,
}

/// A recurring payment subscription.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Subscription {
    /// Platform subscription identifier.
    pub id: String,
    /// Subscribed user.
    pub user: User,
    /// Recurring price.
    pub price: Money,
    /// Current subscription lifecycle state.
    pub status: PaymentStatus,
    /// Start of the current billing period.
    pub period_start: Option<DateTime<Utc>>,
    /// End of the current billing period.
    pub period_end: Option<DateTime<Utc>>,
    /// Whether the subscription renews automatically.
    pub auto_renewing: Option<bool>,
    /// Lossless platform-specific subscription metadata.
    pub platform_data: Option<PlatformNativeData>,
}
