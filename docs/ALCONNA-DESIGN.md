# Domain-native command and message authoring

OxideBot borrows the useful product ideas demonstrated by plugin-alconna while
keeping a Rust-native, bounded, statically composed runtime. The goal is not API
compatibility. The goal is one coherent authoring pipeline built on the
canonical `Event`, `Message`, and `CommandMatch` intermediate representations.

## One pipeline

```text
platform Event
-> canonical Message
-> MessageNormalizer
-> static Shortcut / CommandRewriter
-> one Command tree parse
-> CommandMiddleware
-> static and dynamic completion
-> synchronous Extract / Args / BranchArgs
-> ordinary async handler
-> Outcome / CommandOutput
-> CommandOutputMiddleware / CommandRenderer
-> DeliveryMiddleware
-> capability-aware DeliveryPlan
-> adapter transport
-> DeliveryReport / Receipt
```

Every extension point is registered on `OxideBot` before `build`. There is no
global mutable extension registry, dynamically injected `Any` state, or second
plugin scheduler.

## Function commands

Small commands can be generated directly from an async function. Parameters
with `#[arg(...)]` become command fields; other parameters remain normal
`Extract<S>` values.

```rust
#[oxidebot::command("echo")]
async fn echo(
    #[arg(rest, required = true)] text: Vec<String>,
    Sender(user): Sender,
) -> String {
    format!("{}: {}", user.id, text.join(" "))
}

let module = Module::new().command(echo_command(), echo);
```

Use `CommandArgs` and `BotCommand` when a schema is shared, nested, or large.

## Command trees and branch handlers

```rust
#[derive(CommandArgs)]
struct AddArgs {
    #[arg(rest, required = true)]
    text: Vec<String>,
}

#[derive(BotCommand)]
#[command(name = "todo")]
enum TodoCommand {
    Add(AddArgs),
    List,
}

async fn add(BranchArgs(args): BranchArgs<todo_command_branches::Add>) -> String {
    args.text.join(" ")
}

async fn list(_: BranchArgs<todo_command_branches::List>) -> &'static str {
    "no todos"
}

let module = Module::new()
    .command_branch(todo_command_branches::Add, add)
    .command_branch(todo_command_branches::List, list);
```

A command is parsed once. Branch selection uses the stable command-tree path;
it does not invoke a second parser. Nested `#[command(subcommand)]` variants
preserve their descendant path.

For data-dependent conditions, use generated field IDs with a guard:

```rust
let urgent = Module::new()
    .command(TodoCommand::command(), urgent_handler)
    .guard(when_field_equals(todo_field_id, 1_u8));
```

## Static shortcuts and command rewriting

Static shortcuts compile with the command:

```rust
let command = TodoCommand::command()
    .shortcut(Shortcut::literal("待办列表", "/todo list"))
    .shortcut(Shortcut::regex(
        r"^添加(?P<text>.+)$",
        "/todo add {text}",
    )?);
```

Replacement templates support numbered captures, named captures, and `{*}`
for the unmatched tail. Runtime shortcuts are bounded by `CommandRegistry` and
are opt-in through `Module::runtime_shortcuts()` or the standard shortcut
administration module.

A `CommandRewriter<S>` handles transformations that need the complete message,
state, locale, or an external model:

```rust
async fn rewrite(
    _context: Context<AppState>,
    input: RewriteInput,
) -> HandlerResult<RewriteInput> {
    Ok(input)
}

let app = OxideBot::with_state(state).command_rewriter(rewrite);
```

Rewriters must produce another canonical message. They do not directly execute
a command.

## Dynamic completion

Static choices, subcommands, and options come from the command IR. A dynamic
provider can query application state for one stable field ID:

```rust
async fn complete_projects(
    context: Context<AppState>,
    input: CompletionInput,
) -> HandlerResult<Vec<CompletionItem>> {
    let projects = context.state().projects.search(&input.partial).await?;
    Ok(projects
        .into_iter()
        .map(|project| {
            CompletionItem::new(
                project.id,
                CompletionKind::Choice,
                input.replace,
            )
            .description(project.name)
        })
        .collect())
}

let command = DeployArgs::command("deploy");
let app = OxideBot::with_state(state)
    .completer_for(&command, &[], "project", complete_projects)?
    .include(Module::new().command(command, deploy));
```

The same provider is used by text `?` completion, interactive missing-argument
recovery, and platform-native autocomplete. Candidate count is bounded.

## Renderer and locale

Help, parse diagnostics, catalog pages, and completion remain structured as
`CommandOutput` until the final renderer.

```rust
let app = OxideBot::with_state(state)
    .locale_resolver(StoredLocaleResolver)
    .command_renderer(|output: &CommandOutput, locale| {
        DefaultCommandRenderer.render(output, locale)
    });
```

`TranslationCatalog` stores bounded, structure-preserving `MessageTemplate`
values. It falls back from a full locale to the language and then to the
configured default locale.

```rust
let translations = TranslationCatalog::bounded(1_024, "en-US");
translations.insert("en-US", "todo.created", "Created {id:text}")?;
translations.insert("zh-CN", "todo.created", "已创建 {id:text}")?;

let message = LocalizedMessage::new("todo.created")
    .arg("id", 42_u64)
    .render(&translations, Some("zh-CN"))?;
```

`language_module` and `StoredLocaleResolver` use a user-provided
`LocaleStorage`; they do not impose a database.

## Message queries, templates, and transformation

The canonical message IR supports typed selectors without a second message
model:

```rust
if message.has_type::<Images>() {
    let first = message.first_type::<Images>();
    let nested = message.select_recursive::<Images>();
}

let safe = message.transform_recursive(|segment| match segment {
    MessageSegment::PlatformNative { .. } => SegmentTransform::Drop,
    _ => SegmentTransform::Keep,
});
```

Templates produce structure directly:

```rust
let template = message_template!("Welcome {user:mention}; file: {report:file}");
let message = template.render(&message_args! {
    user => TemplateValue::mention(user.id),
    report => report_file,
})?;
```

No template string is executed as code.

## Delivery extensions and media

`DeliveryMiddleware<S>` transforms a bounded `DeliveryPlan`, not an arbitrary
adapter payload. The planner records every semantic degradation in the final
`DeliveryReport`.

```rust
async fn inspect_delivery(
    _context: Option<Context<AppState>>,
    _target: MessageTarget,
    plan: DeliveryPlan,
) -> HandlerResult<DeliveryPlan> {
    Ok(plan)
}

let app = OxideBot::with_state(state).delivery_middleware(inspect_delivery);
```

`MediaResolver`, `MediaFetcher`, and `MediaHost` are explicit services. The core
never downloads an arbitrary URL implicitly. `LocalMediaResolver` enforces a
byte limit; `PortableMediaResolver` delegates network access to the caller.

## Deterministic proactive targets

```rust
let address = Address::channel(channel_id)
    .through(BotSelection::Exact(bot_identity));

bot_directory
    .send_address(address, message, FallbackPolicy::Auto)
    .await?;
```

`BotSelection::Platform` succeeds only when one connected bot matches. OxideBot
does not randomly choose a bot. `TargetDirectory` is a bounded alias map for
application-defined destinations.

## Command lifecycle and standard modules

`CommandRegistry` provides explicit runtime enable/disable state, bounded
shortcuts, revision tracking, and native-command republishing. Disabled commands
are removed from help, completion, text dispatch, and platform-native command
definitions.

Standard capabilities are ordinary modules:

```rust
let features = Module::new()
    .include(echo_module())
    .include(language_module())
    .include(shortcut_admin_module())
    .include(command_admin_module())
    .include(diagnostics_module())
    .help();
```

Administration modules deliberately ship without a permission policy. An
application must wrap them in an administrator guard.

## Deliberately not copied

OxideBot intentionally rejects these patterns:

- a global mutable extension registry or load-order-dependent behavior;
- dynamic `Any` dependency injection by parameter name;
- exception-driven `finish`, `reject`, or pause control flow;
- executable code in JSON, YAML, or TOML command configuration;
- implicit fuzzy execution of a near-matching command;
- random bot selection for proactive sends;
- silent loss of unsupported message semantics;
- unbounded message, target, shortcut, completion, or media caches.

Fuzzy matching may suggest a correction, but never authorizes or executes a
command by itself.
