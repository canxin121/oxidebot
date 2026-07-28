# OxideBot

OxideBot is a high-performance, platform-neutral bot framework built around one
complete semantic model: the restored **0.1.8 `Event` hierarchy**, one extended
cross-platform `Message` / `MessageSegment` intermediate representation, and
the full `CallApiTrait` API. The stable 0.1.8 message variants remain available,
but rich text, media, components, polls, layouts, and native segments now use
the same message value instead of parallel models.

The authoring layer keeps the parts of Axum that are genuinely useful outside
HTTP—ordinary async functions, typed extraction, composable application
modules, and explicit effects—without importing Web-only concepts such as
requests, responses, URL trees, nested routes, Tower layers, request
extensions, or middleware continuation objects.

The application model is deliberately small:

```text
Adapter -> Event -> Module match -> typed Extract -> async fn -> Outcome / API calls
```

There is no reduced “simple event”, no `RichEvent`, no second API, and no
compatibility event bus. Every handler reads the same shared 0.1.8 event that
adapters decode.

## A complete small bot

```rust
use oxidebot::prelude::*;

async fn ping() -> &'static str {
    "pong"
}

#[derive(Debug, CommandArgs)]
#[command(
    description = "Search the knowledge base",
    category = "knowledge",
    alias = "find"
)]
struct SearchArgs {
    /// What to search for.
    #[arg(prompt = "What should I search for?")]
    query: String,

    /// Maximum number of results.
    #[arg(long, short = 'n', default = 10_u32)]
    limit: u32,

    /// Include diagnostic details.
    #[arg(long, short = 'v')]
    verbose: bool,

    /// Optional labels after the query.
    #[arg(rest)]
    labels: Vec<String>,
}

#[derive(Clone, Default)]
struct AppState;

impl AppState {
    async fn search(
        &self,
        user_id: String,
        query: String,
        limit: u32,
        verbose: bool,
        labels: Vec<String>,
    ) -> String {
        format!(
            "user={user_id} query={query} limit={limit} verbose={verbose} labels={labels:?}",
        )
    }
}

async fn search(
    Args(args): Args<SearchArgs>,
    State(state): State<AppState>,
    Sender(user): Sender,
) -> String {
    state
        .search(user.id, args.query, args.limit, args.verbose, args.labels)
        .await
}

fn features() -> Module<AppState> {
    Module::new()
        .command(
            command("ping").description("Check whether the bot is alive"),
            ping,
        )
        .command(SearchArgs::command("search"), search)
        .help()
}

OxideBot::with_state(AppState)
    .adapter(adapter)
    .include(features())
    .run()
    .await?;
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
    MaybeGroup(group): MaybeGroup,
    Text(text): Text,
) -> Message {
    let location = group
        .as_ref()
        .map(|group| format!("group {}", group.id))
        .unwrap_or_else(|| "private chat".to_owned());

    Message::text("Hello ")
        .at(user.id)
        .then(format!(" in {location}. You said: {text}"))
}
```

Built-in extractors include:

- `Text`, `Segments`, `Sender`, `ChatGroup`, `MaybeGroup`, `MessageId`, and
  `Target`;
- root `State<S>` and the common `Context<S>`;
- typed command `Args<T>` and raw `CommandResult`;
- `Bot`, `Reply`, and `Dialogue`;
- `EventContext<tags::...>`, `BotIdentity`, `EventId`, and `ShutdownSignal`.

Extraction is synchronous, monomorphized, and runs only after a handler has
matched and command validation/completion has succeeded. There is no
asynchronous `FromRequest`, dynamic extension map, or substate conversion
registry in the hot path. A custom domain extractor is a small implementation
of `Extract<S>`:

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
`MaybeGroup`. A generic `Option<T>` extractor is intentionally absent because
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

let deploy_command = DeployArgs::command("deploy").completion(
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

let module = Module::new().command(TodoCommand::command(), todo).help();
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
        .command(command("account show"), show_account)
        .command(command("account delete"), delete_account)
}

fn moderation_module() -> Module<AppState> {
    Module::new()
        .command(command("admin ban"), ban_member)
        .command(command("admin mute"), mute_member)
        .guard(admin_only)
}

let features = Module::new()
    .include(account_module())
    .include(moderation_module())
    .on(tags::GroupMemberIncrease, welcome_member)
    .interaction("settings.save", save_settings)
    .native("vendor.special_event", native_event)
    .after(trace_outcome)
    .help();
```

`include` preserves registration order and flattens behavior into the same
compiled route tables. It does not add a command prefix, create nesting, start a
plugin runtime, or introduce another event bus. Multi-word command paths are
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

There is no `Request`, `Response`, `Next`, `from_fn`, `layer`, or
`route_layer`. Bot concerns are expressed directly:

- `guard` for permission, rate-limit, chat-type, and feature admission;
- `before` for pre-handler observation or preparation;
- `after` for observing or transforming produced effects.

Parent module guards and hooks apply to included modules regardless of whether
they are declared before or after `include`, eliminating Tower-style ordering
traps. See [docs/MODULES.md](docs/MODULES.md).

## One message IR, delivery planning, and receipts

All inbound events, handler results, active sends, command tokenization, and
adapter export use the same `Message`. Stable 0.1.8 constructors remain the
shortest path, while portable rich segments live in the same enum:

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

Rich and interactive content uses the same value:

```rust
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

Returning a message is the shortest path. Use `Reply` when the workflow needs
the platform result immediately:

```rust
async fn generate_report(reply: Reply) -> HandlerResult<()> {
    let progress = reply.send("Generating report…").await?;
    create_report().await.map_err(|error| HandlerError::internal(error.to_string()))?;
    progress.edit("Report complete").await?;
    progress.react("✅").await?;
    Ok(())
}
```

A `Receipt` is produced only by a send operation. It tracks every physical
message ID returned for one logical send and can edit, delete, react to, or
delay-delete all of them. It is not used as an ambiguous handle for the
incoming message.

## Bounded dialogues

```rust
async fn setup(dialogue: Dialogue) -> HandlerResult<String> {
    let name = dialogue.ask_text("Project name?").await?;
    let workers: usize = dialogue.ask("Worker count?").await?;
    let enabled = dialogue.confirm("Enable it now?").await?;

    Ok(format!("{name}: workers={workers}, enabled={enabled}"))
}
```

`Dialogue` uses the existing bounded session registry and exact bot,
conversation, actor, and namespace keys. It supports custom timeouts,
consume-versus-tap policy, typed parsing, confirmation, choices, and complete
reply events.

## Complete event and API access

Typed handlers can use a concrete view of the original event allocation:

```rust
async fn joined(
    event: EventContext<tags::GroupMemberIncrease>,
    bot: Bot,
) -> HandlerResult<()> {
    bot.send_message(
        vec![MessageSegment::text(format!("Welcome {}", event.user.id))],
        oxidebot::api::payload::SendMessageTarget::Group(event.group.id.clone()),
    )
    .await
    .map(|_| ())
    .map_err(|error| HandlerError::Api(error.to_string()))
}
```

`EventContext<Tag>` dereferences to the concrete tagged 0.1.8 payload and
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

Extractor failures are also safe, user-facing errors because they describe a
handler contract such as “this command requires a group chat” or invalid typed
arguments. Their replies inherit the matched handler kind's normal propagation
policy.

## Runtime performance model

The smaller authoring API still compiles into the existing bounded runtime:

- a stable 53-slot dense event table;
- exact command, interaction, and native-key indexes;
- pre-decode interest filtering;
- one shared `Arc<Event>` payload per admitted event;
- candidate merging in registration order without a temporary candidate list;
- synchronous, on-demand extraction after matching;
- bounded ingress, executor, session, and API command queues;
- per-conversation virtual-actor ordering;
- broad message matching only for commands that request custom prefixes,
  no-prefix matching, or case-insensitive matching.

## Workspace

- `oxidebot-core`: complete 0.1.8 events, messages, content models, and API;
- `oxidebot-macros`: `CommandArgs` derive;
- `oxidebot-runtime`: modules, extractors, commands, compiled dispatch, bounded
  queues, sessions, and supervision;
- `oxidebot`: batteries-included facade and prelude;
- `oxidebot-testkit`: deterministic adapter and API fixtures;
- `examples/minimal`: a finite runnable example.

Additional documentation:

- [Module and handler model](docs/MODULES.md)
- [Command system](docs/COMMANDS.md)
- [Migration from the Web-shaped draft API](docs/MIGRATION.md)
- [Runtime architecture](ARCHITECTURE.md)
