//! Command shortcuts and data-only schema overlays.
//!
//! These immutable authoring inputs are shared by generated modules and the
//! runtime registry while remaining independent of middleware execution.

use crate::{Command, HandlerError};
use regex::Regex;
use std::sync::Arc;

/// Maximum UTF-8 byte length of a shortcut's literal or regex pattern.
pub const MAX_SHORTCUT_PATTERN_BYTES: usize = 4 * 1024;
/// Maximum UTF-8 byte length of a shortcut replacement template.
pub const MAX_SHORTCUT_REPLACEMENT_BYTES: usize = 16 * 1024;
/// Maximum UTF-8 byte length of a shortcut's human-readable display text.
pub const MAX_SHORTCUT_HUMANIZED_BYTES: usize = 4 * 1024;
/// Maximum input bytes inspected when looking up runtime shortcuts.
pub const MAX_SHORTCUT_SCAN_PER_MESSAGE: usize = 4_096;
/// Maximum runtime shortcuts retained by one command registry.
pub const MAX_REGISTRY_SHORTCUTS: usize = 65_536;

/// One command shortcut compiled at application build time.
#[derive(Clone, Debug)]
pub struct Shortcut {
    pattern: ShortcutPattern,
    replacement: Arc<str>,
    humanized: Option<Arc<str>>,
    keep_tail: bool,
    compact: bool,
}

/// Matching representation used by a [`Shortcut`].
#[derive(Clone, Debug)]
pub enum ShortcutPattern {
    /// Prefix match against literal command text.
    Literal(Arc<str>),
    /// Prefix-anchored regular-expression match.
    Regex(Arc<Regex>),
}

impl Shortcut {
    /// Creates a literal prefix shortcut with a replacement command text.
    #[must_use]
    pub fn literal(pattern: impl Into<Arc<str>>, replacement: impl Into<Arc<str>>) -> Self {
        Self {
            pattern: ShortcutPattern::Literal(pattern.into()),
            replacement: replacement.into(),
            humanized: None,
            keep_tail: true,
            compact: false,
        }
    }

    /// Compiles a regular-expression shortcut with a replacement template.
    pub fn regex(pattern: &str, replacement: impl Into<Arc<str>>) -> Result<Self, regex::Error> {
        if pattern.is_empty() || pattern.len() > MAX_SHORTCUT_PATTERN_BYTES {
            return Err(regex::Error::Syntax(format!(
                "shortcut regex must contain 1..={MAX_SHORTCUT_PATTERN_BYTES} bytes"
            )));
        }
        Ok(Self {
            pattern: ShortcutPattern::Regex(Arc::new(Regex::new(pattern)?)),
            replacement: replacement.into(),
            humanized: None,
            keep_tail: true,
            compact: false,
        })
    }

    /// Sets presentation text used instead of the raw pattern in user interfaces.
    #[must_use]
    pub fn humanized(mut self, value: impl Into<Arc<str>>) -> Self {
        self.humanized = Some(value.into());
        self
    }

    /// Controls whether unmatched input text after a match is preserved.
    #[must_use]
    pub const fn keep_tail(mut self, enabled: bool) -> Self {
        self.keep_tail = enabled;
        self
    }

    /// Allows a literal shortcut to be immediately followed by its tail.
    /// By default a literal shortcut must end at a token boundary.
    #[must_use]
    pub const fn compact(mut self, enabled: bool) -> Self {
        self.compact = enabled;
        self
    }

    /// Returns whether this shortcut uses a regular-expression pattern.
    #[must_use]
    pub fn is_regex(&self) -> bool {
        matches!(self.pattern, ShortcutPattern::Regex(_))
    }

    /// Returns the literal or regular-expression source text.
    #[must_use]
    pub fn pattern_text(&self) -> &str {
        match &self.pattern {
            ShortcutPattern::Literal(value) => value,
            ShortcutPattern::Regex(value) => value.as_str(),
        }
    }

    /// Returns human-readable shortcut text, falling back to its pattern.
    #[must_use]
    pub fn display(&self) -> &str {
        self.humanized
            .as_deref()
            .unwrap_or_else(|| match &self.pattern {
                ShortcutPattern::Literal(value) => value,
                ShortcutPattern::Regex(value) => value.as_str(),
            })
    }

    /// Rewrites matching command input, or returns `None` when it does not match.
    #[must_use]
    pub fn rewrite(&self, input: &str) -> Option<String> {
        match &self.pattern {
            ShortcutPattern::Literal(pattern) => {
                let tail = input.strip_prefix(pattern.as_ref())?;
                if !self.compact
                    && !tail.is_empty()
                    && !tail.chars().next().is_some_and(char::is_whitespace)
                {
                    return None;
                }
                if !self.keep_tail && !tail.trim().is_empty() {
                    return None;
                }
                Some(if self.keep_tail {
                    format!("{}{}", self.replacement, tail)
                } else {
                    self.replacement.to_string()
                })
            }
            ShortcutPattern::Regex(pattern) => {
                let captures = pattern.captures(input)?;
                if captures.get(0).is_some_and(|capture| capture.start() != 0) {
                    return None;
                }
                Some(expand_replacement(
                    &self.replacement,
                    &captures,
                    input,
                    self.keep_tail,
                ))
            }
        }
    }

    pub(crate) fn validate(&self) -> Result<(), HandlerError> {
        if self.pattern_text().is_empty() || self.pattern_text().len() > MAX_SHORTCUT_PATTERN_BYTES
        {
            return Err(HandlerError::internal(
                "shortcut pattern byte limit exceeded",
            ));
        }
        if self.replacement.len() > MAX_SHORTCUT_REPLACEMENT_BYTES {
            return Err(HandlerError::internal(
                "shortcut replacement byte limit exceeded",
            ));
        }
        if self
            .humanized
            .as_ref()
            .is_some_and(|value| value.len() > MAX_SHORTCUT_HUMANIZED_BYTES)
        {
            return Err(HandlerError::internal(
                "shortcut display text byte limit exceeded",
            ));
        }
        Ok(())
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        self.pattern_text()
            .len()
            .saturating_add(self.replacement.len())
            .saturating_add(self.humanized.as_ref().map_or(0, |value| value.len()))
            .saturating_add(128)
    }
}

/// Data-only command metadata overlay. It can adjust presentation, prefixes,
/// aliases, and shortcuts without replacing the statically compiled handler or
/// executing code from configuration files.
#[derive(Clone, Debug)]
pub struct CommandOverlay {
    target: Arc<str>,
    description: Option<Arc<str>>,
    translations: Vec<(Arc<str>, Arc<str>)>,
    aliases: Vec<Arc<str>>,
    prefixes: Option<Vec<Arc<str>>>,
    shortcuts: Vec<Shortcut>,
    hidden: bool,
}

impl CommandOverlay {
    /// Creates a data-only overlay targeting a command name or alias.
    #[must_use]
    pub fn new(target: impl Into<Arc<str>>) -> Self {
        Self {
            target: target.into(),
            description: None,
            translations: Vec::new(),
            aliases: Vec::new(),
            prefixes: None,
            shortcuts: Vec::new(),
            hidden: false,
        }
    }

    /// Replaces the command's default description.
    #[must_use]
    pub fn description(mut self, value: impl Into<Arc<str>>) -> Self {
        self.description = Some(value.into());
        self
    }

    /// Adds a localized description override for `locale`.
    #[must_use]
    pub fn description_translation(
        mut self,
        locale: impl Into<Arc<str>>,
        value: impl Into<Arc<str>>,
    ) -> Self {
        self.translations.push((locale.into(), value.into()));
        self
    }

    /// Adds an alias to the targeted command.
    #[must_use]
    pub fn alias(mut self, value: impl Into<Arc<str>>) -> Self {
        self.aliases.push(value.into());
        self
    }

    /// Replaces command prefixes with `values`.
    #[must_use]
    pub fn prefixes<I, T>(mut self, values: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<Arc<str>>,
    {
        self.prefixes = Some(values.into_iter().map(Into::into).collect());
        self
    }

    /// Adds a static shortcut to the targeted command.
    #[must_use]
    pub fn shortcut(mut self, value: Shortcut) -> Self {
        self.shortcuts.push(value);
        self
    }

    /// Hides the targeted command from generated catalogs and help.
    #[must_use]
    pub const fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }

    pub(crate) fn matches(&self, command: &Command) -> bool {
        command.name().eq_ignore_ascii_case(&self.target)
            || command
                .aliases_list()
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(&self.target))
    }

    pub(crate) fn apply(&self, mut command: Command) -> Command {
        if let Some(description) = &self.description {
            command = command.description(Arc::clone(description));
        }
        for (locale, value) in &self.translations {
            command = command.description_translation(Arc::clone(locale), Arc::clone(value));
        }
        command = command.aliases(self.aliases.iter().cloned());
        if let Some(prefixes) = &self.prefixes {
            command = command.prefixes(prefixes.iter().cloned());
        }
        for shortcut in &self.shortcuts {
            command = command.shortcut(shortcut.clone());
        }
        if self.hidden {
            command = command.hidden();
        }
        command
    }
}

fn expand_replacement(
    template: &str,
    captures: &regex::Captures<'_>,
    input: &str,
    keep_tail: bool,
) -> String {
    let mut output = String::with_capacity(template.len().saturating_add(input.len()));
    let mut used_tail = false;
    let mut chars = template.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '{' {
            output.push(ch);
            continue;
        }
        let mut key = String::new();
        for next in chars.by_ref() {
            if next == '}' {
                break;
            }
            key.push(next);
        }
        if key == "*" {
            used_tail = true;
            if keep_tail {
                let end = captures.get(0).map_or(0, |capture| capture.end());
                output.push_str(input.get(end..).unwrap_or_default().trim_start());
            }
        } else if let Ok(index) = key.parse::<usize>() {
            if let Some(value) = captures.get(index) {
                output.push_str(value.as_str());
            }
        } else if let Some(value) = captures.name(&key) {
            output.push_str(value.as_str());
        }
    }
    if keep_tail && !used_tail {
        let end = captures.get(0).map_or(0, |capture| capture.end());
        output.push_str(input.get(end..).unwrap_or_default());
    }
    output
}
