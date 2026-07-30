//! Rendering boundary for structured command output.
//!
//! Parsing and catalog lookup create [`CommandOutput`] values. This module is
//! deliberately the only place that turns those values into the canonical
//! [`Message`] IR, keeping adapters free to replace presentation without
//! depending on parser internals.

use super::{
    is_chinese, render_catalog_text, render_command_help, Command, CommandParseError,
    CompletionItem,
};
use oxidebot_core::{source::message::Message, TemplateValue, TranslationCatalog};
use std::{collections::BTreeMap, sync::Arc};

/// Renderer-neutral command output. Help, diagnostics, and completion are kept
/// structured until the last step so they can become plain text, rich layout,
/// buttons, or platform-native UI without changing the parser.
// `Message` remains inline to preserve the ergonomic public output API.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum CommandOutput {
    /// Catalog of discoverable commands.
    Catalog {
        /// Commands included in the catalog.
        commands: Arc<[Command]>,
    },
    /// Help for one command and optional branch path.
    Help {
        /// Command being described.
        command: Command,
        /// Selected branch names beneath the command root.
        branch_names: Arc<[Arc<str>]>,
    },
    /// No command or branch matched the requested query.
    NotFound {
        /// Original user query.
        query: Arc<str>,
        /// Commands searched when rendering alternatives.
        commands: Arc<[Command]>,
    },
    /// Parsing failed for a command invocation.
    ParseError {
        /// Command whose invocation failed.
        command: Command,
        /// Selected branch names at the failure point.
        branch_names: Arc<[Arc<str>]>,
        /// Structured parse failure.
        error: CommandParseError,
    },
    /// Completion items for a partially entered command.
    Suggestions {
        /// Candidate completion items.
        items: Arc<[CompletionItem]>,
    },
    /// Prebuilt canonical message output.
    Message(Message),
}

/// Converts structured command output into the canonical message IR.
pub trait CommandRenderer: Send + Sync + 'static {
    /// Renders structured output for an optional locale.
    fn render(&self, output: &CommandOutput, locale: Option<&str>) -> Message;
}

impl<F> CommandRenderer for F
where
    F: Fn(&CommandOutput, Option<&str>) -> Message + Send + Sync + 'static,
{
    fn render(&self, output: &CommandOutput, locale: Option<&str>) -> Message {
        (self)(output, locale)
    }
}

/// Built-in renderer for catalog, help, diagnostics, and parse-error output.
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultCommandRenderer;

impl CommandRenderer for DefaultCommandRenderer {
    fn render(&self, output: &CommandOutput, locale: Option<&str>) -> Message {
        match output {
            CommandOutput::Catalog { commands } => {
                Message::text(render_catalog_text(commands, locale))
            }
            CommandOutput::Help {
                command,
                branch_names,
            } => Message::text(render_command_help(command, branch_names, locale)),
            CommandOutput::NotFound { query, commands } => {
                let text = if is_chinese(locale) {
                    format!(
                        "没有找到命令 `{query}`。\n\n{}",
                        render_catalog_text(commands, locale)
                    )
                } else {
                    format!(
                        "Command `{query}` was not found.\n\n{}",
                        render_catalog_text(commands, locale)
                    )
                };
                Message::text(text)
            }
            CommandOutput::ParseError {
                command,
                branch_names,
                error,
            } => {
                let usage = command.usage_for(branch_names);
                let label = if is_chinese(locale) {
                    "用法"
                } else {
                    "Usage"
                };
                Message::text(format!(
                    "{}\n\n{label}：{usage}",
                    error.localized_message(locale)
                ))
            }
            CommandOutput::Suggestions { items } => {
                let heading = if is_chinese(locale) {
                    "可继续输入："
                } else {
                    "Suggestions:"
                };
                let mut text = String::from(heading);
                for item in items.iter() {
                    text.push_str("\n  ");
                    text.push_str(&item.display.get_raw_text());
                    if let Some(description) = &item.description {
                        let description = description.get_raw_text();
                        if !description.is_empty() {
                            text.push_str(" — ");
                            text.push_str(&description);
                        }
                    }
                }
                Message::text(text)
            }
            CommandOutput::Message(message) => message.clone(),
        }
    }
}

/// Resource-backed renderer that lets applications wrap or replace the
/// framework's structured command output with normal message templates.
/// Missing or invalid resource entries fall back to [`DefaultCommandRenderer`]
/// instead of making help/error delivery fallible.
#[derive(Clone, Debug)]
pub struct CatalogCommandRenderer {
    catalog: TranslationCatalog,
    prefix: Arc<str>,
}

impl CatalogCommandRenderer {
    /// Creates a resource-backed renderer with the default command key prefix.
    #[must_use]
    pub fn new(catalog: TranslationCatalog) -> Self {
        Self {
            catalog,
            prefix: Arc::from("oxidebot.command"),
        }
    }

    /// Replaces the translation-key prefix used by this renderer.
    #[must_use]
    pub fn prefix(mut self, value: impl Into<Arc<str>>) -> Self {
        self.prefix = value.into();
        self
    }

    fn key(&self, suffix: &str) -> String {
        format!("{}.{}", self.prefix.trim_end_matches('.'), suffix)
    }
}

impl CommandRenderer for CatalogCommandRenderer {
    fn render(&self, output: &CommandOutput, locale: Option<&str>) -> Message {
        if let CommandOutput::Message(message) = output {
            return message.clone();
        }
        let fallback = DefaultCommandRenderer.render(output, locale);
        let (suffix, mut values) = match output {
            CommandOutput::Catalog { .. } => ("catalog", BTreeMap::new()),
            CommandOutput::Help {
                command,
                branch_names,
            } => {
                let mut values = BTreeMap::new();
                values.insert(Arc::from("command"), TemplateValue::text(command.name()));
                values.insert(
                    Arc::from("usage"),
                    TemplateValue::text(command.usage_for(branch_names)),
                );
                ("help", values)
            }
            CommandOutput::NotFound { query, .. } => {
                let mut values = BTreeMap::new();
                values.insert(Arc::from("query"), TemplateValue::text(query.as_ref()));
                ("not_found", values)
            }
            CommandOutput::ParseError {
                command,
                branch_names,
                error,
            } => {
                let mut values = BTreeMap::new();
                values.insert(Arc::from("command"), TemplateValue::text(command.name()));
                values.insert(
                    Arc::from("usage"),
                    TemplateValue::text(command.usage_for(branch_names)),
                );
                values.insert(
                    Arc::from("error"),
                    TemplateValue::text(error.localized_message(locale)),
                );
                ("parse_error", values)
            }
            CommandOutput::Suggestions { .. } => ("suggestions", BTreeMap::new()),
            CommandOutput::Message(_) => unreachable!(),
        };
        values.insert(Arc::from("body"), TemplateValue::from(fallback.clone()));
        self.catalog
            .render(locale, &self.key(suffix), &values)
            .unwrap_or(fallback)
    }
}
