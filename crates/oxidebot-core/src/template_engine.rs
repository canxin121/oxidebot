use crate::{File, Message, MessageSegment, SegmentKind, UserId};
use std::{collections::BTreeMap, fmt, sync::Arc};

/// Compile-time selector for a portable message segment kind.
pub trait SegmentSelector: Send + Sync + 'static {
    /// Segment kind selected by this marker.
    const KIND: SegmentKind;
}

macro_rules! selectors {
    ($($name:ident => $kind:ident),* $(,)?) => {$(
        #[derive(Clone, Copy, Debug, Default)]
        #[doc = concat!("Compile-time selector for [`SegmentKind::", stringify!($kind), "`].")]
        pub struct $name;
        impl SegmentSelector for $name {
            const KIND: SegmentKind = SegmentKind::$kind;
        }
    )*};
}

selectors! {
    TextSegments => Text,
    RichTextSegments => RichText,
    Images => Image,
    Videos => Video,
    Audios => Audio,
    Files => File,
    UserMentions => MentionUser,
    RoleMentions => MentionRole,
    ChannelMentions => MentionChannel,
    References => Reference,
    Forwards => Forward,
    Polls => Poll,
    NativeSegments => PlatformNative,
}

impl Message {
    /// Returns whether the message contains a segment selected by `T`.
    #[must_use]
    pub fn has_type<T: SegmentSelector>(&self) -> bool {
        self.has(T::KIND)
    }

    /// Returns the first segment selected by `T`.
    #[must_use]
    pub fn first_type<T: SegmentSelector>(&self) -> Option<&MessageSegment> {
        self.first(T::KIND)
    }

    /// Iterates segments selected by `T`.
    pub fn segments_of<T: SegmentSelector>(
        &self,
    ) -> impl DoubleEndedIterator<Item = &MessageSegment> {
        self.select(T::KIND)
    }

    /// Recursively selects segments from nested forwarded custom messages.
    pub fn select_recursive<T: SegmentSelector>(&self) -> Vec<&MessageSegment> {
        fn visit<'a>(
            message: &'a Message,
            kind: SegmentKind,
            output: &mut Vec<&'a MessageSegment>,
        ) {
            for segment in &message.segments {
                if segment.kind() == kind {
                    output.push(segment);
                }
                if let MessageSegment::ForwardCustomNode { message, .. } = segment {
                    visit(message, kind, output);
                }
            }
        }
        let mut output = Vec::new();
        visit(self, T::KIND, &mut output);
        output
    }

    /// Returns whether every non-empty segment is selected by `T`.
    #[must_use]
    pub fn only_type<T: SegmentSelector>(&self) -> bool {
        !self.segments.is_empty()
            && self
                .segments
                .iter()
                .all(|segment| segment.kind() == T::KIND)
    }

    /// Recursively transforms segments within custom forwarded messages.
    #[must_use]
    pub fn transform_recursive(
        &self,
        mut transform: impl FnMut(&MessageSegment) -> SegmentTransform,
    ) -> Message {
        fn visit(
            message: &Message,
            transform: &mut impl FnMut(&MessageSegment) -> SegmentTransform,
        ) -> Message {
            let mut output = Message {
                id: message.id.clone(),
                options: message.options.clone(),
                ..Message::default()
            };
            for segment in &message.segments {
                let nested = match segment {
                    MessageSegment::ForwardCustomNode { user, message } => {
                        MessageSegment::ForwardCustomNode {
                            user: user.clone(),
                            message: Box::new(visit(message, transform)),
                        }
                    }
                    other => other.clone(),
                };
                match transform(&nested) {
                    SegmentTransform::Keep => output.push(nested),
                    SegmentTransform::Drop => {}
                    SegmentTransform::Replace(value) => output.push(*value),
                    SegmentTransform::Expand(values) => output.extend(values),
                }
            }
            output
        }
        visit(self, &mut transform)
    }
}

/// Result of transforming one message segment.
#[derive(Clone, Debug)]
pub enum SegmentTransform {
    /// Preserve the segment unchanged.
    Keep,
    /// Omit the segment.
    Drop,
    /// Replace it with one segment.
    Replace(Box<MessageSegment>),
    /// Replace it with multiple segments.
    Expand(Vec<MessageSegment>),
}

/// Typed value substituted into a [`MessageTemplate`].
#[derive(Clone, Debug)]
pub enum TemplateValue {
    /// Plain text value.
    Text(String),
    /// Complete portable message.
    Message(Box<Message>),
    /// One portable segment.
    Segment(Box<MessageSegment>),
    /// User identifier rendered as a mention.
    Mention(UserId),
    /// File rendered as an attachment.
    File(File),
}

impl TemplateValue {
    /// Creates a mention value.
    #[must_use]
    pub fn mention(user_id: impl Into<UserId>) -> Self {
        Self::Mention(user_id.into())
    }

    /// Creates a text value.
    #[must_use]
    pub fn text(value: impl Into<String>) -> Self {
        Self::Text(value.into())
    }
}

impl From<String> for TemplateValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}
impl From<&str> for TemplateValue {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}
impl From<&String> for TemplateValue {
    fn from(value: &String) -> Self {
        Self::Text(value.clone())
    }
}
impl From<Arc<str>> for TemplateValue {
    fn from(value: Arc<str>) -> Self {
        Self::Text(value.to_string())
    }
}
impl From<bool> for TemplateValue {
    fn from(value: bool) -> Self {
        Self::Text(value.to_string())
    }
}
impl From<char> for TemplateValue {
    fn from(value: char) -> Self {
        Self::Text(value.to_string())
    }
}
impl From<Message> for TemplateValue {
    fn from(value: Message) -> Self {
        Self::Message(Box::new(value))
    }
}
impl From<MessageSegment> for TemplateValue {
    fn from(value: MessageSegment) -> Self {
        Self::Segment(Box::new(value))
    }
}
impl From<File> for TemplateValue {
    fn from(value: File) -> Self {
        Self::File(value)
    }
}
macro_rules! template_numbers {
    ($($type:ty),* $(,)?) => {$(
        impl From<$type> for TemplateValue {
            fn from(value: $type) -> Self {
                Self::Text(value.to_string())
            }
        }
    )*};
}

template_numbers!(i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64,);

/// Parsed structure-preserving message template.
#[derive(Clone, Debug)]
pub struct MessageTemplate {
    source: Arc<str>,
    parts: Arc<[TemplatePart]>,
}

#[derive(Clone, Debug)]
enum TemplatePart {
    Text(Arc<str>),
    Placeholder { name: Arc<str>, kind: TemplateKind },
}

#[derive(Clone, Copy, Debug)]
enum TemplateKind {
    Auto,
    Text,
    Mention,
    File,
    Segment,
    Message,
}

impl MessageTemplate {
    /// Parses a template with `{name[:kind]}` placeholders and escaped braces.
    pub fn parse(source: impl Into<Arc<str>>) -> Result<Self, TemplateError> {
        let source = source.into();
        let mut parts = Vec::new();
        let mut text = String::new();
        let mut chars = source.char_indices().peekable();
        while let Some((_, ch)) = chars.next() {
            if ch == '}' && chars.peek().is_some_and(|(_, next)| *next == '}') {
                chars.next();
                text.push('}');
                continue;
            }
            if ch != '{' {
                text.push(ch);
                continue;
            }
            if chars.peek().is_some_and(|(_, next)| *next == '{') {
                chars.next();
                text.push('{');
                continue;
            }
            if !text.is_empty() {
                parts.push(TemplatePart::Text(Arc::from(std::mem::take(&mut text))));
            }
            let mut body = String::new();
            let mut closed = false;
            for (_, next) in chars.by_ref() {
                if next == '}' {
                    closed = true;
                    break;
                }
                body.push(next);
            }
            if !closed {
                return Err(TemplateError::UnclosedPlaceholder);
            }
            let (name, kind) = body.split_once(':').map_or((body.as_str(), "auto"), |v| v);
            if name.trim().is_empty() {
                return Err(TemplateError::EmptyPlaceholder);
            }
            let kind = match kind.trim() {
                "auto" | "" => TemplateKind::Auto,
                "text" => TemplateKind::Text,
                "mention" | "at" => TemplateKind::Mention,
                "file" => TemplateKind::File,
                "segment" => TemplateKind::Segment,
                "message" => TemplateKind::Message,
                other => return Err(TemplateError::UnknownKind(other.to_owned())),
            };
            parts.push(TemplatePart::Placeholder {
                name: Arc::from(name.trim()),
                kind,
            });
        }
        if !text.is_empty() {
            parts.push(TemplatePart::Text(Arc::from(text)));
        }
        Ok(Self {
            source,
            parts: parts.into(),
        })
    }

    /// Returns original template source.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Renders the template into a portable message using supplied values.
    pub fn render(
        &self,
        values: &BTreeMap<Arc<str>, TemplateValue>,
    ) -> Result<Message, TemplateError> {
        let mut output = Message::default();
        for part in self.parts.iter() {
            match part {
                TemplatePart::Text(value) => output.push(value.as_ref()),
                TemplatePart::Placeholder { name, kind } => {
                    let value = values
                        .get(name)
                        .ok_or_else(|| TemplateError::MissingValue(name.to_string()))?;
                    render_value(&mut output, name, *kind, value)?;
                }
            }
        }
        Ok(output)
    }
}

fn render_value(
    output: &mut Message,
    name: &str,
    kind: TemplateKind,
    value: &TemplateValue,
) -> Result<(), TemplateError> {
    match (kind, value) {
        (TemplateKind::Auto | TemplateKind::Text, TemplateValue::Text(value)) => output.push(value),
        (TemplateKind::Auto | TemplateKind::Message, TemplateValue::Message(value)) => {
            output.extend(value.segments.clone())
        }
        (TemplateKind::Auto | TemplateKind::Segment, TemplateValue::Segment(value)) => {
            output.push(value.as_ref().clone())
        }
        (TemplateKind::Auto | TemplateKind::Mention, TemplateValue::Mention(value)) => {
            output.push(MessageSegment::at(value.clone()))
        }
        (TemplateKind::Auto | TemplateKind::File, TemplateValue::File(value)) => {
            output.push(MessageSegment::file(value.clone()))
        }
        (TemplateKind::Mention, TemplateValue::Text(value)) => {
            output.push(MessageSegment::at(value.clone()))
        }
        (TemplateKind::Text, value) => output.push(match value {
            TemplateValue::Text(value) => value.clone(),
            TemplateValue::Mention(value) => value.to_string(),
            TemplateValue::File(value) => value.name.clone(),
            TemplateValue::Segment(value) => value.fallback_text().unwrap_or_default(),
            TemplateValue::Message(value) => value.extract_plain_text(),
        }),
        _ => return Err(TemplateError::TypeMismatch(name.to_owned())),
    }
    Ok(())
}

/// Error while parsing or rendering a message template.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TemplateError {
    /// Placeholder started but was not closed.
    UnclosedPlaceholder,
    /// Placeholder name was empty.
    EmptyPlaceholder,
    /// Placeholder kind was unknown.
    UnknownKind(String),
    /// Required named value was absent.
    MissingValue(String),
    /// Value cannot be rendered using requested placeholder kind.
    TypeMismatch(String),
}

impl fmt::Display for TemplateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnclosedPlaceholder => {
                formatter.write_str("unclosed message template placeholder")
            }
            Self::EmptyPlaceholder => {
                formatter.write_str("message template placeholder cannot be empty")
            }
            Self::UnknownKind(kind) => write!(formatter, "unknown message template kind `{kind}`"),
            Self::MissingValue(name) => {
                write!(formatter, "missing message template value `{name}`")
            }
            Self::TypeMismatch(name) => write!(
                formatter,
                "message template value `{name}` has the wrong type"
            ),
        }
    }
}
impl std::error::Error for TemplateError {}
