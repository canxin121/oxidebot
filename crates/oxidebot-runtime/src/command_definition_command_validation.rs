//! Structural validation and routing metadata for command trees.

use super::*;

impl Command {
    /// Stable-in-process structural fingerprint used to ensure one public
    /// command ID never refers to different parser grammars in the shared
    /// registry. Presentation-only metadata is intentionally excluded.
    pub(crate) fn structural_fingerprint(&self) -> u64 {
        let structure = format!(
            "aliases={:?};prefixes={:?};case={};global_schema={:?};schema={:?};branches={:?};completion={:?};shortcuts={:?}",
            self.aliases,
            self.prefixes,
            self.case_sensitive,
            self.global_schema,
            self.schema,
            self.branches,
            self.completion,
            self.shortcuts,
        );
        stable_hash(structure.as_bytes())
    }

    /// Canonical root names used to reject ambiguous registrations across
    /// overlapping Bot scopes. Prefix variants belong to one command schema,
    /// so two separately registered commands with the same logical name are
    /// considered a conflict even when their textual prefixes differ.
    pub(crate) fn registration_keys(&self) -> Vec<Arc<str>> {
        let mut keys = Vec::new();
        for name in std::iter::once(&self.name).chain(self.aliases.iter()) {
            let key: Arc<str> = Arc::from(normalize_native_name(name).to_ascii_lowercase());
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        // Hash collisions are improbable but must fail deterministically rather
        // than aliasing two command schemas in caches and native publication.
        keys.push(Arc::from(format!("\0command-id:{:016x}", self.id.0)));
        keys
    }

    /// Builds root command usage text from its active schema.
    #[must_use]
    pub fn usage(&self) -> String {
        let mut usage = self.display_name();
        if let Some(schema) = &self.schema {
            for argument in schema.arguments() {
                if argument.is_hidden() {
                    continue;
                }
                usage.push(' ');
                usage.push_str(&argument.usage_fragment());
            }
        }
        usage
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        let max_key = oxidebot_core::event::kernel::MAX_ROUTE_KEY_BYTES;
        if self.name.trim().is_empty() {
            return Err("command name cannot be empty".into());
        }
        if self.prefixes.is_empty() {
            return Err(format!("command `{}` has no prefixes", self.name));
        }

        let mut names = HashSet::new();
        for name in std::iter::once(&self.name).chain(self.aliases.iter()) {
            let canonical = name.split_whitespace().collect::<Vec<_>>().join(" ");
            if canonical.is_empty() || canonical != name.as_ref() {
                return Err(format!(
                    "command path `{name}` must use single spaces without leading or trailing whitespace"
                ));
            }
            if name.len() > max_key {
                return Err(format!("command path `{name}` exceeds {max_key} bytes"));
            }
            let root = name
                .split_whitespace()
                .next()
                .ok_or_else(|| format!("command `{}` has an empty route root", self.name))?;
            if root.len() > max_key {
                return Err(format!(
                    "command route root `{root}` exceeds {max_key} bytes"
                ));
            }
            let identity = if self.case_sensitive {
                name.to_string()
            } else {
                name.to_ascii_lowercase()
            };
            if !names.insert(identity) {
                return Err(format!(
                    "command `{}` contains a duplicate name or alias",
                    self.name
                ));
            }
        }

        let mut prefixes = HashSet::new();
        for prefix in &self.prefixes {
            if prefix.len() > max_key {
                return Err(format!(
                    "command `{}` contains an oversized prefix",
                    self.name
                ));
            }
            if prefix.chars().any(char::is_whitespace) {
                return Err(format!(
                    "command `{}` contains a whitespace prefix",
                    self.name
                ));
            }
            if !prefixes.insert(prefix.as_ref()) {
                return Err(format!(
                    "command `{}` contains a duplicate prefix",
                    self.name
                ));
            }
        }

        if self.shortcuts.len() > 256 {
            return Err(format!(
                "command `{}` contains more than 256 shortcuts",
                self.name
            ));
        }
        let mut shortcut_patterns = HashSet::new();
        for shortcut in &self.shortcuts {
            let identity = (shortcut.is_regex(), shortcut.pattern_text().to_owned());
            if !shortcut_patterns.insert(identity) {
                return Err(format!(
                    "command `{}` contains a duplicate shortcut `{}`",
                    self.name,
                    shortcut.pattern_text(),
                ));
            }
        }

        if let Some(completion) = &self.completion {
            if completion.timeout.is_zero() {
                return Err(format!(
                    "command `{}` has a zero completion timeout",
                    self.name
                ));
            }
            if completion.max_rounds == 0 || completion.max_attempts_per_field == 0 {
                return Err(format!(
                    "command `{}` has an invalid completion attempt limit",
                    self.name
                ));
            }
            if completion
                .cancel_words
                .iter()
                .any(|word| word.trim().is_empty())
            {
                return Err(format!(
                    "command `{}` has an empty completion cancel word",
                    self.name
                ));
            }
        }
        if let Some(schema) = &self.global_schema {
            schema.validate()?;
            if schema.arguments().iter().any(ArgumentSpec::is_positional) {
                return Err(format!(
                    "command `{}` global arguments must be named options or flags",
                    self.name
                ));
            }
        }
        if let Some(schema) = &self.schema {
            schema.validate()?;
        }
        if self.schema.is_some() && !self.branches.is_empty() {
            return Err(format!(
                "command `{}` cannot contain both root arguments and subcommands; move shared values into explicit branch options",
                self.name
            ));
        }
        let mut branch_names = HashSet::new();
        for branch in &self.branches {
            for name in std::iter::once(&branch.name).chain(branch.aliases.iter()) {
                let identity = if self.case_sensitive {
                    name.to_string()
                } else {
                    name.to_ascii_lowercase()
                };
                if !branch_names.insert(identity) {
                    return Err(format!(
                        "command `{}` contains duplicate subcommand name or alias `{name}`",
                        self.name
                    ));
                }
            }
            branch.validate(&format!("{} {}", self.name, branch.name))?;
            if let Some(global_schema) = &self.global_schema {
                validate_global_schema_for_branch(global_schema, branch, &self.name)?;
            }
        }

        let mut node_ids = HashMap::<CommandNodeId, String>::new();
        let mut field_ids = HashMap::<CommandFieldId, String>::new();
        if let Some(schema) = &self.global_schema {
            collect_schema_field_ids(schema, &format!("{}.__global", self.name), &mut field_ids)?;
        }
        if let Some(schema) = &self.schema {
            collect_schema_field_ids(schema, &self.name, &mut field_ids)?;
        }
        for branch in &self.branches {
            collect_branch_ids(
                branch,
                &format!("{} {}", self.name, branch.name),
                &mut node_ids,
                &mut field_ids,
            )?;
        }
        Ok(())
    }

    /// Returns exact pre-decode keys when this command can use the fast `/name`
    /// path. `None` means it needs the broad message candidate table.
    pub(crate) fn fast_route_keys(&self) -> Option<Vec<Arc<str>>> {
        if !self.case_sensitive
            || self.prefixes.len() != 1
            || self
                .prefixes
                .first()
                .is_none_or(|prefix| prefix.as_ref() != "/")
        {
            return None;
        }
        let mut keys = Vec::new();
        for name in std::iter::once(&self.name).chain(self.aliases.iter()) {
            let root = name.split_whitespace().next()?;
            if root.is_empty() {
                return None;
            }
            let root: Arc<str> = Arc::from(root);
            if !keys.contains(&root) {
                keys.push(root);
            }
        }
        Some(keys)
    }
}
