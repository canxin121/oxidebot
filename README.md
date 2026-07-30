# OxideBot

> **Public API:** The `1.0` line is covered by the
> [stability policy](STABILITY.md); see [the changelog](CHANGELOG.md),
> [the 0.1 migration guide](MIGRATING_FROM_0_1.md), and
> [the stability policy](STABILITY.md).

OxideBot is a high-performance, platform-neutral bot framework built around one
complete semantic model: one `Event` hierarchy, one cross-platform `Message`
intermediate representation, and one canonical `CallApiTrait` adapter boundary.
Text, rich text, media, polls, layouts, components, and platform-native content
all travel through that same model.

The authoring layer uses ordinary async functions, typed extraction,
composable application modules, and explicit effects. Bot-domain concepts stay
visible from adapter ingress through handler output.

The application model is deliberately small:

```text
Adapter -> Event -> Module match -> typed Extract -> async fn -> Outcome / API calls
```

There is no reduced “simple event”, no `RichEvent`, and no second API or event
bus. Every handler reads the same shared event that adapters decode.

## Run a bot immediately

The workspace includes a real stdin/stdout development adapter, a Telegram Bot
API long-polling adapter, and a copyable example:

```bash
cargo run -p oxidebot-console-example
```

Then type `/ping`, `/echo hello`, or `/help` in the terminal. The complete
example is in [`examples/console-bot`](examples/console-bot), and the adapter is
published as the independent `oxidebot-adapter-console` crate inside this
workspace. Telegram integration lives in `oxidebot-adapter-telegram`; see its
crate README before connecting production traffic.

A minimal application is deliberately small:

```rust
use oxidebot::prelude::*;
use oxidebot_adapter_console::ConsoleAdapter;

#[oxidebot::command("ping")]
/// Check whether the bot is alive.
async fn ping() -> &'static str {
    "pong"
}

#[oxidebot::command("echo")]
async fn echo(
    #[arg(rest, required = true, prompt = "What should I echo?")]
    content: Vec<String>,
) -> String {
    content.join(" ")
}

#[tokio::main]
async fn main() -> oxidebot::Result<()> {
    OxideBot::new()
        .adapter(ConsoleAdapter::development())
        .add(ping)
        .add(echo)
        .include(Module::new().help())
        .run()
        .await
}
```

`#[oxidebot::command]` produces one generated feature value containing the
command schema, handler, help metadata, platform-command definition, and any
field-local completers. It is installed once with `.add(ping)`; there is no
separate `ping_command()`/handler pairing to keep synchronized.

For reusable command argument types, derive `CommandArgs` and attach the command
and handler as one locally configurable feature:

```rust
#[derive(Debug, CommandArgs)]
#[command(description = "Search the knowledge base", alias = "find")]
struct SearchArgs {
    #[arg(prompt = "What should I search for?")]
    query: String,

    #[arg(long, short = 'n', default = 10_u32, min = 1.0, max = 100.0)]
    limit: u32,
}

async fn search(
    Args(args): Args<SearchArgs>,
    State(state): State<AppState>,
    Sender(user): Sender,
) -> HandlerResult<String> {
    state
        .search(user.id, args.query, args.limit)
        .await
        .internal("search knowledge base")
}

let search = SearchArgs::feature("search", search)
    .guard(can_search)
    .before(audit_start)
    .after(record_metrics);

let features = Module::new()
    .add(search)
    .help();
```

Handlers may return:

- `&'static str`, `String`, `Message`, `MessageSegment`, or
  `Vec<MessageSegment>` to enqueue a reply;
- `()` when no deferred reply is needed;
- `Option<T>` for an optional effect without changing the handler kind's routing default;
- `Outcome` for multiple replies or an explicit propagation override;
- `Result<T, E>` when `T: IntoOutcome` and `E: Into<HandlerError>`.

A return value controls **effects**, not hidden routing policy. Commands and
interactions block later matching handlers by default; ordinary event and
native-event observers continue by default. Use `Outcome::stop()` or
`Outcome::continue_()` only when a handler needs to override that natural
default.

## Typed extraction without a Web request

A handler declares exactly the event data it needs:

```rust
async fn greet(
    Sender(user): Sender,
    MaybeConversation(conversation): MaybeConversation,
    Text(text): Text,
) -> Message {
    let location = conversation
        .as_ref()
        .map(|conversation| format!("conversation {}", conversation.id))
        .unwrap_or_else(|| "platform event".to_owned());

    Message::text("Hello ")
        .at(user.id)
        .then(format!(" in {location}. You said: {text}"))
}
```

Built-in extractors include:

- `Text`, `Segments`, `Sender`, `Conversation`, `MaybeConversation`, `IncomingMessageId`, and
  `Target`;
- root `State<S>` and the common `Context<S>`;
- typed command `Args<T>` and raw `CommandMatch`;
- `Bot`, `Reply`, and `Dialogue`;
- `EventContext<tags::...>`, `BotIdentity`, `EventId`, and `ShutdownSignal`.

Extraction is synchronous, monomorphized, and runs only after a handler has
matched and command validation/completion has succeeded. There is no
asynchronous `FromRequest`, dynamic extension map, or runtime substate registry
in the hot path.

Application services can be exposed statically with `BotState`:

```rust
#[derive(BotState)]
struct AppState {
    #[state]
    projects: ProjectStore,

    #[state]
    database: std::sync::Arc<Database>,
}

async fn list(
    State(projects): State<ProjectStore>,
    State(database): State<Database>,
) -> HandlerResult<String> {
    // Both values are selected at compile time from the root AppState.
    let count = projects.count(database.as_ref()).await.internal("count projects")?;
    Ok(format!("{count} projects"))
}
```

For event-derived domain values, a custom extractor remains a small explicit
implementation:

```rust
struct ProjectId(String);
struct CurrentProject(ProjectId);

impl Extract<AppState> for CurrentProject {
    fn extract(context: &Context<AppState>) -> Result<Self, ExtractError> {
        let text = context
            .message()
            .ok_or_else(|| ExtractError::new("a message is required"))?
            .message
            .get_raw_text();

        (!text.trim().is_empty())
            .then(|| Self(ProjectId(text)))
            .ok_or_else(|| ExtractError::new("no valid project was selected"))
    }
}
```

Optional data is represented by explicit domain extractors such as
`MaybeConversation`. A generic `Option<T>` extractor is intentionally absent because
it would turn permission, parsing, API, and configuration failures into
indistinguishable `None` values.

## Strongly typed command grammars

`#[derive(CommandArgs)]` generates one leaf schema, while `#[derive(BotCommand)]`
generates a real user-visible subcommand tree. The same immutable command IR is
used by text parsing, structured `CommandMatch`, localized help, completion,
platform-native command publication, and native autocomplete. Handlers never
need to split raw strings or maintain a second slash-command definition.

```rust
#[derive(Debug, CommandArgs)]
#[command(description = "Deploy a service", category = "operations", alias = "ship")]
struct DeployArgs {
    #[arg(prompt = "Which service should be deployed?")]
    service: String,

    #[arg(long, short = 'e')]
    environment: Option<String>,

    #[arg(long, short = 'f')]
    force: bool,

    #[arg(rest)]
    extra: Vec<String>,
}

async fn deploy(Args(args): Args<DeployArgs>) -> String {
    format!("deploying {}", args.service)
}

let deploy = DeployArgs::feature("deploy", deploy).completion(
    CompletionConfig::new()
        .timeout(std::time::Duration::from_secs(60))
        .max_rounds(3)
        .cancel_words(["cancel", "stop", "取消"]),
);
```

The parser supports multi-word commands, aliases, multiple prefixes,
case-insensitive and no-prefix commands, quoted or escaped text, empty quoted
values, `--long=value`, combined short flags, attached short-option values,
`--`, negative numeric positionals, repeated/rest values, and typed mentions,
files, and original message segments.

A command tree can be declared directly:

```rust
#[derive(Debug, BotCommand)]
#[command(name = "todo", description = "Manage todos")]
enum TodoCommand {
    Add(AddTodoArgs),
    Done(DoneTodoArgs),
    List,
}

async fn todo(Args(command): Args<TodoCommand>) -> String {
    match command {
        TodoCommand::Add(args) => format!("add {}", args.text.join(" ")),
        TodoCommand::Done(args) => format!("done {}", args.id),
        TodoCommand::List => "list".to_owned(),
    }
}

let module = Module::new()
    .add(TodoCommand::feature(todo))
    .help();
```

Only a missing required argument can enter interactive dialogue recovery.
Unknown subcommands/options, missing option values, and invalid conversions
fail immediately with localized choices and command-specific usage. Guards run
before completion, so an unauthorized or rate-limited user is never prompted.

See [docs/COMMANDS.md](docs/COMMANDS.md).

## Flat feature modules

A `Module<S>` is a reusable collection of bot behavior, not a URL tree:

```rust
fn account_module() -> Module<AppState> {
    Module::new()
        .add(command("account show").handle(show_account))
        .add(command("account delete").handle(delete_account))
}

fn moderation_module() -> Module<AppState> {
    Module::new()
        .add(
            command("admin ban")
                .handle(ban_member)
                .guard(admin_only)
                .before(audit_start),
        )
        .add(
            command("admin mute")
                .handle(mute_member)
                .guard(admin_only),
        )
}

let features = Module::new()
    .include(account_module())
    .include(moderation_module())
    .on(tags::GroupMemberJoined, welcome_member)
    .interaction("settings.save", save_settings)
    .native("vendor.special_event", native_event)
    .after(trace_outcome)
    .help();
```

`include` preserves registration order and flattens behavior into the same
compiled route tables. It does not add a command prefix, create nesting, start a
runtime, or introduce another event bus. Multi-word command paths are
written explicitly because they are command syntax, not nested routes.

Module-wide guards and hooks are declaration-order independent:

```rust
async fn admin_only(context: Context<AppState>) -> GuardDecision {
    let Some(message) = context.message() else {
        return GuardDecision::skip();
    };

    if context.state().is_admin(&message.sender.id) {
        GuardDecision::allow()
    } else {
        GuardDecision::deny("You do not have administrator permission.")
    }
}

async fn started(context: Context<AppState>) {
    tracing::debug!(event = ?context.event_type(), "handler started");
}

async fn finished(context: Context<AppState>, outcome: Outcome) -> Outcome {
    tracing::debug!(
        event = ?context.event_type(),
        replies = outcome.replies().len(),
        propagation = ?outcome.propagation(),
        "handler finished",
    );
    outcome
}

let admin = moderation_module()
    .guard(admin_only)
    .before(started)
    .after(finished);
```

Bot concerns are expressed directly:

- `guard` for permission, rate-limit, chat-type, and feature admission;
- `before` for pre-handler observation or preparation;
- `after` for observing or transforming produced effects.

Parent module guards and hooks apply to included modules regardless of whether
they are declared before or after `include`, eliminating ordering traps. See
[docs/MODULES.md](docs/MODULES.md).

## One message IR, delivery planning, and receipts

All inbound events, handler results, active sends, command tokenization, and
adapter export use the same `Message`. Concise constructors and rich segments
produce the same canonical enum:

```rust
let message = Message::text("Hello ")
    .at(user_id)
    .then(" — your report is ready")
    .file(File::from_path("report.pdf"))
    .reply_to(message_id);

let equivalent = message![
    "Hello ",
    MessageSegment::at(user_id),
    " — your report is ready",
    MessageSegment::file(File::from_path("report.pdf")),
];
```

Rich and interactive content uses the same value. Import the focused message
prelude when constructing advanced layouts:

```rust
use oxidebot::message::prelude::*;

let message = Message::rich_text(
    RichText::plain("Deployment complete")
        .span(0..10, TextStyle::Bold),
)
.components(MessageComponents::InlineKeyboard(
    InlineKeyboard::new([ActionRow::buttons([
        Button::url("Open", "https://example.invalid"),
    ])]),
));
```

Before I/O, `Message::plan_for` combines the target `BotCapabilities` with a
`FallbackPolicy`. It returns a `DeliveryPlan` containing physical messages and
explicit degradations. `Strict` refuses unsupported semantics; `Auto`,
`ToText`, `Flatten`, and `DropUnsupported` remain observable through
`DeliveryReport` rather than silently losing content.

Returning a message is the shortest path. Use `Messenger` when the workflow
needs a platform result immediately or an explicit proactive destination:

```rust
use oxidebot::delivery::prelude::Address;

async fn generate_report(messenger: Messenger) -> HandlerResult<()> {
    let progress = messenger.reply("Generating report…").await?;
    create_report().await.internal("create report")?;
    progress.edit("Report complete").await?;
    progress.react("✅").await?;

    messenger
        .to(Address::group("operations"))?
        .send("A report was generated")
        .await?;
    Ok(())
}
```

`Reply` remains the smaller current-conversation primitive. Framework sends
from `Reply`, `Messenger`, dialogues, deferred handler results, receipts,
interactions, cross-bot addresses, and command publication all pass through the
same bounded outbound scheduler. Raw `Bot`/`CallApiTrait` access is the explicit
platform escape hatch.

A `Receipt` is produced only by a send operation. It tracks every physical
message ID returned for one logical send and can edit, delete, react to, or
delay-delete all of them. `DeliveryReport::items` and the receipt
`*_report` methods retain partial success when a later physical operation
fails, so callers can retry or compensate without duplicating prior sends. A
receipt is not used as an ambiguous handle for the incoming message.

## Bounded dialogues and typed forms

One-off questions are retryable and validated without a hand-written loop:

```rust
async fn setup(dialogue: Dialogue) -> HandlerResult<String> {
    let workers = dialogue
        .named("project-setup")
        .question::<usize>("Worker count?")
        .attempts(3)
        .error("Enter a number between 1 and 64.")
        .validate(|value| (1..=64).contains(value))
        .await?;

    let environment = dialogue
        .choose(
            "Environment?",
            [("Development", "dev"), ("Production", "prod")],
        )
        .await?;

    Ok(format!("workers={workers} environment={environment}"))
}
```

Reusable flows can be declared once:

```rust
#[derive(DialogueForm)]
struct ProjectSetup {
    #[dialogue(prompt = "Project name?", attempts = 3)]
    name: String,

    #[dialogue(
        prompt = "Environment?",
        choice = "Development=dev",
        choice = "Production=prod"
    )]
    environment: String,

    #[dialogue(prompt = "Worker count?", attempts = 3, validate = valid_workers)]
    workers: usize,

    #[dialogue(prompt = "Create the project?", confirm)]
    confirmed: bool,
}

fn valid_workers(value: &usize) -> Result<(), &'static str> {
    (1..=64)
        .contains(value)
        .then_some(())
        .ok_or("worker count must be between 1 and 64")
}

async fn wizard(dialogue: Dialogue) -> HandlerResult<String> {
    let setup = dialogue.named("project-setup").form::<ProjectSetup>().await?;
    Ok(format!("{}: {} workers", setup.name, setup.workers))
}
```

Choice prompts carry buttons in the unified message IR and retain numbered text
fallbacks for platforms without components. Dialogues use the existing bounded
session registry and exact bot, conversation, actor, and namespace keys.

## Command authoring capabilities

Shortcuts, rewriting, completion, and structured output are integrated into the
same `Message`, `Command`, `CommandMatch`, and `Module` pipeline.

### Shortcuts and command rewriting

Static shortcuts are compiled with a command and always flow back through the
canonical parser:

```rust
let roll = RollArgs::command("roll")
    .shortcut(Shortcut::literal("掷骰子", "/roll 1d6"))
    .shortcut(
        Shortcut::regex(r"^掷(?P<count>\d+)个骰子$", "/roll {count}d6")
            .expect("valid shortcut pattern")
            .humanized("掷 N 个骰子"),
    );
```

Application-wide natural-language rewrites use a bounded `CommandRewriter`:

```rust
let app = OxideBot::with_state(state)
    .command_rewriter(|context, mut input: RewriteInput| async move {
        if input.message.extract_plain_text() == "查看状态" {
            input.message = Message::text("/status");
        }
        Ok(input)
    });
```

`shortcut_admin_module()` optionally manages runtime shortcuts through the
bounded `CommandRegistry`. Static shortcuts remain immutable, duplicate patterns
are rejected globally, and runtime shortcuts cannot silently shadow another
command.

### Branch handlers and simple function commands

Large command trees do not need one giant `match` expression. The derive macro
generates branch marker types that reuse the already parsed `CommandMatch`:

```rust
#[oxidebot::branch(todo_command_branches::Add)]
async fn add(
    args: AddTodoArgs,
    State(state): State<AppState>,
) -> HandlerResult<Message> {
    // `args` is taken from the already parsed CommandMatch.
    todo_service::add(&state, args).await
}

#[oxidebot::branch(todo_command_branches::List, unit)]
async fn list(State(state): State<AppState>) -> HandlerResult<Message> {
    todo_service::list(&state).await
}

let todo = Module::new()
    .add(add)
    .add(list);
```

Small commands may be generated directly from a function signature:

```rust
#[oxidebot::command("echo")]
async fn echo(#[arg(rest)] content: Vec<String>) -> String {
    content.join(" ")
}
```

The attribute macro only treats command-value parameters as schema fields;
normal `Extract<S>` parameters remain ordinary handler dependencies.

### Dynamic completion next to the owning feature

```rust
use oxidebot::commands::prelude::{CompletionInput, CompletionItem, CompletionKind};
```

Static choices, command branches, options, and usage are generated from the
command IR. A `CommandArgs` derive also generates stable field marker types, so
dynamic providers no longer use a command reference, string branch path, and
string field name in a distant application builder:

```rust
#[oxidebot::completer]
async fn complete_projects(
    context: Context<AppState>,
    input: CompletionInput,
) -> HandlerResult<Vec<CompletionItem>> {
    let projects = context
        .state()
        .projects
        .search(input.partial.as_ref())
        .await
        .internal("complete projects")?;

    Ok(projects
        .into_iter()
        .take(input.limit)
        .map(|project| {
            CompletionItem::new(
                project.id.to_string(),
                CompletionKind::Choice,
                input.replace,
            )
            .description(project.name)
            .field(input.field)
        })
        .collect())
}

#[derive(CommandArgs)]
struct ProjectDeployArgs {
    #[arg(complete = complete_projects, prompt = "Which project?")]
    project: ProjectId,
}

let deploy = ProjectDeployArgs::feature("deploy", deploy_project)
    .completion(CompletionConfig::new().max_rounds(3));

let module = Module::new().add(deploy);
```

For a function-signature command, the provider stays directly on the field:

```rust
#[oxidebot::command("deploy")]
async fn deploy(
    #[arg(complete = complete_projects)] project: ProjectId,
    State(state): State<AppState>,
) -> HandlerResult<Message> {
    // ...
}

let module = Module::new().add(deploy);
```

The same provider serves text suggestions, bounded interactive completion, and
platform-native autocomplete. It runs only on the completion cold path.

### Configurable output, locale, and resource translations

Help, usage, parse errors, shortcut output, and suggestions first become a
structured `CommandOutput`. Applications may install a renderer and locale
resolver without replacing command parsing:

```rust
OxideBot::with_state(state)
    .command_renderer(MyRenderer)
    .locale_resolver(StoredLocaleResolver)
    .include(language_module());
```

For the common resource-file path, place one JSON object per locale:

```text
locales/
  en-US.json
  zh-CN.json
```

```json
{
  "deploy.started": "Deploying {project:text}…",
  "deploy.finished": "Deployment {project:text} completed",
  "oxidebot.command.parse_error": "{error:text}\n\nUsage: {usage:text}"
}
```

Then install application messages and resource-backed command output together:

```rust
let app = OxideBot::with_state(state)
    .localization_from_dir("locales", "en-US", 1_024)
    .expect("translation resources must be valid");
```

Handlers request the resolved locale and catalog through one extractor:

```rust
async fn deploy(
    i18n: I18n,
    Args(args): Args<DeployArgs>,
) -> HandlerResult<Message> {
    i18n
        .message("deploy.finished")
        .arg("project", args.project)
        .await
}
```

Translation values are structure-preserving `MessageTemplate`s, so placeholders
may produce mentions, files, complete messages, or other canonical segments
instead of being flattened to text. `CommandOverlay` remains the data-only way
to adjust command descriptions, aliases, prefixes, visibility, and shortcuts;
it never loads executable code from configuration.

### Narrow authoring extensions

OxideBot deliberately avoids a giant dynamically typed extension object. Each
phase has a narrow trait and a frozen build-time chain:

```text
canonical Message
-> MessageNormalizer
-> Shortcut / CommandRewriter
-> Command parser
-> CommandMiddleware
-> dynamic or interactive completion
-> typed handler
-> CommandOutputMiddleware / CommandRenderer
-> DeliveryMiddleware
-> capability planner
-> adapter transport
```

This supports normalization, permission-aware command transformations, themed
output, audit metadata, and delivery policy without runtime parameter capture or
global mutable plugin hooks.

### Proactive targets and portable media

Proactive sends use an explicit `Address` and deterministic bot selection:

```rust
use oxidebot::delivery::prelude::{Address, BotSelection, FallbackPolicy};

bot_directory
    .send_address(
        Address::group("ops")
            .through(BotSelection::Exact(bot_identity)),
        Message::text("deployment complete"),
        FallbackPolicy::Auto,
    )
    .await?;
```

A platform-only selection succeeds only when exactly one connected bot matches.
`TargetDirectory` is an optional bounded alias directory; it is not part of the
dispatch hot path.

`MediaResolver`, `MediaFetcher`, and `MediaHost` form an optional, policy-owned
media pipeline. The core local resolver enforces a byte limit and supports local
paths and retained base64 data. Network fetching and object hosting are explicit
application services rather than hidden framework I/O.

### Standard modules and runtime command control

The facade includes ordinary reusable modules:

```rust
use oxidebot::standard::{echo_module, language_module, AdminTools};

let features = Module::new()
    .include(echo_module())
    .include(language_module())
    .include(
        AdminTools::new()
            .prefix("oxidebot")
            .protected(admin_only),
    );
```

`AdminTools` can independently enable command management, runtime shortcuts,
and diagnostics. One application guard protects the complete tool set without
manually creating several tiny modules. Command enable/disable changes update
help, completion, dispatch, and platform-native publication through the same
bounded registry.

## Complete event and API access

Typed handlers can use a concrete view of the shared event allocation:

```rust
async fn joined(
    event: EventContext<tags::GroupMemberJoined>,
) -> Message {
    Message::text("Welcome ")
        .at(event.user.id.clone())
        .then(format!(" to {}", event.conversation.id))
}
```

`EventContext<Tag>` dereferences to the concrete tagged payload and
exposes the event ID, bot identity, complete bot API, and shutdown signal. State
stays explicit: ask for `State<AppState>` or `Context<AppState>` as another
handler argument. `Bot` dereferences to the complete `CallApiTrait`, including
platform-neutral APIs and `call_platform_api`.

## Explicit error boundary

OxideBot never converts an arbitrary `Display` error into a user reply.
Internal command, session, API, application, and service errors are logged by
the runtime. Only an explicitly user-facing error is sent:

```rust
return Err(HandlerError::user("The requested project does not exist."));
```

Safe result extensions keep ordinary business code concise without restoring
unsafe `Display`-to-chat conversion:

```rust
let project = repository
    .find(project_id)
    .await
    .internal("load project")?;

bot.delete_message_ref(oxidebot::core::MessageRef::new(message_id))
    .await
    .api_context("delete progress message")?;
```

Extractor failures are also safe, user-facing errors because they describe a
handler contract such as “this command requires a group chat” or invalid typed
arguments. Their replies inherit the matched handler kind's normal propagation
policy.

## Advanced command and message authoring

The unified IR also drives the higher-level facilities that make large bot
projects practical:

- `Shortcut` and `CommandRewriter<S>` for bounded literal, regex, or
  natural-language command rewrites;
- `#[oxidebot::branch(...)]` for separately implemented command-tree
  branches without exposing marker plumbing or reparsing;
- dynamic completion providers shared by text `?` completion, dialogue
  recovery, and platform-native autocomplete;
- `#[oxidebot::command("name")]` for small function-signature commands;
- configurable `CommandRenderer`, `LocaleResolver`, `TranslationCatalog`, and
  the optional language module;
- `MessageNormalizer`, `CommandMiddleware`, `CommandOutputMiddleware`, and
  `DeliveryMiddleware` as narrow, build-time-frozen extension phases;
- structure-preserving `MessageTemplate`, typed message selectors, recursive
  transformation, explicit media resolution/hosting, deterministic `Address`
  targeting, and a bounded `CommandRegistry`;
- built-in help plus ordinary standard modules for echo, language, shortcut administration,
  command administration, and diagnostics.

These are all ordinary Rust values or traits attached before `build`; none of
them introduces a second runtime or a global mutable extension table.

## Concise deterministic tests

The high-level test DSL covers ordinary commands, multi-turn dialogues, and
answerable interactions while remaining backed by the same finite
`ScriptedAdapter`:

```rust
use oxidebot_testkit::BotTest;

BotTest::feature(ping)
    .message("/ping")
    .expect_reply("pong")
    .run()
    .await?;
```

```rust
BotTest::empty()
    .add(deploy)
    .message("/deploy")
    .expect_reply("Which project?")
    .message("oxidebot")
    .expect_reply_contains("deployment started")
    .run()
    .await?;
```

```rust
BotTest::new(interactions)
    .click("deploy.confirm")
    .expect_interaction_ack()
    .expect_edit_contains("started")
    .run()
    .await?;
```

Command parsing can be tested without starting adapters, queues, or workers:

```rust
let args = oxidebot_testkit::command_test::<DeployArgs>("deploy")
    .parse("/deploy oxidebot --environment prod")?;
```

The lower-level `TestFrame`, `ScriptedAdapter`, and `ScriptedApi` remain
available for admission, retry, scheduling, and byte-budget tests.

## Focused imports

`oxidebot::prelude::*` now contains the normal application path rather than the
entire framework. Specialized work can opt into focused preludes:

```rust
use oxidebot::commands::prelude::*;
use oxidebot::message::prelude::*;
use oxidebot::delivery::prelude::*;
use oxidebot::adapter::prelude::*;
use oxidebot::standard::*;
```

Adapter and framework work should import the required focused module or the
`oxidebot::core` / `oxidebot::runtime` crate re-exports explicitly.

## Grouped runtime configuration

Profiles remain the normal choice. When one area needs tuning, grouped builders
avoid editing a large flat structure:

```rust
let config = RuntimeConfig::balanced()
    .configure_ingress(|ingress| {
        ingress
            .global(1_024, 32 * 1024 * 1024)
            .per_bot(256, 8 * 1024 * 1024)
            .max_frame_events(128);
    })
    .configure_execution(|execution| {
        execution
            .shards(8)
            .in_flight_per_shard(32)
            .handler_timeout(Some(std::time::Duration::from_secs(30)));
    })
    .configure_commands(|commands| {
        commands
            .retries(3)
            .attempt_timeout(Some(std::time::Duration::from_secs(10)))
            .total_timeout(Some(std::time::Duration::from_secs(30)));
    });

config.validate()?;
```

## Adapter authoring shortcut

Simple transports no longer need to implement pre-decode indexing and complete
event construction manually. They may submit the canonical message directly:

```rust
context
    .submit_text(
        event_id,
        channel_id,
        user_id,
        message_id,
        text,
    )
    .await?;
```

For richer sender metadata and an explicit conversation kind, build a `MessageFrame`:

```rust
let frame = MessageFrameBuilder::new(
    event_id,
    ConversationRef::group(channel_id),
    user.id.clone(),
    message,
)
    .sender(user)
    .build();

context.submit(frame).await?;
```

For every other portable event, use `EventFrame` or the even shorter
`submit_event`. The runtime derives the event type, conversation/actor
partition, command key, interaction action key, and native key from the public
`Event`, so normal adapters never import `event::kernel`:

```rust
use oxidebot::adapter::prelude::*;
use oxidebot::event::Event;

// `event` is the adapter's fully normalized portable event.
context.submit_event(event_id, event).await?;

// Preserve a platform occurrence time or use a larger accounting bound only
// for unusually large retained event payloads.
context
    .submit_event_frame(
        EventFrame::new(event_id, Event::Native(native_event))
            .occurred_at(platform_time)
            .retained_bytes(256 * 1024),
    )
    .await?;
```

When one webhook delivery contains multiple normalized events, submit them as
one bounded admission unit with `context.submit_events(frames)`. The runtime
still discards individually uninterested events after validation.

High-throughput adapters may still implement `InboundFrame` directly to reuse
wire-format offsets and preserve the earliest possible interest rejection.

## Plugin bundles

For a reusable feature that needs both handlers and a supervised background
task, package it as a `PluginBundle`. A bundle is deliberately not a nested
router or a second runtime: `OxideBot::plugin` flattens its module and services
into the application in registration order.

```rust
use oxidebot::{command, Module, OxideBot, PluginBundle};

let reminders = PluginBundle::new("reminders")
    .version("1.0")
    .description("Reminder commands and their scheduler")
    .require_capability("outbound plain text", |capabilities| {
        capabilities.content.plain_text.is_supported()
    })
    .add(command("remind").handle(|| async { "saved" }));

let app = OxideBot::new().plugin(reminders);
```

Use a plain `Module` for a stateless handler collection. Use a bundle when the
feature owns configuration, metadata, or one or more `Service` tasks.
`require_capability` makes a portable prerequisite explicit and rejects an
application at build time when any registered adapter cannot meet it.

## Runtime performance model

The smaller authoring API still compiles into the existing bounded runtime:

- a 52-slot dense event table;
- exact command, interaction, and native-key indexes;
- pre-decode interest filtering;
- one shared `Arc<Event>` payload per admitted event;
- candidate merging in registration order without a temporary candidate list;
- synchronous, on-demand extraction after matching;
- bounded ingress, executor, session, and API command queues;
- scheduler-backed send/edit/delete/reaction/interaction/publication calls with
  byte admission, priority reserves, deadlines, safe idempotent retries,
  rate-limit cooldowns, and panic isolation;
- observable queue depth/high-water marks, queue and handler latency totals,
  retries, delivery failures/degradations, active handlers, and active sessions;
- per-conversation virtual-actor ordering;
- broad message matching only for commands that request custom prefixes,
  no-prefix matching, case-insensitive matching, or application-wide message
  normalization/rewriting; static and runtime shortcuts otherwise select only
  their owning command IDs before parsing.

## Workspace

- `oxidebot-core`: canonical events, messages, content models, and adapter API;
- `oxidebot-macros`: command, branch, state, and typed dialogue-form macros;
- `oxidebot-runtime`: modules, extractors, commands, compiled dispatch, bounded
  queues, sessions, and supervision;
- `oxidebot`: batteries-included facade and prelude;
- `oxidebot-testkit`: concise `BotTest`/`CommandTest` APIs plus lower-level deterministic fixtures;
- `oxidebot-adapter-console`: a real stdin/stdout adapter for local development;
- `examples/minimal`: a finite scripted example;
- `examples/console-bot`: an immediately runnable interactive bot.

Additional documentation:

- [Module and handler model](docs/MODULES.md)
- [Command system](docs/COMMANDS.md)
- [Runtime architecture](ARCHITECTURE.md)
- [Ergonomic authoring guide](docs/ERGONOMIC_AUTHORING.md)
