//! Structure-preserving message templates and typed segment selectors.

use crate::{File, Message, MessageSegment, SegmentKind};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt, fs,
    path::Path,
    sync::{Arc, RwLock},
};

pub trait SegmentSelector: Send + Sync + 'static {
    const KIND: SegmentKind;
}

macro_rules! selectors {
    ($($name:ident => $kind:ident),* $(,)?) => {$(
        #[derive(Clone, Copy, Debug, Default)]
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
    Replies => Reply,
    UserMentions => MentionUser,
    RoleMentions => MentionRole,
    ChannelMentions => MentionChannel,
    References => Reference,
    Forwards => Forward,
    Polls => Poll,
    Components => Components,
    NativeSegments => PlatformNative,
}

impl Message {
    #[must_use]
    pub fn has_type<T: SegmentSelector>(&self) -> bool {
        self.has(T::KIND)
    }

    #[must_use]
    pub fn first_type<T: SegmentSelector>(&self) -> Option<&MessageSegment> {
        self.first(T::KIND)
    }

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

    #[must_use]
    pub fn only_type<T: SegmentSelector>(&self) -> bool {
        !self.segments.is_empty()
            && self
                .segments
                .iter()
                .all(|segment| segment.kind() == T::KIND)
    }

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

#[derive(Clone, Debug)]
pub enum SegmentTransform {
    Keep,
    Drop,
    Replace(Box<MessageSegment>),
    Expand(Vec<MessageSegment>),
}

#[derive(Clone, Debug)]
pub enum TemplateValue {
    Text(String),
    Message(Message),
    Segment(MessageSegment),
    Mention(String),
    File(File),
}

impl TemplateValue {
    #[must_use]
    pub fn mention(user_id: impl Into<String>) -> Self {
        Self::Mention(user_id.into())
    }

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
        Self::Message(value)
    }
}
impl From<MessageSegment> for TemplateValue {
    fn from(value: MessageSegment) -> Self {
        Self::Segment(value)
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

    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

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
            output.push(value.clone())
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
            TemplateValue::Mention(value) => value.clone(),
            TemplateValue::File(value) => value.name.clone(),
            TemplateValue::Segment(value) => value.fallback_text().unwrap_or_default(),
            TemplateValue::Message(value) => value.extract_plain_text(),
        }),
        _ => return Err(TemplateError::TypeMismatch(name.to_owned())),
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TemplateError {
    UnclosedPlaceholder,
    EmptyPlaceholder,
    UnknownKind(String),
    MissingValue(String),
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

/// Bounded structure-preserving translation catalog. Values are parsed as
/// `MessageTemplate`s, so translations may contain mentions, files, or other
/// canonical message segments instead of being flattened to text.
#[derive(Clone, Debug)]
pub struct TranslationCatalog {
    inner: Arc<RwLock<TranslationCatalogState>>,
}

#[derive(Debug)]
struct TranslationCatalogState {
    capacity: usize,
    default_locale: Arc<str>,
    templates: HashMap<(Arc<str>, Arc<str>), MessageTemplate>,
}

impl TranslationCatalog {
    /// Creates a bounded catalog and loads every `<locale>.json` resource in
    /// one directory.
    pub fn from_dir(
        capacity: usize,
        default_locale: impl Into<Arc<str>>,
        directory: impl AsRef<Path>,
    ) -> Result<Self, TranslationError> {
        let catalog = Self::bounded(capacity, default_locale);
        catalog.load_dir(directory)?;
        Ok(catalog)
    }

    #[must_use]
    pub fn bounded(capacity: usize, default_locale: impl Into<Arc<str>>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(TranslationCatalogState {
                capacity: capacity.max(1),
                default_locale: normalize_locale(default_locale.into()),
                templates: HashMap::new(),
            })),
        }
    }

    pub fn insert(
        &self,
        locale: impl Into<Arc<str>>,
        key: impl Into<Arc<str>>,
        template: impl Into<Arc<str>>,
    ) -> Result<(), TranslationError> {
        let locale = normalize_locale(locale.into());
        let key = key.into();
        let template = MessageTemplate::parse(template).map_err(TranslationError::Template)?;
        let mut state = self
            .inner
            .write()
            .expect("translation catalog lock poisoned");
        let map_key = (locale, key);
        if !state.templates.contains_key(&map_key) && state.templates.len() >= state.capacity {
            return Err(TranslationError::CapacityExceeded);
        }
        state.templates.insert(map_key, template);
        Ok(())
    }

    /// Loads one JSON object whose keys are translation keys and whose values
    /// are structure-preserving message templates. Parsing and capacity
    /// validation finish before the catalog is mutated, so a malformed file
    /// cannot leave a partially loaded locale behind.
    pub fn load_json_file(
        &self,
        locale: impl Into<Arc<str>>,
        path: impl AsRef<Path>,
    ) -> Result<usize, TranslationError> {
        let entries = parse_translation_file(locale.into(), path.as_ref())?;
        self.insert_parsed(entries)
    }

    /// Loads every `<locale>.json` file from a directory in deterministic file
    /// name order. The entire directory is parsed and capacity-checked before
    /// any entry is committed.
    pub fn load_dir(&self, directory: impl AsRef<Path>) -> Result<usize, TranslationError> {
        let directory = directory.as_ref();
        let mut files = fs::read_dir(directory)
            .map_err(|error| {
                TranslationError::Io(format!(
                    "could not read translation directory {}: {error}",
                    directory.display(),
                ))
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| TranslationError::Io(error.to_string()))?;
        files.sort_by_key(|entry| entry.file_name());

        let mut entries = Vec::new();
        for entry in files {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let locale = path
                .file_stem()
                .and_then(|value| value.to_str())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    TranslationError::Document(format!(
                        "translation file {} has no locale file stem",
                        path.display(),
                    ))
                })?;
            entries.extend(parse_translation_file(Arc::<str>::from(locale), &path)?);
        }
        self.insert_parsed(entries)
    }

    fn insert_parsed(
        &self,
        entries: Vec<((Arc<str>, Arc<str>), MessageTemplate)>,
    ) -> Result<usize, TranslationError> {
        let mut state = self
            .inner
            .write()
            .expect("translation catalog lock poisoned");
        let mut new_keys = HashSet::new();
        for (key, _) in &entries {
            if !state.templates.contains_key(key) {
                new_keys.insert(key.clone());
            }
        }
        if state.templates.len().saturating_add(new_keys.len()) > state.capacity {
            return Err(TranslationError::CapacityExceeded);
        }
        let count = entries.len();
        for (key, template) in entries {
            state.templates.insert(key, template);
        }
        Ok(count)
    }

    #[must_use]
    pub fn contains(&self, locale: &str, key: &str) -> bool {
        self.find_template(locale, key).is_some()
    }

    pub fn render(
        &self,
        locale: Option<&str>,
        key: &str,
        values: &BTreeMap<Arc<str>, TemplateValue>,
    ) -> Result<Message, TranslationError> {
        let template = self
            .find_template(locale.unwrap_or_default(), key)
            .ok_or_else(|| TranslationError::MissingKey {
                locale: locale.unwrap_or_default().to_owned(),
                key: key.to_owned(),
            })?;
        template.render(values).map_err(TranslationError::Template)
    }

    fn find_template(&self, locale: &str, key: &str) -> Option<MessageTemplate> {
        let state = self
            .inner
            .read()
            .expect("translation catalog lock poisoned");
        let normalized = normalize_locale(Arc::<str>::from(locale));
        let language = normalized
            .split(&['-', '_'][..])
            .next()
            .filter(|value| !value.is_empty())
            .map(Arc::<str>::from);
        state
            .templates
            .get(&(Arc::clone(&normalized), Arc::from(key)))
            .or_else(|| {
                language.as_ref().and_then(|language| {
                    state.templates.get(&(Arc::clone(language), Arc::from(key)))
                })
            })
            .or_else(|| {
                state
                    .templates
                    .get(&(Arc::clone(&state.default_locale), Arc::from(key)))
            })
            .cloned()
    }
}

fn parse_translation_file(
    locale: Arc<str>,
    path: &Path,
) -> Result<Vec<((Arc<str>, Arc<str>), MessageTemplate)>, TranslationError> {
    let source = fs::read_to_string(path).map_err(|error| {
        TranslationError::Io(format!("could not read {}: {error}", path.display()))
    })?;
    let values: BTreeMap<String, String> = serde_json::from_str(&source).map_err(|error| {
        TranslationError::Document(format!(
            "invalid translation JSON {}: {error}",
            path.display(),
        ))
    })?;
    let locale = normalize_locale(locale);
    values
        .into_iter()
        .map(|(key, template)| {
            let template = MessageTemplate::parse(template).map_err(TranslationError::Template)?;
            Ok(((Arc::clone(&locale), Arc::<str>::from(key)), template))
        })
        .collect()
}

fn normalize_locale(locale: Arc<str>) -> Arc<str> {
    Arc::from(locale.trim().replace('_', "-"))
}

#[derive(Clone, Debug)]
pub struct LocalizedMessage {
    key: Arc<str>,
    values: BTreeMap<Arc<str>, TemplateValue>,
}

impl LocalizedMessage {
    #[must_use]
    pub fn new(key: impl Into<Arc<str>>) -> Self {
        Self {
            key: key.into(),
            values: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn arg(mut self, name: impl Into<Arc<str>>, value: impl Into<TemplateValue>) -> Self {
        self.values.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    #[must_use]
    pub fn values(&self) -> &BTreeMap<Arc<str>, TemplateValue> {
        &self.values
    }

    pub fn render(
        &self,
        catalog: &TranslationCatalog,
        locale: Option<&str>,
    ) -> Result<Message, TranslationError> {
        catalog.render(locale, &self.key, &self.values)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TranslationError {
    CapacityExceeded,
    MissingKey { locale: String, key: String },
    Io(String),
    Document(String),
    Template(TemplateError),
}

impl fmt::Display for TranslationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CapacityExceeded => formatter.write_str("translation catalog capacity exceeded"),
            Self::MissingKey { locale, key } => {
                write!(
                    formatter,
                    "missing translation `{key}` for locale `{locale}`"
                )
            }
            Self::Io(error) | Self::Document(error) => formatter.write_str(error),
            Self::Template(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for TranslationError {}
