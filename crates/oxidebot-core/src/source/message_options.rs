//! Message-wide delivery and interaction options.

use super::*;

/// Message-wide options are part of the same message IR rather than a parallel
/// outgoing-message type.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MessageOptions {
    /// Optional interactive component container.
    pub components: Option<MessageComponents>,
    /// Optional message being replied to.
    pub reply: Option<ReplyOptions>,
    /// Notification behavior for delivery.
    pub notification: NotificationPolicy,
    /// Intended message visibility.
    pub visibility: MessageVisibility,
    /// Optional link-preview preferences.
    pub link_preview: Option<LinkPreviewOptions>,
    /// Optional mention-delivery preferences.
    pub mentions: Option<MentionPolicy>,
    /// Whether platforms should protect content from forwarding or saving.
    pub protect_content: bool,
    /// Immediate or scheduled delivery time.
    pub delivery_time: DeliveryTime,
    /// Optional caller-supplied idempotency key.
    pub idempotency_key: Option<String>,
    /// Optional caller-supplied client message identifier.
    pub client_message_id: Option<String>,
    /// Adapter-independent metadata associated with the message.
    pub metadata: BTreeMap<String, Value>,
    /// Lossless platform-specific delivery metadata.
    pub platform_data: Option<PlatformNativeData>,
}

impl MessageOptions {
    /// Sets the interactive component container.
    #[must_use]
    pub fn components(mut self, components: MessageComponents) -> Self {
        self.components = Some(components);
        self
    }

    /// Sets the reply options.
    #[must_use]
    pub fn reply(mut self, reply: ReplyOptions) -> Self {
        self.reply = Some(reply);
        self
    }

    /// Sets the notification policy.
    #[must_use]
    pub fn notification(mut self, notification: NotificationPolicy) -> Self {
        self.notification = notification;
        self
    }

    /// Sets the intended message visibility.
    #[must_use]
    pub fn visibility(mut self, visibility: MessageVisibility) -> Self {
        self.visibility = visibility;
        self
    }

    /// Sets link-preview preferences.
    #[must_use]
    pub fn link_preview(mut self, link_preview: LinkPreviewOptions) -> Self {
        self.link_preview = Some(link_preview);
        self
    }

    /// Sets mention-delivery preferences.
    #[must_use]
    pub fn mentions(mut self, mentions: MentionPolicy) -> Self {
        self.mentions = Some(mentions);
        self
    }

    /// Enables or disables protected content.
    #[must_use]
    pub fn protect_content(mut self, protect_content: bool) -> Self {
        self.protect_content = protect_content;
        self
    }

    /// Sets immediate or scheduled delivery time.
    #[must_use]
    pub fn delivery_time(mut self, delivery_time: DeliveryTime) -> Self {
        self.delivery_time = delivery_time;
        self
    }

    /// Returns whether all options are their default empty values.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.components.is_none()
            && self.reply.is_none()
            && self.notification == NotificationPolicy::Default
            && self.visibility == MessageVisibility::Public
            && self.link_preview.is_none()
            && self.mentions.is_none()
            && !self.protect_content
            && self.delivery_time == DeliveryTime::Immediate
            && self.idempotency_key.is_none()
            && self.client_message_id.is_none()
            && self.metadata.is_empty()
            && self.platform_data.is_none()
    }

    /// Estimates bytes retained by options and owned metadata.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        self.idempotency_key
            .as_ref()
            .map_or(0, String::len)
            .saturating_add(self.client_message_id.as_ref().map_or(0, String::len))
            .saturating_add(
                self.metadata
                    .iter()
                    .map(|(key, value)| key.len().saturating_add(value.to_string().len()))
                    .sum::<usize>(),
            )
            .saturating_add(256)
    }
}
