//! Canonical message container and capability-aware delivery-plan entry point.

use super::*;

/// The single cross-platform message intermediate representation used for
/// inbound events, handler results, active sends, command parsing, delivery
/// planning, and adapter export.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Message {
    /// Platform message ID for an inbound or already-sent message. It is absent
    /// for a newly constructed outgoing message.
    #[serde(default)]
    pub id: Option<MessageId>,
    /// Ordered, lossless message segments.
    #[serde(default)]
    pub segments: Vec<MessageSegment>,
    /// Message-wide delivery and interaction options.
    #[serde(default)]
    pub options: MessageOptions,
}

impl Message {
    /// Creates a message from ordered portable segments.
    #[must_use]
    pub fn new(segments: impl IntoIterator<Item = MessageSegment>) -> Self {
        Self::from_segments(segments)
    }

    /// Creates a one-segment plain-text message.
    #[must_use]
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            id: None,
            segments: vec![MessageSegment::text(content)],
            options: MessageOptions::default(),
        }
    }

    /// Creates a one-segment rich-text message.
    #[must_use]
    pub fn rich_text(content: RichText) -> Self {
        Self::from(MessageSegment::RichText(content))
    }

    /// Creates a message from segments while merging adjacent text segments.
    #[must_use]
    pub fn from_segments(segments: impl IntoIterator<Item = MessageSegment>) -> Self {
        let mut message = Self::default();
        message.extend(segments);
        message
    }

    /// Replaces all message-wide delivery options.
    #[must_use]
    pub fn options(mut self, options: MessageOptions) -> Self {
        self.options = options;
        self
    }

    /// Replaces the message interactive component container.
    #[must_use]
    pub fn components(mut self, components: MessageComponents) -> Self {
        self.options.components = Some(components);
        self
    }

    /// Appends one row of interactive components. Existing inline-keyboard
    /// rows are preserved; another component surface is intentionally replaced
    /// because platforms expose at most one message component container.
    #[must_use]
    pub fn component_row(mut self, row: ActionRow) -> Self {
        match self.options.components.as_mut() {
            Some(MessageComponents::InlineKeyboard(keyboard)) => keyboard.rows.push(row),
            _ => {
                self.options.components =
                    Some(MessageComponents::InlineKeyboard(InlineKeyboard::new([
                        row,
                    ])));
            }
        }
        self
    }

    /// Appends a row of buttons without manually constructing keyboard wrappers.
    #[must_use]
    pub fn buttons(self, buttons: impl IntoIterator<Item = Button>) -> Self {
        self.component_row(ActionRow::buttons(buttons))
    }

    /// Appends one button as a new inline keyboard row.
    #[must_use]
    pub fn button(self, button: Button) -> Self {
        self.buttons([button])
    }

    /// Appends a URL button as a new inline keyboard row.
    #[must_use]
    pub fn button_url(self, label: impl Into<String>, url: impl Into<String>) -> Self {
        self.button(Button::url(label, url))
    }

    /// Appends a callback-action button as a new inline keyboard row.
    #[must_use]
    pub fn button_action(self, label: impl Into<String>, data: impl Into<String>) -> Self {
        self.button(Button::callback(label, data))
    }

    /// Appends a send-text button as a new inline keyboard row.
    #[must_use]
    pub fn button_text(self, label: impl Into<String>, text: impl Into<String>) -> Self {
        self.button(Button::send_text(label, text))
    }

    /// Appends one segment and returns the changed message.
    #[must_use]
    pub fn then(mut self, segment: impl IntoMessageSegment) -> Self {
        self.push(segment);
        self
    }

    /// Appends one segment, merging it with a preceding text segment when possible.
    pub fn push(&mut self, segment: impl IntoMessageSegment) {
        let segment = segment.into_message_segment();
        match segment {
            MessageSegment::Text { content } => {
                if let Some(MessageSegment::Text { content: previous }) = self.segments.last_mut() {
                    previous.push_str(&content);
                } else {
                    self.segments.push(MessageSegment::Text { content });
                }
            }
            segment => self.segments.push(segment),
        }
    }

    /// Appends all segments while preserving text-segment normalization.
    pub fn extend(&mut self, segments: impl IntoIterator<Item = MessageSegment>) {
        for segment in segments {
            self.push(segment);
        }
    }

    /// Appends a user mention.
    #[must_use]
    pub fn at(self, user_id: impl Into<UserId>) -> Self {
        self.then(MessageSegment::at(user_id))
    }

    /// Appends a role mention.
    #[must_use]
    pub fn at_role(self, role_id: impl Into<RoleId>) -> Self {
        self.then(MessageSegment::AtRole {
            role_id: role_id.into(),
        })
    }

    /// Appends a channel mention.
    #[must_use]
    pub fn at_channel(self, channel_id: impl Into<ConversationId>) -> Self {
        self.then(MessageSegment::AtChannel {
            channel_id: channel_id.into(),
        })
    }

    /// Appends an everyone mention.
    #[must_use]
    pub fn at_all(self) -> Self {
        self.then(MessageSegment::at_all())
    }

    /// Sets a reply target by platform message identifier.
    #[must_use]
    pub fn reply_to(mut self, message_id: impl Into<MessageId>) -> Self {
        self.options.reply = Some(ReplyOptions::new(MessageRef::new(message_id)));
        self
    }

    /// Appends a reference to another message.
    #[must_use]
    pub fn reference(self, message_id: impl Into<MessageId>) -> Self {
        self.then(MessageSegment::reference(message_id))
    }

    /// Appends image media.
    #[must_use]
    pub fn image(self, file: File) -> Self {
        self.then(MessageSegment::image(file))
    }

    /// Appends video media with an optional duration.
    #[must_use]
    pub fn video(self, file: File, duration: Option<std::time::Duration>) -> Self {
        self.then(MessageSegment::video(file, duration))
    }

    /// Appends audio media with an optional duration.
    #[must_use]
    pub fn audio(self, file: File, duration: Option<std::time::Duration>) -> Self {
        self.then(MessageSegment::audio(file, duration))
    }

    /// Appends a document attachment.
    #[must_use]
    pub fn file(self, file: File) -> Self {
        self.then(MessageSegment::file(file))
    }

    /// Appends media of an explicit portable kind.
    #[must_use]
    pub fn media(self, kind: MediaType, media: Media) -> Self {
        self.then(MessageSegment::Media {
            kind,
            media: Box::new(media),
        })
    }

    /// Appends a poll.
    #[must_use]
    pub fn poll(self, poll: Poll) -> Self {
        self.then(MessageSegment::Poll(poll))
    }

    /// Appends a rich layout.
    #[must_use]
    pub fn layout(self, layout: RichLayout) -> Self {
        self.then(MessageSegment::Layout(layout))
    }

    /// Appends custom emoji by its platform identifier.
    #[must_use]
    pub fn emoji(self, id: impl Into<String>) -> Self {
        self.then(MessageSegment::emoji(id))
    }

    /// Iterates image and animation files in segment order.
    pub fn image_files(&self) -> impl DoubleEndedIterator<Item = &File> {
        self.segments.iter().filter_map(|segment| match segment {
            MessageSegment::Media {
                kind: MediaType::Image | MediaType::Animation,
                media,
            } => Some(&media.file),
            _ => None,
        })
    }

    /// Iterates document and platform-native attached files in segment order.
    pub fn attached_files(&self) -> impl DoubleEndedIterator<Item = &File> {
        self.segments.iter().filter_map(|segment| match segment {
            MessageSegment::Media {
                kind: MediaType::Document | MediaType::PlatformNative(_),
                media,
            } => Some(&media.file),
            _ => None,
        })
    }

    /// Iterates explicitly mentioned user identifiers in segment order.
    pub fn mentioned_users(&self) -> impl DoubleEndedIterator<Item = &UserId> {
        self.segments.iter().filter_map(|segment| match segment {
            MessageSegment::At { user_id } => Some(user_id),
            _ => None,
        })
    }

    /// Iterates configured reply target identifiers.
    pub fn reply_ids(&self) -> impl DoubleEndedIterator<Item = &MessageId> {
        self.options.reply.iter().map(|reply| &reply.message.id)
    }

    /// Consumes the message and returns its normalized segments.
    #[must_use]
    pub fn into_segments(self) -> Vec<MessageSegment> {
        self.segments
    }

    /// Returns whether the message has no content and no delivery options.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty() && self.options.is_empty()
    }

    /// Returns the number of normalized segments.
    #[must_use]
    pub fn len(&self) -> usize {
        self.segments.len()
    }

    /// Returns whether the first segment starts with `text`.
    #[must_use]
    pub fn starts_with_text(&self, text: &str) -> bool {
        self.segments.first().is_some_and(|segment| match segment {
            MessageSegment::Text { content } => content.starts_with(text),
            MessageSegment::RichText(content) => content.text.starts_with(text),
            _ => false,
        })
    }

    /// Returns a clone with `text` removed from its first matching text segment.
    #[must_use]
    pub fn trim_head_text(&self, text: &str) -> Vec<MessageSegment> {
        let mut segments = self.segments.clone();
        for segment in &mut segments {
            match segment {
                MessageSegment::Text { content } => {
                    if content.starts_with(text) {
                        *content = content.trim_start_matches(text).to_owned();
                        break;
                    }
                }
                MessageSegment::RichText(content) if content.text.starts_with(text) => {
                    content.text = content.text.trim_start_matches(text).to_owned();
                    content.spans.clear();
                    break;
                }
                _ => {}
            }
        }
        segments
    }

    /// Returns a plain-text rendering of all portable segments.
    #[must_use]
    pub fn get_raw_text(&self) -> String {
        self.extract_plain_text()
    }

    /// Returns a plain-text rendering suitable for command parsing.
    #[must_use]
    pub fn extract_plain_text(&self) -> String {
        let capacity = self
            .segments
            .iter()
            .map(MessageSegment::plain_text_len)
            .sum();
        let mut output = String::with_capacity(capacity);
        for segment in &self.segments {
            segment.write_plain_text(&mut output);
        }
        output
    }

    /// Returns whether at least one segment has `kind`.
    #[must_use]
    pub fn has(&self, kind: SegmentKind) -> bool {
        self.segments.iter().any(|segment| segment.kind() == kind)
    }

    /// Returns the first segment with `kind`.
    #[must_use]
    pub fn first(&self, kind: SegmentKind) -> Option<&MessageSegment> {
        self.segments.iter().find(|segment| segment.kind() == kind)
    }

    /// Iterates all segments with `kind`.
    pub fn select(&self, kind: SegmentKind) -> impl DoubleEndedIterator<Item = &MessageSegment> {
        self.segments
            .iter()
            .filter(move |segment| segment.kind() == kind)
    }

    /// Returns a clone that retains only the requested segment kinds.
    #[must_use]
    pub fn include(&self, kinds: &[SegmentKind]) -> Self {
        Self {
            id: self.id.clone(),
            segments: self
                .segments
                .iter()
                .filter(|segment| kinds.contains(&segment.kind()))
                .cloned()
                .collect(),
            options: self.options.clone(),
        }
    }

    /// Returns a clone that omits the requested segment kinds.
    #[must_use]
    pub fn exclude(&self, kinds: &[SegmentKind]) -> Self {
        Self {
            id: self.id.clone(),
            segments: self
                .segments
                .iter()
                .filter(|segment| !kinds.contains(&segment.kind()))
                .cloned()
                .collect(),
            options: self.options.clone(),
        }
    }

    /// Returns a clone produced by mapping or removing each segment.
    #[must_use]
    pub fn map_segments(
        &self,
        mut mapper: impl FnMut(&MessageSegment) -> Option<MessageSegment>,
    ) -> Self {
        Self {
            id: self.id.clone(),
            segments: self.segments.iter().filter_map(&mut mapper).collect(),
            options: self.options.clone(),
        }
    }

    /// Returns whether the message explicitly mentions `user_id`.
    #[must_use]
    pub fn is_related_to_user(&self, user_id: &UserId) -> bool {
        self.segments.iter().any(|segment| match segment {
            MessageSegment::At { user_id: id } => id == user_id,
            _ => false,
        })
    }

    /// Produces a capability-aware physical delivery plan without performing
    /// I/O. The same plan is used by handler replies and active sends.
    pub fn plan_for(
        &self,
        capabilities: &BotCapabilities,
        policy: FallbackPolicy,
    ) -> Result<DeliveryPlan, DeliveryPlanningError> {
        let mut message = Self {
            id: None,
            segments: Vec::with_capacity(self.segments.len()),
            options: self.options.clone(),
        };
        let mut degradations = Vec::new();

        for (index, segment) in self.segments.iter().enumerate() {
            adapt_segment(
                segment,
                index,
                capabilities,
                policy,
                &mut message.segments,
                &mut degradations,
            )?;
        }
        adapt_options(&mut message, capabilities, policy, &mut degradations)?;

        let messages = split_for_media_limit(message, capabilities.limits.max_media_per_message)
            .into_iter()
            .flat_map(|message| split_for_text_limit(message, capabilities.limits.max_text_length))
            .filter(|message| !message.is_empty())
            .collect::<Vec<_>>();
        if messages.is_empty() {
            return Err(DeliveryPlanningError {
                path: "message".to_owned(),
                feature: "deliverable content".to_owned(),
            });
        }
        if messages.len() > 1 {
            degradations.push(DeliveryDegradation {
                path: "message".to_owned(),
                feature: "platform limits".to_owned(),
                kind: DegradationKind::MessageSplit,
                detail: format!(
                    "logical message was split into {} physical messages",
                    messages.len()
                ),
            });
        }
        Ok(DeliveryPlan {
            messages,
            degradations,
        })
    }

    /// Estimates bytes retained by this message and its owned data.
    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        self.id
            .as_ref()
            .map_or(0, MessageId::estimated_bytes)
            .saturating_add(
                self.segments
                    .iter()
                    .map(MessageSegment::estimated_bytes)
                    .sum::<usize>(),
            )
            .saturating_add(self.options.estimated_bytes())
            .saturating_add(
                self.segments
                    .capacity()
                    .saturating_mul(std::mem::size_of::<MessageSegment>()),
            )
    }
}
