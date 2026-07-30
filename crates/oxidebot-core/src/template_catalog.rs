use super::{MessageTemplate, TemplateError, TemplateValue};
use crate::Message;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt, fs,
    io::{Read, Take},
    path::Path,
    sync::{Arc, RwLock},
};

type TranslationEntry = ((Arc<str>, Arc<str>), MessageTemplate);

/// Maximum UTF-8 bytes in a translation locale identifier.
pub const MAX_TRANSLATION_LOCALE_BYTES: usize = 256;
/// Maximum UTF-8 bytes in a translation key.
pub const MAX_TRANSLATION_KEY_BYTES: usize = 4 * 1024;
/// Maximum UTF-8 bytes in one translation template source.
pub const MAX_TRANSLATION_TEMPLATE_BYTES: usize = 1024 * 1024;
/// Maximum UTF-8 bytes in one translation JSON document.
pub const MAX_TRANSLATION_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;

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
    max_bytes: usize,
    retained_bytes: usize,
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

    /// Creates a count-bounded catalog with a derived byte limit.
    #[must_use]
    pub fn bounded(capacity: usize, default_locale: impl Into<Arc<str>>) -> Self {
        let capacity = capacity.max(1);
        let default_max_bytes = capacity.saturating_mul(64 * 1024).min(256 * 1024 * 1024);
        Self::bounded_bytes(capacity, default_max_bytes, default_locale)
    }

    /// Creates a catalog bounded by both entry count and retained source
    /// bytes. The byte budget includes locale, key, and template source text.
    #[must_use]
    pub fn bounded_bytes(
        capacity: usize,
        max_bytes: usize,
        default_locale: impl Into<Arc<str>>,
    ) -> Self {
        Self {
            inner: Arc::new(RwLock::new(TranslationCatalogState {
                capacity: capacity.max(1),
                max_bytes: max_bytes.max(1),
                retained_bytes: 0,
                default_locale: normalize_locale(default_locale.into()),
                templates: HashMap::new(),
            })),
        }
    }

    /// Parses and inserts one locale/key/template entry.
    pub fn insert(
        &self,
        locale: impl Into<Arc<str>>,
        key: impl Into<Arc<str>>,
        template: impl Into<Arc<str>>,
    ) -> Result<(), TranslationError> {
        let locale = normalize_locale(locale.into());
        let key = key.into();
        let template = template.into();
        validate_translation_entry(&locale, &key, &template)?;
        let template = MessageTemplate::parse(template).map_err(TranslationError::Template)?;
        let mut state = self
            .inner
            .write()
            .expect("translation catalog lock poisoned");
        let map_key = (locale, key);
        if !state.templates.contains_key(&map_key) && state.templates.len() >= state.capacity {
            return Err(TranslationError::CapacityExceeded);
        }
        let previous_bytes = state
            .templates
            .get(&map_key)
            .map_or(0, |previous| translation_entry_bytes(&map_key, previous));
        let next_bytes = translation_entry_bytes(&map_key, &template);
        let retained_bytes = state
            .retained_bytes
            .saturating_sub(previous_bytes)
            .saturating_add(next_bytes);
        if retained_bytes > state.max_bytes {
            return Err(TranslationError::ByteCapacityExceeded);
        }
        state.templates.insert(map_key, template);
        state.retained_bytes = retained_bytes;
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
        let max_document_bytes = self
            .inner
            .read()
            .expect("translation catalog lock poisoned")
            .max_bytes
            .min(MAX_TRANSLATION_DOCUMENT_BYTES);
        let entries = parse_translation_file(locale.into(), path.as_ref(), max_document_bytes)?;
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
            let max_document_bytes = self
                .inner
                .read()
                .expect("translation catalog lock poisoned")
                .max_bytes
                .min(MAX_TRANSLATION_DOCUMENT_BYTES);
            entries.extend(parse_translation_file(
                Arc::<str>::from(locale),
                &path,
                max_document_bytes,
            )?);
        }
        self.insert_parsed(entries)
    }

    fn insert_parsed(&self, entries: Vec<TranslationEntry>) -> Result<usize, TranslationError> {
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
        let mut retained_bytes = state.retained_bytes;
        for (key, template) in &entries {
            retained_bytes = retained_bytes.saturating_sub(
                state
                    .templates
                    .get(key)
                    .map_or(0, |previous| translation_entry_bytes(key, previous)),
            );
            retained_bytes = retained_bytes.saturating_add(translation_entry_bytes(key, template));
        }
        if retained_bytes > state.max_bytes {
            return Err(TranslationError::ByteCapacityExceeded);
        }
        let count = entries.len();
        for (key, template) in entries {
            state.templates.insert(key, template);
        }
        state.retained_bytes = retained_bytes;
        Ok(count)
    }

    /// Returns whether a template resolves for locale and key.
    #[must_use]
    pub fn contains(&self, locale: &str, key: &str) -> bool {
        self.find_template(locale, key).is_some()
    }

    /// Resolves and renders a localized template.
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
    max_document_bytes: usize,
) -> Result<Vec<TranslationEntry>, TranslationError> {
    let file = fs::File::open(path).map_err(|error| {
        TranslationError::Io(format!("could not open {}: {error}", path.display()))
    })?;
    let limit = u64::try_from(max_document_bytes.saturating_add(1)).unwrap_or(u64::MAX);
    let mut reader: Take<fs::File> = file.take(limit);
    let mut bytes = Vec::with_capacity(max_document_bytes.min(64 * 1024));
    reader.read_to_end(&mut bytes).map_err(|error| {
        TranslationError::Io(format!("could not read {}: {error}", path.display()))
    })?;
    if bytes.len() > max_document_bytes {
        return Err(TranslationError::DocumentTooLarge {
            limit: max_document_bytes,
        });
    }
    let source = String::from_utf8(bytes).map_err(|error| {
        TranslationError::Document(format!(
            "translation JSON {} is not UTF-8: {error}",
            path.display(),
        ))
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
            let key = Arc::<str>::from(key);
            let source = Arc::<str>::from(template);
            validate_translation_entry(&locale, &key, &source)?;
            let template = MessageTemplate::parse(source).map_err(TranslationError::Template)?;
            Ok(((Arc::clone(&locale), key), template))
        })
        .collect()
}

fn validate_translation_entry(
    locale: &str,
    key: &str,
    template: &str,
) -> Result<(), TranslationError> {
    if locale.is_empty() || locale.len() > MAX_TRANSLATION_LOCALE_BYTES {
        return Err(TranslationError::ValueTooLarge("locale"));
    }
    if key.is_empty() || key.len() > MAX_TRANSLATION_KEY_BYTES {
        return Err(TranslationError::ValueTooLarge("translation key"));
    }
    if template.len() > MAX_TRANSLATION_TEMPLATE_BYTES {
        return Err(TranslationError::ValueTooLarge("translation template"));
    }
    Ok(())
}

fn translation_entry_bytes(key: &(Arc<str>, Arc<str>), template: &MessageTemplate) -> usize {
    key.0
        .len()
        .saturating_add(key.1.len())
        .saturating_add(template.source().len())
        .saturating_add(128)
}

fn normalize_locale(locale: Arc<str>) -> Arc<str> {
    Arc::from(locale.trim().replace('_', "-"))
}

/// Keyed message that can be rendered by a translation catalog.
#[derive(Clone, Debug)]
pub struct LocalizedMessage {
    key: Arc<str>,
    values: BTreeMap<Arc<str>, TemplateValue>,
}

impl LocalizedMessage {
    /// Creates a localized-message request for one template key.
    #[must_use]
    pub fn new(key: impl Into<Arc<str>>) -> Self {
        Self {
            key: key.into(),
            values: BTreeMap::new(),
        }
    }

    /// Adds or replaces one named template value.
    #[must_use]
    pub fn arg(mut self, name: impl Into<Arc<str>>, value: impl Into<TemplateValue>) -> Self {
        self.values.insert(name.into(), value.into());
        self
    }

    /// Returns the template key.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Returns named template values.
    #[must_use]
    pub fn values(&self) -> &BTreeMap<Arc<str>, TemplateValue> {
        &self.values
    }

    /// Renders this request using a catalog and optional locale.
    pub fn render(
        &self,
        catalog: &TranslationCatalog,
        locale: Option<&str>,
    ) -> Result<Message, TranslationError> {
        catalog.render(locale, &self.key, &self.values)
    }
}

/// Error while loading, storing, or resolving translations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TranslationError {
    /// Entry count capacity would be exceeded.
    CapacityExceeded,
    /// Retained-byte capacity would be exceeded.
    ByteCapacityExceeded,
    /// Translation JSON document exceeded its byte limit.
    DocumentTooLarge {
        /// Maximum permitted document bytes.
        limit: usize,
    },
    /// Locale, key, or template value exceeded its byte limit.
    ValueTooLarge(&'static str),
    /// No template could be resolved for locale and key.
    MissingKey {
        /// Requested locale.
        locale: String,
        /// Requested template key.
        key: String,
    },
    /// Filesystem I/O failed.
    Io(String),
    /// Translation document was malformed.
    Document(String),
    /// Message template parsing or rendering failed.
    Template(TemplateError),
}

impl fmt::Display for TranslationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CapacityExceeded => formatter.write_str("translation catalog capacity exceeded"),
            Self::ByteCapacityExceeded => {
                formatter.write_str("translation catalog byte capacity exceeded")
            }
            Self::DocumentTooLarge { limit } => {
                write!(formatter, "translation document exceeds {limit} bytes")
            }
            Self::ValueTooLarge(value) => {
                write!(
                    formatter,
                    "{value} exceeds its translation catalog byte limit"
                )
            }
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

#[cfg(test)]
mod catalog_limit_tests {
    use super::*;

    #[test]
    fn catalog_enforces_per_value_and_total_byte_limits() {
        let catalog = TranslationCatalog::bounded_bytes(8, 256, "en");
        catalog
            .insert("en", "small", "hello")
            .expect("small translation fits");
        let error = catalog
            .insert("en", "large", "x".repeat(512))
            .expect_err("total catalog byte budget rejects large value");
        assert_eq!(error, TranslationError::ByteCapacityExceeded);

        let error = TranslationCatalog::bounded_bytes(8, 2 * 1024 * 1024, "en")
            .insert("en", "key", "x".repeat(MAX_TRANSLATION_TEMPLATE_BYTES + 1))
            .expect_err("single template limit is enforced");
        assert_eq!(
            error,
            TranslationError::ValueTooLarge("translation template")
        );
    }
}
