//! Rendering boundary for structured command output.
//!
//! Parsing and catalog lookup create [`CommandOutput`] values. This module is
//! deliberately the only place that turns those values into the canonical
//! [`Message`] IR, keeping adapters free to replace presentation without
//! depending on parser internals.

use super::{Command, CommandBranch, CommandParseError, CompletionItem};
use oxidebot_core::{source::message::Message, TemplateValue, TranslationCatalog};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

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

fn is_chinese(locale: Option<&str>) -> bool {
    locale.is_none_or(|locale| locale.starts_with("zh"))
}

fn render_catalog_text(commands: &[Command], locale: Option<&str>) -> String {
    let visible = commands.iter().filter(|command| !command.hidden);
    let mut groups = HashMap::<Option<Arc<str>>, Vec<&Command>>::new();
    for command in visible {
        groups
            .entry(command.category.clone())
            .or_default()
            .push(command);
    }
    let mut categories = groups.into_iter().collect::<Vec<_>>();
    categories.sort_by(|(left, _), (right, _)| left.cmp(right));
    let mut output = String::from(if is_chinese(locale) {
        "可用命令\n"
    } else {
        "Available commands\n"
    });
    for (category, mut commands) in categories {
        commands.sort_by(|left, right| left.name.cmp(&right.name));
        if let Some(category) = category {
            output.push('\n');
            output.push_str(&category);
            output.push('\n');
        }
        for command in commands {
            output.push_str("  ");
            output.push_str(&command.display_name());
            let description = command.localized_description(locale);
            if !description.is_empty() {
                output.push_str(" — ");
                output.push_str(description);
            }
            if !command.branches.is_empty() {
                output.push_str(if is_chinese(locale) {
                    "（含子命令）"
                } else {
                    " (subcommands)"
                });
            }
            output.push('\n');
        }
    }
    output.push_str(if is_chinese(locale) {
        "\n使用 /help <命令> 查看详细说明。"
    } else {
        "\nUse /help <command> for detailed help."
    });
    output
}

fn resolve_branch<'a>(command: &'a Command, path: &[Arc<str>]) -> Option<&'a CommandBranch> {
    let mut children = command.branches.as_slice();
    let mut selected = None;
    for name in path {
        let branch = children.iter().find(|branch| branch.name == *name)?;
        selected = Some(branch);
        children = branch.children.as_slice();
    }
    selected
}

fn render_command_help(
    command: &Command,
    branch_names: &[Arc<str>],
    locale: Option<&str>,
) -> String {
    let usage = command.usage_for(branch_names);
    let selected = resolve_branch(command, branch_names);
    let description = selected.map_or_else(
        || command.localized_description(locale),
        |branch| branch.localized_description(locale),
    );
    let schema = command.schema_for_branch_names(branch_names);
    let children = selected.map_or(command.branches.as_slice(), |branch| {
        branch.children.as_slice()
    });
    let title = if branch_names.is_empty() {
        command.display_name()
    } else {
        format!(
            "{} {}",
            command.display_name(),
            branch_names
                .iter()
                .map(AsRef::as_ref)
                .collect::<Vec<_>>()
                .join(" ")
        )
    };
    let usage_label = if is_chinese(locale) {
        "用法"
    } else {
        "Usage"
    };
    let mut output = format!("{title}\n\n{usage_label}：{usage}");
    if !description.is_empty() {
        output.push_str("\n\n");
        output.push_str(description);
    }
    if branch_names.is_empty() && !command.aliases.is_empty() {
        output.push_str(if is_chinese(locale) {
            "\n\n别名："
        } else {
            "\n\nAliases: "
        });
        output.push_str(
            &command
                .aliases
                .iter()
                .map(AsRef::as_ref)
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    if !children.is_empty() {
        output.push_str(if is_chinese(locale) {
            "\n\n子命令：\n"
        } else {
            "\n\nSubcommands:\n"
        });
        for branch in children.iter().filter(|branch| !branch.hidden) {
            output.push_str("  ");
            output.push_str(&branch.name);
            let branch_description = branch.localized_description(locale);
            if !branch_description.is_empty() {
                output.push_str(" — ");
                output.push_str(branch_description);
            }
            output.push('\n');
        }
    }
    if !schema.arguments.is_empty() {
        output.push_str(if is_chinese(locale) {
            "\n参数：\n"
        } else {
            "\nArguments:\n"
        });
        let mut previous_heading = None::<String>;
        for argument in schema.arguments.iter().filter(|argument| !argument.hidden) {
            let heading = argument.heading_text(locale).unwrap_or_default();
            if !heading.is_empty() && previous_heading.as_deref() != Some(heading) {
                output.push_str("\n  ");
                output.push_str(heading);
                output.push('\n');
                previous_heading = Some(heading.to_owned());
            }
            output.push_str("  ");
            output.push_str(&argument.usage_fragment());
            let help = argument.help_text(locale);
            if !help.is_empty() {
                output.push_str(" — ");
                output.push_str(help);
            }
            if let Some(default) = &argument.default {
                if is_chinese(locale) {
                    output.push_str(&format!("（默认：{default}）"));
                } else {
                    output.push_str(&format!(" (default: {default})"));
                }
            }
            if !argument.choices.is_empty() {
                output.push_str(if is_chinese(locale) {
                    "；可选："
                } else {
                    "; choices: "
                });
                output.push_str(
                    &argument
                        .choices
                        .iter()
                        .map(|choice| choice.value.as_ref())
                        .collect::<Vec<_>>()
                        .join(", "),
                );
            }
            output.push('\n');
        }
    }
    if !schema.groups.is_empty() {
        output.push_str(if is_chinese(locale) {
            "\n参数组：\n"
        } else {
            "\nArgument groups:\n"
        });
        for group in &schema.groups {
            output.push_str("  ");
            output.push_str(&group.name);
            output.push_str(" — ");
            let relation = match (group.required, group.multiple) {
                (true, false) => {
                    if is_chinese(locale) {
                        "必须且只能提供其中一个"
                    } else {
                        "exactly one is required"
                    }
                }
                (true, true) => {
                    if is_chinese(locale) {
                        "至少提供其中一个"
                    } else {
                        "at least one is required"
                    }
                }
                (false, false) => {
                    if is_chinese(locale) {
                        "最多提供其中一个"
                    } else {
                        "at most one may be supplied"
                    }
                }
                (false, true) => {
                    if is_chinese(locale) {
                        "可组合提供"
                    } else {
                        "may be combined"
                    }
                }
            };
            output.push_str(relation);
            output.push('：');
            output.push_str(
                &group
                    .arguments
                    .iter()
                    .map(AsRef::as_ref)
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            output.push('\n');
        }
    }
    if branch_names.is_empty() && !command.examples.is_empty() {
        output.push_str(if is_chinese(locale) {
            "\n示例：\n"
        } else {
            "\nExamples:\n"
        });
        for example in &command.examples {
            output.push_str("  ");
            output.push_str(example);
            output.push('\n');
        }
    }
    output
}
