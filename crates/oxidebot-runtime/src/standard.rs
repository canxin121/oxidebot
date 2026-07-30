//! Optional standard features built from ordinary [`crate::Module`] values.

use crate::{
    command, Args, ArgumentSpec, CommandArgs, CommandParseError, CommandRegistry, CommandSchema,
    Context, EventLocaleResolver, Guard, HandlerError, HandlerResult, LocaleResolver, Module,
    ParsedArguments, Sender, Shortcut, State,
};
use async_trait::async_trait;
use oxidebot_core::Message;
use std::sync::Arc;

/// Configures the built-in command-management, shortcut-management, and
/// diagnostics features as one ordinary [`Module`].
#[derive(Clone, Debug)]
pub struct AdminTools {
    prefix: Arc<str>,
    command_management: bool,
    shortcut_management: bool,
    diagnostics: bool,
}

impl Default for AdminTools {
    fn default() -> Self {
        Self::new()
    }
}

impl AdminTools {
    /// Creates an enabled standard administrative-tool set with the `oxidebot` prefix.
    #[must_use]
    pub fn new() -> Self {
        Self {
            prefix: Arc::from("oxidebot"),
            command_management: true,
            shortcut_management: true,
            diagnostics: true,
        }
    }

    /// Sets the command-word prefix. An empty prefix exposes leaf names
    /// directly. The default creates `/oxidebot commands`,
    /// `/oxidebot shortcut`, and `/oxidebot diagnostics`.
    #[must_use]
    pub fn prefix(mut self, value: impl Into<Arc<str>>) -> Self {
        self.prefix = value.into();
        self
    }

    /// Enables or disables the command-management submodule.
    #[must_use]
    pub const fn command_management(mut self, enabled: bool) -> Self {
        self.command_management = enabled;
        self
    }

    /// Enables or disables the shortcut-management submodule.
    #[must_use]
    pub const fn shortcut_management(mut self, enabled: bool) -> Self {
        self.shortcut_management = enabled;
        self
    }

    /// Enables or disables the diagnostics submodule.
    #[must_use]
    pub const fn diagnostics(mut self, enabled: bool) -> Self {
        self.diagnostics = enabled;
        self
    }

    /// Builds the configured administrative tools as an ordinary module.
    #[must_use]
    pub fn module<S>(&self) -> Module<S>
    where
        S: Send + Sync + 'static,
    {
        let mut module = Module::new();
        if self.command_management {
            module = module.include(command_admin_module_named(self.name("commands")));
        }
        if self.shortcut_management {
            module = module.include(shortcut_admin_module_named(self.name("shortcut")));
        }
        if self.diagnostics {
            module = module.include(diagnostics_module_named(self.name("diagnostics")));
        }
        module
    }

    /// Builds the configured administrative tools behind an application guard.
    #[must_use]
    pub fn protected<S, G>(&self, guard: G) -> Module<S>
    where
        S: Send + Sync + 'static,
        G: Guard<S>,
    {
        self.module::<S>().guard(guard)
    }

    fn name(&self, leaf: &str) -> String {
        let prefix = self.prefix.trim();
        if prefix.is_empty() {
            leaf.to_owned()
        } else {
            format!("{prefix} {leaf}")
        }
    }
}

/// Parsed arguments accepted by the standard echo command.
#[derive(Clone, Debug)]
pub struct EchoArguments {
    /// Remaining command words to return unchanged.
    pub content: Vec<String>,
}

impl CommandArgs for EchoArguments {
    fn schema() -> CommandSchema {
        CommandSchema::new().argument(
            ArgumentSpec::new("content")
                .rest(true)
                .multiple(true)
                .required(false)
                .help("要原样返回的内容"),
        )
    }

    fn from_arguments(arguments: &ParsedArguments) -> Result<Self, CommandParseError> {
        Ok(Self {
            content: arguments.many("content")?,
        })
    }
}

async fn echo_handler(Args(arguments): Args<EchoArguments>) -> Message {
    Message::text(arguments.content.join(" "))
}

/// Returns a module providing the `/echo` command.
#[must_use]
pub fn echo_module<S>() -> Module<S>
where
    S: Send + Sync + 'static,
{
    Module::new().command(
        EchoArguments::command("echo")
            .description("原样返回文本")
            .description_translation("en-US", "Echo text"),
        echo_handler,
    )
}

/// Application-state persistence required by the standard language command.
#[async_trait]
pub trait LocaleStorage: Send + Sync + 'static {
    /// Persists `locale` as the preferred locale for `user_id`.
    async fn set_locale(&self, user_id: &oxidebot_core::UserId, locale: &str) -> HandlerResult<()>;
    /// Loads the preferred locale for `user_id`, if one has been stored.
    async fn get_locale(&self, user_id: &oxidebot_core::UserId) -> HandlerResult<Option<Arc<str>>>;
}

/// Locale resolver that first consults [`LocaleStorage`] in application state.
#[derive(Clone, Copy, Debug, Default)]
pub struct StoredLocaleResolver;

#[async_trait]
impl<S> LocaleResolver<S> for StoredLocaleResolver
where
    S: LocaleStorage,
{
    async fn resolve(&self, context: &Context<S>) -> Option<Arc<str>> {
        let user_id = match context.event() {
            oxidebot_core::Event::Message(event) => Some(&event.sender.id),
            oxidebot_core::Event::Interaction(event) => Some(&event.user.id),
            _ => None,
        };
        if let Some(locale) = match user_id {
            Some(user_id) => context.state().get_locale(user_id).await.ok().flatten(),
            None => None,
        } {
            return Some(locale);
        }
        EventLocaleResolver.resolve(context).await
    }
}

/// Parsed arguments accepted by the standard language command.
#[derive(Clone, Debug)]
pub struct LanguageArguments {
    /// Optional locale to persist; omitted to display the current locale.
    pub locale: Option<String>,
}

impl CommandArgs for LanguageArguments {
    fn schema() -> CommandSchema {
        CommandSchema::new().argument(
            ArgumentSpec::new("locale")
                .required(false)
                .help("语言标签，例如 zh-CN 或 en-US"),
        )
    }

    fn from_arguments(arguments: &ParsedArguments) -> Result<Self, CommandParseError> {
        Ok(Self {
            locale: arguments.optional("locale")?,
        })
    }
}

async fn language_handler<S>(
    Args(arguments): Args<LanguageArguments>,
    Sender(user): Sender,
    State(state): State<S>,
) -> HandlerResult<String>
where
    S: LocaleStorage,
{
    if let Some(locale) = arguments.locale {
        let locale = locale.trim().replace('_', "-");
        if locale.is_empty()
            || locale.len() > 64
            || !locale
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '-')
        {
            return Err(HandlerError::user(
                "语言标签必须是类似 zh-CN 或 en-US 的短标签",
            ));
        }
        state.set_locale(&user.id, &locale).await?;
        return Ok(format!("语言已切换为 {locale}"));
    }
    Ok(state.get_locale(&user.id).await?.map_or_else(
        || "尚未设置个人语言".to_owned(),
        |locale| format!("当前语言：{locale}"),
    ))
}

/// Returns a module providing the `/language` and `/lang` commands.
#[must_use]
pub fn language_module<S>() -> Module<S>
where
    S: LocaleStorage,
{
    Module::new().command(
        LanguageArguments::command("language")
            .alias("lang")
            .alias("语言")
            .description("查看或切换个人语言")
            .description_translation("en-US", "Show or change your language"),
        language_handler::<S>,
    )
}

/// Parsed arguments accepted by the standard command-management command.
#[derive(Clone, Debug)]
pub struct CommandAdminArguments {
    /// Requested operation: `list`, `enable`, or `disable`.
    pub action: String,
    /// Command name or hexadecimal command ID for enable and disable operations.
    pub command: Option<String>,
}

impl CommandArgs for CommandAdminArguments {
    fn schema() -> CommandSchema {
        CommandSchema::new()
            .argument(
                ArgumentSpec::new("action")
                    .choice(crate::ArgumentChoice::new("list", "list"))
                    .choice(crate::ArgumentChoice::new("enable", "enable"))
                    .choice(crate::ArgumentChoice::new("disable", "disable")),
            )
            .argument(ArgumentSpec::new("command").required(false))
    }

    fn from_arguments(arguments: &ParsedArguments) -> Result<Self, CommandParseError> {
        Ok(Self {
            action: arguments.required("action")?,
            command: arguments.optional("command")?,
        })
    }
}

async fn command_admin_handler(
    Args(arguments): Args<CommandAdminArguments>,
    registry: CommandRegistry,
) -> HandlerResult<String> {
    match arguments.action.as_str() {
        "list" => {
            let lines = registry
                .snapshot()
                .into_iter()
                .map(|(id, state)| {
                    format!(
                        "{} {:016x} {}",
                        if state.enabled { "on " } else { "off" },
                        id.0,
                        state.name
                    )
                })
                .collect::<Vec<_>>();
            Ok(if lines.is_empty() {
                "没有已注册命令".to_owned()
            } else {
                lines.join("\n")
            })
        }
        "enable" | "disable" => {
            let command = arguments.command.ok_or_else(|| {
                HandlerError::user("enable/disable 需要命令名称或十六进制命令 ID")
            })?;
            let id = u64::from_str_radix(command.trim_start_matches("0x"), 16)
                .ok()
                .map(crate::CommandId)
                .or_else(|| registry.find(&command))
                .ok_or_else(|| HandlerError::user("没有找到该命令"))?;
            let changed = if arguments.action == "enable" {
                registry.enable_and_publish(id).await?
            } else {
                registry.disable_and_publish(id).await?
            };
            Ok(if changed {
                format!(
                    "命令 {} 已{}",
                    command,
                    if arguments.action == "enable" {
                        "启用"
                    } else {
                        "停用"
                    }
                )
            } else {
                "命令状态未改变".to_owned()
            })
        }
        _ => Err(HandlerError::user("未知命令管理操作")),
    }
}

/// Returns a module providing the command-management command.
#[must_use]
pub fn command_admin_module<S>() -> Module<S>
where
    S: Send + Sync + 'static,
{
    command_admin_module_named("commands")
}

fn command_admin_module_named<S>(name: impl Into<Arc<str>>) -> Module<S>
where
    S: Send + Sync + 'static,
{
    Module::new().command(
        CommandAdminArguments::command(name).description("查看、启用或停用运行时命令"),
        command_admin_handler,
    )
}

/// Parsed arguments accepted by the standard shortcut-management command.
#[derive(Clone, Debug)]
pub struct ShortcutArguments {
    /// Requested operation: `list`, `add`, `remove`, or `clear`.
    pub action: String,
    /// Name of the command whose shortcuts are being managed.
    pub command: String,
    /// Literal or regular-expression shortcut pattern for add and remove.
    pub pattern: Option<String>,
    /// Command text that replaces the pattern when adding a shortcut.
    pub replacement: Vec<String>,
    /// Whether `pattern` is interpreted as a regular expression.
    pub regex: bool,
    /// Whether literal replacement preserves only a compact tail.
    pub compact: bool,
}

impl CommandArgs for ShortcutArguments {
    fn schema() -> CommandSchema {
        CommandSchema::new()
            .argument(
                ArgumentSpec::new("action")
                    .choice(crate::ArgumentChoice::new("list", "list"))
                    .choice(crate::ArgumentChoice::new("add", "add"))
                    .choice(crate::ArgumentChoice::new("remove", "remove"))
                    .choice(crate::ArgumentChoice::new("clear", "clear")),
            )
            .argument(ArgumentSpec::new("command"))
            .argument(ArgumentSpec::new("pattern").required(false))
            .argument(
                ArgumentSpec::new("replacement")
                    .rest(true)
                    .multiple(true)
                    .required(false),
            )
            .argument(
                ArgumentSpec::new("regex")
                    .long("regex")
                    .short('r')
                    .flag(true),
            )
            .argument(ArgumentSpec::new("compact").long("compact").flag(true))
    }

    fn from_arguments(arguments: &ParsedArguments) -> Result<Self, CommandParseError> {
        Ok(Self {
            action: arguments.required("action")?,
            command: arguments.required("command")?,
            pattern: arguments.optional("pattern")?,
            replacement: arguments.many("replacement")?,
            regex: arguments.flag("regex"),
            compact: arguments.flag("compact"),
        })
    }
}

async fn shortcut_handler(
    Args(arguments): Args<ShortcutArguments>,
    registry: CommandRegistry,
) -> HandlerResult<String> {
    let id = registry
        .find(&arguments.command)
        .ok_or_else(|| HandlerError::user("没有找到目标命令"))?;
    match arguments.action.as_str() {
        "list" => {
            let shortcuts = registry.shortcuts(id);
            Ok(if shortcuts.is_empty() {
                format!("命令 {} 没有快捷指令", arguments.command)
            } else {
                shortcuts
                    .iter()
                    .map(|shortcut| {
                        format!(
                            "{}{}",
                            if shortcut.is_regex() {
                                "regex: "
                            } else {
                                "literal: "
                            },
                            shortcut.display(),
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
        }
        "add" => {
            let pattern = arguments
                .pattern
                .ok_or_else(|| HandlerError::user("add 需要快捷指令匹配模式"))?;
            let replacement = arguments.replacement.join(" ");
            if replacement.trim().is_empty() {
                return Err(HandlerError::user("add 需要替换后的标准命令"));
            }
            let shortcut = if arguments.regex {
                Shortcut::regex(&pattern, replacement)
                    .map_err(|error| HandlerError::user(format!("无效正则表达式：{error}")))?
            } else {
                Shortcut::literal(pattern.clone(), replacement).compact(arguments.compact)
            };
            registry.add_shortcut(id, shortcut)?;
            Ok(format!(
                "已为 {} 添加快捷指令 {}",
                arguments.command, pattern
            ))
        }
        "remove" => {
            let pattern = arguments
                .pattern
                .ok_or_else(|| HandlerError::user("remove 需要快捷指令匹配模式"))?;
            if registry.remove_shortcut(id, &pattern)? {
                Ok(format!("已删除快捷指令 {pattern}"))
            } else {
                Err(HandlerError::user("没有找到该快捷指令"))
            }
        }
        "clear" => {
            let removed = registry.clear_shortcuts(id);
            Ok(format!("已清除 {removed} 条快捷指令"))
        }
        _ => Err(HandlerError::user("未知快捷指令操作")),
    }
}

/// Returns a module providing the shortcut-management command.
#[must_use]
pub fn shortcut_admin_module<S>() -> Module<S>
where
    S: Send + Sync + 'static,
{
    shortcut_admin_module_named("shortcut")
}

fn shortcut_admin_module_named<S>(name: impl Into<Arc<str>>) -> Module<S>
where
    S: Send + Sync + 'static,
{
    Module::new().runtime_shortcuts().command(
        ShortcutArguments::command(name).description("增加、删除或列出有界运行时快捷指令"),
        shortcut_handler,
    )
}

async fn diagnostics_handler(registry: CommandRegistry) -> String {
    let snapshot = registry.snapshot();
    let enabled = snapshot.iter().filter(|(_, state)| state.enabled).count();
    format!(
        "commands={} enabled={} registry_revision={}",
        snapshot.len(),
        enabled,
        registry.revision(),
    )
}

/// Returns a module providing read-only command registry diagnostics.
#[must_use]
pub fn diagnostics_module<S>() -> Module<S>
where
    S: Send + Sync + 'static,
{
    diagnostics_module_named("oxidebot diagnostics")
}

fn diagnostics_module_named<S>(name: impl Into<Arc<str>>) -> Module<S>
where
    S: Send + Sync + 'static,
{
    Module::new().command(
        command(name).description("显示 OxideBot 命令注册表诊断信息"),
        diagnostics_handler,
    )
}
