//! Text and native command-invocation matching.

use super::*;

impl Command {
    /// Matches one canonical message without runtime-added shortcuts. This is
    /// useful for parser tools and focused command tests that do not need to
    /// start the full runtime.
    pub fn match_message(
        &self,
        message: &Message,
    ) -> Result<Option<CommandMatch>, CommandParseError> {
        self.match_message_with_shortcuts(message, &[])
    }

    /// Matches and parses one canonical message, returning a structured
    /// command result or a precise parse error.
    pub fn parse_message(&self, message: &Message) -> Result<CommandMatch, CommandParseError> {
        let matched =
            self.match_message(message)?
                .ok_or_else(|| CommandParseError::NotMatched {
                    command: Arc::clone(&self.name),
                })?;
        let parsed = matched.parse_active()?;
        Ok(matched.with_parsed(parsed))
    }

    pub(crate) fn match_message_with_shortcuts(
        &self,
        message: &Message,
        runtime_shortcuts: &[Shortcut],
    ) -> Result<Option<CommandMatch>, CommandParseError> {
        let tokens = tokenize_segments(&message.segments)?;
        if let Some(result) = self.match_tokens(tokens) {
            return Ok(Some(result));
        }
        let input = message.extract_plain_text();
        for shortcut in self.shortcuts.iter().chain(runtime_shortcuts) {
            let Some(rewritten) = shortcut.rewrite(&input) else {
                continue;
            };
            let tokens = tokenize_text(&rewritten)?
                .into_iter()
                .map(CommandValue::Text)
                .collect();
            if let Some(result) = self.match_tokens(tokens) {
                return Ok(Some(result));
            }
        }
        Ok(None)
    }

    pub(in crate::command) fn match_tokens(
        &self,
        tokens: Vec<CommandValue>,
    ) -> Option<CommandMatch> {
        let first = tokens.first()?.as_text()?;
        let mut matched: Option<(usize, Arc<str>, Arc<str>)> = None;
        let names = std::iter::once(&self.name).chain(self.aliases.iter());

        for prefix in &self.prefixes {
            let Some(root) = first.strip_prefix(prefix.as_ref()) else {
                continue;
            };
            let root = root.split('@').next().unwrap_or(root);
            for name in names.clone() {
                let path = name.split_whitespace().collect::<Vec<_>>();
                let Some(expected_root) = path.first() else {
                    continue;
                };
                if !same_word(root, expected_root, self.case_sensitive) {
                    continue;
                }
                let mut consumed = 1;
                let mut complete = true;
                for expected in path.iter().skip(1) {
                    let Some(actual) = tokens.get(consumed).and_then(CommandValue::as_text) else {
                        complete = false;
                        break;
                    };
                    if !same_word(actual, expected, self.case_sensitive) {
                        complete = false;
                        break;
                    }
                    consumed += 1;
                }
                if complete && matched.as_ref().is_none_or(|(best, _, _)| consumed > *best) {
                    matched = Some((consumed, Arc::clone(name), Arc::clone(prefix)));
                }
            }
        }

        let (mut consumed, invoked_as, prefix) = matched?;
        let mut schema = self.global_schema.clone().unwrap_or_default();
        if let Some(root_schema) = &self.schema {
            schema = schema.merged(root_schema);
        }
        let mut branch_path = Vec::new();
        let mut branch_names = Vec::new();
        let mut completion = self.completion.clone();
        let mut children = self.branches.as_slice();
        let mut values = Vec::new();

        while let Some(actual) = tokens.get(consumed).and_then(CommandValue::as_text) {
            if let Some(branch) = children
                .iter()
                .find(|branch| branch.matches(actual, self.case_sensitive))
            {
                consumed += 1;
                branch_path.push(branch.id);
                branch_names.push(Arc::clone(&branch.name));
                if let Some(branch_schema) = &branch.schema {
                    schema = schema.merged(branch_schema);
                }
                if let Some(branch_completion) = &branch.completion {
                    completion = Some(branch_completion.clone());
                }
                children = branch.children.as_slice();
                continue;
            }
            let Some(global_schema) = &self.global_schema else {
                break;
            };
            let Some(width) = global_option_width(global_schema, &tokens, consumed) else {
                break;
            };
            let end = consumed.saturating_add(width).min(tokens.len());
            values.extend(tokens[consumed..end].iter().cloned());
            consumed = end;
        }
        values.extend(tokens.into_iter().skip(consumed));

        Some(CommandMatch {
            command: self.clone(),
            invoked_as,
            prefix,
            branch_path: branch_path.into(),
            branch_names: branch_names.into(),
            values: values.into(),
            schema: Arc::new(schema),
            parsed: None,
            completion,
            source: CommandSource::Text,
            locale: None,
        })
    }

    pub(crate) fn match_invocation(
        &self,
        invocation: &CommandInvocation,
    ) -> Result<Option<CommandMatch>, CommandParseError> {
        let native_name = normalize_native_name(&invocation.name);
        let matched_name = std::iter::once(&self.name)
            .chain(self.aliases.iter())
            .find(|name| {
                same_word(
                    &native_name,
                    &normalize_native_name(name),
                    self.case_sensitive,
                )
            });
        let Some(invoked_as) = matched_name else {
            return Ok(None);
        };

        let mut path = invocation.path.as_slice();
        if path.first().is_some_and(|first| {
            same_word(
                &normalize_native_name(first),
                &normalize_native_name(invoked_as),
                self.case_sensitive,
            )
        }) {
            path = &path[1..];
        }

        let mut schema = self.global_schema.clone().unwrap_or_default();
        if let Some(root_schema) = &self.schema {
            schema = schema.merged(root_schema);
        }
        let mut branch_path = Vec::new();
        let mut branch_names = Vec::new();
        let mut completion = self.completion.clone();
        let mut children = self.branches.as_slice();
        for component in path {
            let Some(branch) = children
                .iter()
                .find(|branch| branch.matches(component, self.case_sensitive))
            else {
                return Err(unknown_subcommand(children, component, self.case_sensitive));
            };
            branch_path.push(branch.id);
            branch_names.push(Arc::clone(&branch.name));
            if let Some(branch_schema) = &branch.schema {
                schema = schema.merged(branch_schema);
            }
            if let Some(branch_completion) = &branch.completion {
                completion = Some(branch_completion.clone());
            }
            children = branch.children.as_slice();
        }

        let parsed = parse_native_arguments(&schema, invocation)?;
        let values = invocation
            .options
            .values()
            .flatten()
            .cloned()
            .map(CommandValue::Form)
            .collect::<Vec<_>>();
        Ok(Some(CommandMatch {
            command: self.clone(),
            invoked_as: Arc::clone(invoked_as),
            prefix: Arc::from(""),
            branch_path: branch_path.into(),
            branch_names: branch_names.into(),
            values: values.into(),
            schema: Arc::new(schema),
            parsed: Some(Arc::new(parsed)),
            completion,
            source: CommandSource::Native(Arc::new(invocation.clone())),
            locale: invocation.locale.as_deref().map(Arc::from),
        }))
    }
}
