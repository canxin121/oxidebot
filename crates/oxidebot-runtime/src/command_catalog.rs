//! Lookup, discovery, help, and native-completion views for command trees.
//!
//! The command schema owns the immutable tree; this module provides the
//! read-only catalog operations used by help endpoints and adapters. Keeping
//! those queries separate prevents presentation and registry concerns from
//! leaking into parsing.

use super::{
    command_parse::edit_distance, cursor_prefix, normalize_native_name, same_word, ArgumentSpec,
    Command, CommandBranch, CommandFieldId, CommandOutput, CommandRenderer, CommandSchema,
    CompletionItem, CompletionKind, DefaultCommandRenderer, SourceSpan,
};
use crate::handler::RouteScope;
use oxidebot_core::{
    application::{CommandDefinition, Suggestion, SuggestionRequest},
    source::message::Message,
    BotIdentity,
};
use std::sync::Arc;

/// Immutable help/catalog view assembled from all registered commands.
#[derive(Clone, Debug, Default)]
pub struct CommandCatalog {
    commands: Arc<[Command]>,
    scopes: Arc<[RouteScope]>,
}

impl CommandCatalog {
    /// Creates a global-scope catalog from command definitions.
    #[must_use]
    pub fn new(commands: impl IntoIterator<Item = Command>) -> Self {
        let commands = commands.into_iter().collect::<Vec<_>>();
        let scopes = vec![RouteScope::global(); commands.len()];
        Self {
            commands: commands.into(),
            scopes: scopes.into(),
        }
    }

    pub(crate) fn scoped(commands: impl IntoIterator<Item = (Command, RouteScope)>) -> Self {
        let (commands, scopes): (Vec<_>, Vec<_>) = commands.into_iter().unzip();
        Self {
            commands: commands.into(),
            scopes: scopes.into(),
        }
    }

    pub(crate) fn for_identity(&self, identity: &BotIdentity) -> Self {
        Self::scoped(
            self.commands
                .iter()
                .cloned()
                .zip(self.scopes.iter().cloned())
                .filter(|(_, scope)| scope.matches(identity)),
        )
    }

    /// Returns commands in registration order.
    #[must_use]
    pub fn commands(&self) -> &[Command] {
        &self.commands
    }

    /// Finds a command by its portable native command ID.
    #[must_use]
    pub fn find_by_id(&self, id: &str) -> Option<&Command> {
        self.commands
            .iter()
            .find(|command| id == format!("oxidebot:{:016x}", command.id().0))
    }

    /// Finds a command by canonical name or alias.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<&Command> {
        let name = name.trim().trim_start_matches('/');
        self.commands.iter().find(|command| {
            command.name.eq_ignore_ascii_case(name)
                || normalize_native_name(&command.name).eq_ignore_ascii_case(name)
                || command.aliases.iter().any(|alias| {
                    alias.eq_ignore_ascii_case(name)
                        || normalize_native_name(alias).eq_ignore_ascii_case(name)
                })
        })
    }

    /// Returns a conservative suggestion for a mistyped prefixed root command.
    ///
    /// This only suggests; it never treats the input as a command or executes
    /// a handler. Applications opt into delivery with [`crate::Module::typo_assist`].
    #[must_use]
    pub fn suggest_root_command(&self, input: &str) -> Option<String> {
        let first = input.split_whitespace().next()?;
        let mut best = None::<(usize, String)>;
        for command in self.commands.iter().filter(|command| !command.hidden) {
            for prefix in &command.prefixes {
                let Some(root) = first.strip_prefix(prefix.as_ref()) else {
                    continue;
                };
                if root.is_empty() {
                    continue;
                }
                let root = root.split('@').next().unwrap_or(root);
                for candidate in std::iter::once(&command.name).chain(command.aliases.iter()) {
                    let Some(candidate_root) = candidate.split_whitespace().next() else {
                        continue;
                    };
                    if same_word(root, candidate_root, command.case_sensitive) {
                        return None;
                    }
                    let compared_root = if command.case_sensitive {
                        root.to_owned()
                    } else {
                        root.to_ascii_lowercase()
                    };
                    let compared_candidate = if command.case_sensitive {
                        candidate_root.to_owned()
                    } else {
                        candidate_root.to_ascii_lowercase()
                    };
                    let distance = edit_distance(&compared_root, &compared_candidate);
                    let limit = if candidate_root.chars().count() <= 4 {
                        1
                    } else {
                        2
                    };
                    if distance <= limit
                        && best
                            .as_ref()
                            .is_none_or(|(best_distance, _)| distance < *best_distance)
                    {
                        best = Some((distance, command.display_name()));
                    }
                }
            }
        }
        best.map(|(_, suggestion)| suggestion)
    }

    /// Returns a catalog filtered to commands currently enabled in `registry`.
    #[must_use]
    pub fn enabled(&self, registry: &crate::CommandRegistry) -> Self {
        let mut commands = Vec::new();
        let mut scopes = Vec::new();
        for (command, scope) in self.commands.iter().zip(self.scopes.iter()) {
            if registry.is_enabled(command.id()) {
                commands.push(command.clone());
                scopes.push(scope.clone());
            }
        }
        Self {
            commands: commands.into(),
            scopes: scopes.into(),
        }
    }

    /// Converts discoverable commands to portable native definitions.
    #[must_use]
    pub fn definitions(&self) -> Vec<CommandDefinition> {
        self.commands
            .iter()
            .filter(|command| !command.hidden)
            .map(Command::definition)
            .collect()
    }

    /// Builds catalog, help, or not-found output for a query.
    #[must_use]
    pub fn output(&self, query: Option<&str>) -> CommandOutput {
        let Some(query) = query.filter(|value| !value.trim().is_empty()) else {
            return CommandOutput::Catalog {
                commands: Arc::clone(&self.commands),
            };
        };
        let normalized = query.trim().trim_start_matches('/');
        if let Some(command) = self.find(normalized) {
            return CommandOutput::Help {
                command: command.clone(),
                branch_names: Arc::from([]),
            };
        }
        let tokens = normalized.split_whitespace().collect::<Vec<_>>();
        let Some((command, consumed)) = self.commands.iter().find_map(|command| {
            command_name_consumed(command, &tokens).map(|consumed| (command, consumed))
        }) else {
            return CommandOutput::NotFound {
                query: Arc::from(query),
                commands: Arc::clone(&self.commands),
            };
        };
        let mut children = command.branches.as_slice();
        let mut branches = Vec::new();
        for token in tokens.iter().skip(consumed) {
            let Some(branch) = children
                .iter()
                .find(|branch| branch.matches(token, command.case_sensitive))
            else {
                return CommandOutput::NotFound {
                    query: Arc::from(query),
                    commands: Arc::clone(&self.commands),
                };
            };
            branches.push(Arc::clone(&branch.name));
            children = branch.children.as_slice();
        }
        CommandOutput::Help {
            command: command.clone(),
            branch_names: branches.into(),
        }
    }

    /// Renders catalog output to a canonical message in `locale`.
    #[must_use]
    pub fn render_message(&self, query: Option<&str>, locale: Option<&str>) -> Message {
        DefaultCommandRenderer.render(&self.output(query), locale)
    }

    /// Renders the catalog as plain text.
    /// Suggests command, branch, option, and choice completions.
    #[must_use]
    pub fn render_text(&self, query: Option<&str>) -> String {
        self.render_message(query, None).get_raw_text()
    }

    /// Produces portable native suggestions for an adapter request.
    #[must_use]
    pub fn suggest(&self, input: &str, cursor: usize, locale: Option<&str>) -> Vec<CompletionItem> {
        let (prefix, replace) = cursor_prefix(input, cursor);
        let first = prefix.split_whitespace().next().unwrap_or(prefix);
        let has_complete_root = self.commands.iter().any(|command| {
            command.prefixes.iter().any(|command_prefix| {
                first
                    .strip_prefix(command_prefix.as_ref())
                    .is_some_and(|root| {
                        std::iter::once(&command.name)
                            .chain(command.aliases.iter())
                            .any(|name| {
                                name.split_whitespace().next().is_some_and(|expected| {
                                    same_word(root, expected, command.case_sensitive)
                                })
                            })
                    })
            })
        });
        if !has_complete_root {
            let mut output = Vec::new();
            for command in self.commands.iter().filter(|command| !command.hidden) {
                let display = command.display_name();
                if !display.starts_with(first) {
                    continue;
                }
                let mut item = CompletionItem::new(display, CompletionKind::Command, replace);
                let description = command.localized_description(locale);
                if !description.is_empty() {
                    item.description = Some(Message::text(description));
                }
                output.push(item);
            }
            return output;
        }
        self.commands
            .iter()
            .flat_map(|command| command.suggest(input, cursor, locale))
            .collect()
    }

    /// Produces portable native suggestions for an adapter completion request.
    #[must_use]
    pub fn native_suggestions(&self, request: &SuggestionRequest) -> Vec<Suggestion> {
        let limit = request.limit.unwrap_or(25).min(100) as usize;
        let locale = request.locale.as_deref();
        let command = request
            .command_id
            .as_deref()
            .and_then(|id| self.find_by_id(id))
            .or_else(|| {
                request
                    .command_name
                    .as_deref()
                    .and_then(|name| self.find(name))
            });
        let mut items = if let Some(field) = request.field_id.as_deref() {
            self.find_argument_for_request(command, &request.command_path, field)
                .map_or_else(Vec::new, |argument| {
                    argument
                        .choices
                        .iter()
                        .filter(|choice| {
                            request.query.is_empty()
                                || choice
                                    .value
                                    .to_ascii_lowercase()
                                    .contains(&request.query.to_ascii_lowercase())
                                || choice
                                    .name
                                    .resolve(locale)
                                    .to_ascii_lowercase()
                                    .contains(&request.query.to_ascii_lowercase())
                        })
                        .map(|choice| {
                            let mut item = CompletionItem::new(
                                choice.value.to_string(),
                                CompletionKind::Choice,
                                SourceSpan::default(),
                            )
                            .field(argument.id);
                            item.display = Message::text(choice.name.resolve(locale));
                            item
                        })
                        .collect()
                })
        } else if let Some(command) = command {
            command.suggest(&request.query, request.query.len(), locale)
        } else {
            self.suggest(&request.query, request.query.len(), locale)
        };
        items.truncate(limit);
        items
            .into_iter()
            .enumerate()
            .map(|(index, item)| item.into_suggestion(index))
            .collect()
    }

    fn find_argument_for_request<'a>(
        &'a self,
        command: Option<&'a Command>,
        path: &[String],
        field: &str,
    ) -> Option<&'a ArgumentSpec> {
        let field_id = field
            .trim_start_matches("oxidebot:")
            .parse::<u32>()
            .ok()
            .map(CommandFieldId);
        if let Some(command) = command {
            let mut schema = command.schema.as_ref();
            let mut children = command.branches.as_slice();
            for component in path {
                let branch = children
                    .iter()
                    .find(|branch| branch.matches(component, command.case_sensitive))?;
                schema = branch.schema.as_ref().or(schema);
                children = branch.children.as_slice();
            }
            if let Some(argument) = find_argument_in_schema(schema, field, field_id) {
                return Some(argument);
            }
            return find_argument_in_branches(children, field, field_id);
        }
        for command in self.commands.iter() {
            if let Some(argument) =
                find_argument_in_schema(command.schema.as_ref(), field, field_id)
            {
                return Some(argument);
            }
            if let Some(argument) = find_argument_in_branches(&command.branches, field, field_id) {
                return Some(argument);
            }
        }
        None
    }
}

fn command_name_consumed(command: &Command, tokens: &[&str]) -> Option<usize> {
    std::iter::once(&command.name)
        .chain(command.aliases.iter())
        .filter_map(|name| {
            let path = name.split_whitespace().collect::<Vec<_>>();
            (path.len() <= tokens.len()
                && path
                    .iter()
                    .zip(tokens)
                    .all(|(expected, actual)| same_word(actual, expected, command.case_sensitive)))
            .then_some(path.len())
        })
        .max()
}

fn find_argument_in_schema<'a>(
    schema: Option<&'a CommandSchema>,
    field: &str,
    field_id: Option<CommandFieldId>,
) -> Option<&'a ArgumentSpec> {
    schema.and_then(|schema| {
        schema.arguments.iter().find(|argument| {
            argument.name.as_ref() == field
                || argument.long.as_deref() == Some(field)
                || field_id == Some(argument.id)
                || field == format!("oxidebot:{}", argument.id.0)
        })
    })
}

fn find_argument_in_branches<'a>(
    branches: &'a [CommandBranch],
    field: &str,
    field_id: Option<CommandFieldId>,
) -> Option<&'a ArgumentSpec> {
    for branch in branches {
        if let Some(argument) = find_argument_in_schema(branch.schema.as_ref(), field, field_id) {
            return Some(argument);
        }
        if let Some(argument) = find_argument_in_branches(&branch.children, field, field_id) {
            return Some(argument);
        }
    }
    None
}
