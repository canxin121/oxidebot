# Typed command grammars

OxideBot commands are message-domain grammars, not URL routes. A flat
`Module` never creates a Web-style route tree, while a command may contain a
real user-visible subcommand tree that is shared by text parsing, help,
completion, localization, and platform-native command publication.

## One command IR

Every command is compiled into one immutable `Command` grammar. Text messages
and native slash/menu invocations both produce the same `CommandMatch`, and the
match is parsed once before handler extraction.

For a single command, derive `CommandArgs`:

```rust
use oxidebot::prelude::*;

#[derive(Debug, CommandArgs)]
#[command(description = "Create a release", alias = "release-new")]
struct CreateReleaseArgs {
    /// Release name.
    #[arg(prompt = "What should the release be called?")]
    name: String,

    /// Target environment.
    #[arg(long, short = 'e', default = "staging".to_owned())]
    environment: String,

    /// Increase diagnostic verbosity; `-vvv` becomes 3.
    #[arg(long, short = 'v', action = "count")]
    verbose: u8,

    /// Optional labels.
    #[arg(long, short = 't', action = "append")]
    tags: Vec<String>,

    /// Remaining free-form notes.
    #[arg(rest)]
    notes: Vec<String>,
}

async fn create_release(Args(args): Args<CreateReleaseArgs>) -> String {
    format!("creating {} in {}", args.name, args.environment)
}

let release = CreateReleaseArgs::command("release-create")
    .example("/release-create v1.2 -e production -vv -t stable")
    .handle(create_release);

let features = Module::new().add(release);
```

The generated schema drives:

- text and typed-message parsing;
- native command definitions;
- build-time structural validation;
- localized usage and help;
- static and native autocomplete;
- unknown-option suggestions;
- bounded missing-argument recovery.

## Progressive chat forms

Commands are not limited to one-shot shell-style input. Add `interactive` to
the command declaration (or attach `CompletionConfig` manually) when a user
should be able to start with only the command name and fill the remaining
pieces conversationally:

```rust
#[derive(Debug, CommandEnum)]
enum ReminderTime {
    #[choice(value = "10m", name = "10 分钟后")]
    TenMinutes,
    #[choice(value = "1h", name = "1 小时后")]
    OneHour,
    #[choice(value = "tomorrow", name = "明天")]
    Tomorrow,
}

#[derive(Debug, CommandArgs)]
#[command(interactive)]
struct CreateReminder {
    #[arg(prompt = "提醒内容是什么？")]
    text: String,

    #[arg(prompt = "多久后提醒？", value_enum)]
    when: ReminderTime,
}
```

For `/remind` OxideBot asks one field at a time, includes declared choices and
dynamic completion candidates when present, and accepts the next message as
the answer. Invalid choices, type-conversion failures, ranges, lengths, and
synchronous validators re-prompt the same field by default rather than ending
the flow. `cancel`, `stop`, and `取消` always stop the form. Missing
subcommands and required argument groups are also recoverable. The session is
scoped to the exact bot, conversation, actor, and command branch, so another
user cannot answer it.

`CompletionConfig::max_rounds` bounds the total questions, while
`max_attempts_per_field` bounds invalid retries for one field. Native platform
commands remain one-shot because their UI already collects fields; the chat
form path is used only for text invocations.

## Real subcommand trees

Use `BotCommand` for a root command with branches. A tuple variant wraps one
`CommandArgs` structure; a unit variant has no arguments.

```rust
#[derive(Debug, CommandArgs)]
struct AddArgs {
    #[arg(rest, required = true, prompt = "What should I add?")]
    text: Vec<String>,

    #[arg(long, short = 'p', min = 1.0, max = 5.0, default = 3_u8)]
    priority: u8,
}

#[derive(Debug, CommandArgs)]
struct DoneArgs {
    id: u64,
}

#[derive(Debug, BotCommand)]
#[command(name = "todo", description = "Manage todo items")]
enum TodoCommand {
    /// Create a todo item.
    Add(AddArgs),

    /// Mark an item as complete.
    Done(DoneArgs),

    /// List open items.
    List,
}

async fn todo(Args(command): Args<TodoCommand>) -> String {
    match command {
        TodoCommand::Add(args) => format!("add: {}", args.text.join(" ")),
        TodoCommand::Done(args) => format!("done: {}", args.id),
        TodoCommand::List => "list".to_owned(),
    }
}

let features = Module::new()
    .add(TodoCommand::feature(todo))
    .help();
```

The grammar above accepts `/todo add`, `/todo done`, and `/todo list`. Missing
or unknown branches are diagnosed before the handler runs, with localized
choices and edit-distance suggestions. This is a command syntax tree, while
application composition remains flat.

Nested branches can also be built explicitly with `CommandBranch::subcommand`
when a project needs more than one enum level.

## Structured parse results

Most handlers should request `Args<T>`. Framework tooling can request the
shared `CommandMatch` directly:

```rust
async fn inspect(command: CommandMatch) -> String {
    format!(
        "command={} branch={:?} source={:?}",
        command.command().name(),
        command.branch_names(),
        command.source(),
    )
}
```

`CommandMatch` retains:

- deterministic command, branch, and field IDs;
- canonical and invoked names;
- selected branch path;
- typed text, mention, file, message-segment, and native form values;
- the active merged schema;
- locale and invocation source;
- the parsed arguments after validation.

Generated Rust field IDs are the normal query mechanism. Dynamic tools may use
`ParsedArguments` and schema lookup without reparsing the original message.

## Field forms and constraints

A plain field is positional. `Option<T>` is optional command data, and `Vec<T>`
represents repeated or remaining values. This is separate from handler
extraction; OxideBot intentionally has no generic `Option<Extractor>` that
would swallow arbitrary failures.

Useful field attributes include:

```rust
#[arg(long, short = 'f')]
force: bool,

#[arg(long, action = "append")]
include: Vec<String>,

#[arg(long, short = 'v', action = "count")]
verbosity: u8,

#[arg(value_enum)]
environment: Environment,

#[arg(min = 1.0, max = 100.0)]
percentage: f64,

#[arg(min_length = 3, max_length = 40)]
name: String,

#[arg(requires = "token")]
remote: Option<String>,

#[arg(conflicts_with = "dry_run")]
force: bool,
```

Canonical non-text segments are not flattened. Fields may consume `Mention`,
`File`, `MessageSegment`, `FormValue`, or any custom `FromCommandValue` type.

### Argument groups and global options

Use an argument group whenever a relation is about a *set* of fields rather
than one pair of fields. `exactly_one` is useful for credential sources:

```rust
#[derive(CommandArgs)]
#[command(group(name = "credential", exactly_one))]
struct LoginArgs {
    #[arg(long, group = "credential")]
    token: Option<String>,
    #[arg(long, group = "credential")]
    cookie: Option<String>,
}
```

The group declaration supports `required`, `multiple = false`, and
`exactly_one`. Its constraint appears in help and becomes a structured parse
error, so interactive forms can ask for the group instead of exposing a vague
handler failure.

For a command tree, define shared named options once and attach them with
`global_args`:

```rust
#[derive(CommandArgs)]
struct TodoGlobalArgs {
    #[arg(long)]
    project: Option<String>,
}

#[derive(BotCommand)]
#[command(name = "todo", global_args = TodoGlobalArgs)]
enum TodoCommand {
    Add(AddArgs),
    List,
}
```

Both `/todo --project oxidebot add release` and
`/todo add --project oxidebot release` are valid. Global arguments must be
named options or flags; positional globals would make branch recognition
ambiguous and are rejected at build time.

### Strongly typed finite choices and validators

For a finite, stable business domain, use `CommandEnum` rather than a
`String` plus repeated `choice = ...` values. The latter validates input, but
the handler still receives an untyped string: every consumer must repeat
string comparisons and the compiler cannot check that all cases are handled.
An enum makes impossible states unrepresentable in the handler and keeps
parsing, aliases, displayed labels, text completion, and native choices in
one definition:

```rust
#[derive(CommandEnum)]
enum Environment {
    #[choice(value = "development", name = "开发环境", alias = "dev")]
    Development,
    #[choice(value = "production", name = "生产环境", alias = "prod")]
    Production,
}

#[derive(CommandArgs)]
struct DeployArgs {
    #[arg(long, value_enum)]
    environment: Environment,
}
```

Use `String` only when the value is truly open-ended, such as a title or free
text. When the candidate set comes from a database or remote API at runtime,
use a stable typed identifier where possible, attach a dynamic completer for
discovery, and resolve it explicitly with `ResolveCommandValue`. Do not model
a closed domain such as an environment, task priority, or publish mode as a
bare string.

The enum supplies parsing, aliases, help, text completion, and native command
choices from one definition. For pure field checks, use a synchronous
validator:

```rust
fn valid_batch_size(value: &u8) -> Result<(), &'static str> {
    (*value <= 8).then_some(()).ok_or("must not exceed 8")
}

#[derive(CommandArgs)]
struct BatchArgs {
    #[arg(validate = valid_batch_size)]
    size: u8,
}
```

Stateful or I/O-dependent resolution remains explicitly separate through
`ResolveCommandValue`; it never runs in the parser hot path.

Use `heading` to keep a larger command's help scannable, and `hidden` only for
intentional compatibility or integration fields that should remain parseable
without appearing in ordinary help, completion, usage, or native publication:

```rust
#[derive(CommandArgs)]
struct PublishArgs {
    #[arg(long, heading = "Authentication")]
    token: Option<String>,

    #[arg(long, heading = "Output")]
    format: Option<String>,

    #[arg(long, hidden)]
    legacy_wire_mode: bool,
}
```

## Accepted text syntax

The tokenizer and schema parser support:

- single and double quotes, backslash escapes, and empty quoted values;
- `--long value` and `--long=value`;
- short options, combined flags, and attached short-option values;
- `--` to end option parsing;
- negative numeric positionals;
- store, append, count, set-true, and set-false actions;
- required, optional, defaulted, repeated, and rest arguments;
- exact aliases, multiple prefixes, no-prefix commands, and case folding;
- canonical `Text`, `PlainText`, and `RichText` tokens;
- mentions, files, native form values, and original message segments.

The default `/name` form keeps the exact pre-decode route key. Commands only
enter the broad message candidate slot when custom prefix or case rules require
it.

## Completion

The command IR supports three related flows:

1. `CommandCatalog::suggest(input, cursor, locale)` returns structured
   `CompletionItem`s without executing a handler.
2. Native suggestion events resolve the same argument IDs and choices, then
   call `answer_suggestion_request`.
3. `CompletionConfig` enables bounded dialogue recovery only for a missing
   required argument.

```rust
let deploy = DeployArgs::feature("deploy", deploy).completion(
    CompletionConfig::new()
        .timeout(std::time::Duration::from_secs(60))
        .max_rounds(3)
        .cancel_words(["cancel", "stop", "取消"]),
);
```

Unknown branches, unknown options, invalid values, duplicate options, and
missing option values return immediately. Guards run before a completion
session, so an unauthorized user is never prompted.

## Help and localization

`CommandOutput` is structured before it becomes a message. The default
`CommandRenderer` renders catalogs, branch help, parse errors, and completion
items into the unified `Message` IR.

Descriptions, argument help, prompts, and choices use `LocalizedText`:

```rust
let command = command("status")
    .description("Show status")
    .description_translation("zh-CN", "查看状态");

let project = ArgumentSpec::new("project")
    .help("Project name")
    .help_translation("zh-CN", "项目名称")
    .prompt("Which project?")
    .prompt_translation("zh-CN", "请选择项目：");
```

Locale is resolved from native command/suggestion events when available. A
custom `CommandRenderer` can render the same outputs as plain text, rich text,
buttons, or platform-native layouts without changing the grammar.

## Platform-native publication

At startup, the command catalog converts the same grammar to
`CommandDefinition`. Adapters that report
`BotCapabilities.application.structured_commands` receive the definitions via
`set_command_definitions`.

A platform-native invocation is mapped back to the same `CommandMatch`; it does
not need a second slash-command handler. Native autocomplete uses the same
field IDs, choices, locale, and completion items.

## Branch-specific handlers

A large command tree does not need one giant `match`. Derive-generated branch
markers bind themselves to the owning command tree:

```rust
#[oxidebot::branch(todo_command_branches::Add)]
async fn add(args: AddArgs) -> String {
    args.text.join(" ")
}

#[oxidebot::branch(todo_command_branches::List, unit)]
async fn list() -> &'static str {
    "list"
}

let module = Module::new()
    .add(add)
    .add(list);
```

The root command is still parsed only once. The attribute macro keeps the
stable branch marker and `BranchArgs` plumbing inside generated code. The
lower-level marker APIs remain available for framework tooling.

## Function commands

For a small leaf command, `#[oxidebot::command]` derives the private argument
schema from the function signature:

```rust
#[oxidebot::command("echo")]
async fn echo(
    #[arg(rest, required = true)] text: Vec<String>,
    Sender(user): Sender,
) -> String {
    format!("{}: {}", user.id, text.join(" "))
}

let module = Module::new().add(echo);
```

The generated value contains the schema and handler together, so they cannot
be accidentally mismatched during registration. Parameters carrying `#[arg(...)]` are command values. Other parameters remain
ordinary extractors. Generic functions and methods intentionally use the
explicit `CommandArgs` path instead.

## Shortcuts and rewrites

```rust
let command = TodoCommand::command()
    .shortcut(Shortcut::literal("待办列表", "/todo list"))
    .shortcut(Shortcut::regex(
        r"^添加(?P<text>.+)$",
        "/todo add {text}",
    )
    .expect("valid shortcut pattern"));
```

Literal shortcuts may preserve a token-boundary tail or explicitly allow a
compact tail. Regex replacements support `{0}`, named captures, and `{*}`.
Runtime shortcuts are bounded in `CommandRegistry` and require
`Module::runtime_shortcuts()`.

`CommandRewriter<S>` is the explicit extension point for transformations that
need the complete message or state. It returns another canonical message and
never executes a command directly.

## Conservative root-command typo assistance

Unknown roots do not normally enter command dispatch; this preserves the fast
exact-route path and avoids treating ordinary chat as a command. Applications
that want a small, explicit convenience layer can opt in after registering
their commands:

```rust
let module = Module::new()
    .add(DeployArgs::feature("deploy", deploy))
    .typo_assist();
```

For a close prefixed typo such as `/deply`, it replies with a suggestion such
as `/deploy`; it never executes the suggested command. The helper is a broad
message handler by design, so use it only where its small additional decode
cost and user-visible response are appropriate.

## Dynamic completion

Keep the provider beside its field. `#[oxidebot::completer]` turns one
state-aware async function into a statically typed provider, and the generated
`CommandArgs::feature` installs it automatically:

```rust
#[oxidebot::completer]
async fn complete_projects(
    context: Context<AppState>,
    input: CompletionInput,
) -> HandlerResult<Vec<CompletionItem>> {
    Ok(context
        .state()
        .projects
        .search(&input.partial)
        .await
        .internal("complete projects")?
        .into_iter()
        .take(input.limit)
        .map(|project| {
            CompletionItem::new(
                project.id,
                CompletionKind::Choice,
                input.replace,
            )
            .description(project.name)
            .field(input.field)
        })
        .collect())
}

#[derive(CommandArgs)]
struct DeployArgs {
    #[arg(complete = complete_projects, prompt = "Which project?")]
    project: String,
}

let module = Module::new()
    .add(DeployArgs::feature("deploy", deploy));
```

The same provider feeds textual `?` completion, missing-value dialogue, and
platform-native autocomplete. Providers receive an owned clone-cheap `Context`
and are bounded by the requested result limit.

## Command overlays and lifecycle

`CommandOverlay` changes data-only presentation and invocation metadata without
replacing a handler:

```rust
let module = module.command_overlay(
    CommandOverlay::new("deploy")
        .description_translation("zh-CN", "部署服务")
        .alias("ship")
        .shortcut(Shortcut::literal("发版", "/deploy")),
);
```

`CommandRegistry` exposes explicit enable/disable state, bounded shortcuts, a
revision, and native-command republishing. Disabled commands do not appear in
help, completion, text dispatch, or native command definitions.

## Value patterns and async resolution

`ValuePattern<T>` keeps pure validation and conversion synchronous:

```rust
let ticket = RegexTextPattern::new(
    r"^[A-Z]+-[0-9]+$",
    "an uppercase project key followed by a number",
)?;
```

I/O-dependent conversion is explicit through `Resolve<T, S>` and
`ResolveCommandValue<S>` so ordinary `Args<T>` remains a synchronous hot-path
extractor.
