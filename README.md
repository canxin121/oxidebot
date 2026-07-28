# OxideBot

OxideBot keeps the complete **0.1.8 `Event` hierarchy and `CallApiTrait` API**,
but gives bot authors a new, unified application interface inspired by Axum:
ordinary async functions, extractors, a compositional `Router`, typed command
arguments, middleware, automatic help, bounded dialogues, and direct access to
the original platform-neutral model whenever it is needed.

There is no “simple event” versus “rich event”, and no second API hidden behind
a compatibility namespace. The ergonomic layer routes and extracts the same
`Event`, `Message`, `MessageSegment`, and `CallApiTrait` values used by adapters.

## A complete small bot

```rust
use oxidebot::prelude::*;

async fn ping() -> &'static str {
    "pong"
}

#[derive(CommandArgs)]
#[command(description = "Search the knowledge base", alias = "find")]
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

async fn search(
    Parsed(args): Parsed<SearchArgs>,
    State(state): State<AppState>,
    Sender(user): Sender,
) -> Result<String, SearchError> {
    state.search(user.id, args.query, args.limit, args.verbose, args.labels).await
}

let routes = Router::new()
    .command(command("ping").description("Check whether the bot is alive"), ping)
    .command(SearchArgs::command("search"), search)
    .help();

OxideBot::with_state(AppState::new())
    .bot(adapter)
    .router(routes)
    .run()
    .await?;
```

A handler may return `&str`, `String`, `Message`, `MessageSegment`,
`Vec<MessageSegment>`, `Option<T>`, `Result<T, E>`, `Response`, or `()`.
Message-like values reply to the current event and stop routing. `()` and
`None` continue routing. Explicit `Response` values can combine replies and
choose whether later routes should run.

## Extract only what a handler needs

Extractors are evaluated after a route matches. A handler that only needs text
does not construct command arguments, state projections, session objects, or
API wrappers.

```rust
async fn greet(
    Sender(user): Sender,
    MaybeGroup(group): MaybeGroup,
    Text(text): Text,
) -> Message {
    Message::text("Hello ")
        .at(user.id)
        .then(match group.0 {
            Some(group) => format!(" in group {}", group.id),
            None => " in private chat".to_owned(),
        })
        .then(format!(". You said: {text}"))
}
```

Built-in extractors include:

- `Text`, `Segments`, `Sender`, `ChatGroup`, `MaybeGroup`, `MessageId`, and
  `Target`;
- `State<T>` through `FromRef<AppState>` and middleware-provided
  `Extension<T>`;
- `Parsed<T>` and raw `CommandResult`;
- `Bot`, `Reply`, `Receipt`, and `Dialogue`;
- `EventContext<tags::...>`, `BotIdentity`, `EventId`, and `ShutdownSignal`;
- `Option<T>` for optional extraction.

Custom extractors implement one asynchronous `FromRequest<S>` trait.

## Typed commands instead of manual splitting

`#[derive(CommandArgs)]` produces the parser and the help schema from one Rust
struct. The same definition drives positional parameters, long and short
options, flags, aliases, usage text, validation, and interactive prompts.

```rust
#[derive(CommandArgs)]
#[command(
    description = "Deploy a service",
    category = "operations",
    alias = "ship"
)]
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

let deploy = DeployArgs::command("deploy").completion(
    CompletionConfig::new()
        .timeout(std::time::Duration::from_secs(60))
        .max_rounds(3)
        .cancel_words(["cancel", "stop", "取消"]),
);
```

The parser supports:

- aliases, subcommand paths, multiple prefixes, case-insensitive commands, and
  commands without a prefix;
- quoted text, escaped spaces, empty quoted values, `--long=value`, combined
  short flags, attached short-option values, and `--`;
- required, optional, defaulted, repeated, and rest arguments;
- negative numeric positional values;
- typed mentions, files, and original `MessageSegment` values instead of
  flattening every input into text;
- actionable unknown-option suggestions and command-specific usage output.

When completion is enabled, only missing required arguments enter the bounded
session system. Invalid values and unknown options fail immediately rather than
starting a misleading prompt loop. Answers to missing named options are filled
back into that option, not appended as unrelated positional text.

## One Router for the whole application

```rust
fn account_routes() -> Router<AppState> {
    Router::new()
        .command(command("account show"), show_account)
        .command(command("account delete"), delete_account)
}

let routes = Router::new()
    .merge(account_routes())
    .mount("admin", moderation_routes())
    .plugin(audit_plugin)
    .event(tags::GroupMemberIncrease, welcome_member)
    .interaction("settings.save", save_settings)
    .native("vendor.special_event", native_event)
    .layer(from_fn(tracing_middleware))
    .help();
```

A plugin is a reusable `Router<S>` or a function returning one. It does not
own a second event bus, scheduler, state container, command parser, or API
object. Mounted and merged routes use the same compiled route tables and
bounded runtime as routes declared in `main`.

The original typed-context API remains available through `Router::handler` or
`OxideBot::handler` for gradual migration and specialized low-level code.

## Middleware

Middleware receives the same request that extractors use and can inspect the
canonical event, inject typed extensions, short-circuit the endpoint, or alter
the final response.

```rust
async fn tracing_middleware(
    mut request: Request<AppState>,
    next: Next<AppState>,
) -> Response {
    let event_type = request.event_type();
    request.extensions_mut().insert(RequestStarted::now());

    let response = next.run(request).await;
    tracing::debug!(?event_type, stopped = response.is_stopped());
    response
}
```

`from_fn` accepts any middleware return type implementing `IntoResponse`, so a
middleware may return a string or message when it needs to reject a request.

## Messages are still the 0.1.8 messages

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

The fluent builder and `message!` macro only construct the canonical 0.1.8
`Message` and `MessageSegment` types. Adapters do not translate from another
message DSL.

## Immediate operations and receipts

Returning a message is the shortest reply path. Use `Reply` when a handler must
perform multiple operations immediately or needs the platform's send result.

```rust
async fn generate_report(reply: Reply) -> HandlerResult<()> {
    let progress = reply.send("Generating report…").await?;
    create_report().await?;
    progress.edit("Report complete").await?;
    progress.react("✅").await?;
    Ok(())
}
```

A logical send may produce several physical platform messages. `Receipt`
applies edit, delete, reaction, and delayed-delete operations to all returned
message IDs.

## Bounded dialogues

```rust
async fn setup(dialogue: Dialogue) -> HandlerResult<String> {
    let name = dialogue.ask_text("Project name?").await?;
    let workers: usize = dialogue.ask("Worker count?").await?;
    let enabled = dialogue.confirm("Enable it now?").await?;

    Ok(format!("{name}: workers={workers}, enabled={enabled}"))
}
```

`Dialogue` uses the existing bounded session registry and exact
conversation/actor keys. It supports custom timeouts, namespaces,
consume-versus-tap policy, typed parsing, confirmation, choices, and access to
the complete reply event.

## Full Event and API escape hatch

No ergonomic feature replaces the original model:

```rust
async fn joined(
    context: EventContext<tags::GroupMemberIncrease>,
    bot: Bot,
) -> HandlerResult<()> {
    let event = context.event();

    bot.send_message(
        vec![MessageSegment::text(format!("Welcome {}", event.user.id))],
        oxidebot::api::payload::SendMessageTarget::Group(event.group.id.clone()),
    )
    .await
    .map(|_| ())
    .map_err(|error| HandlerError::Api(error.to_string()))
}
```

`Bot` dereferences to the complete `CallApiTrait`, including the restored 0.1.8
high-level methods and `call_platform_api`. `EventContext<tags::...>` borrows a
typed view from the one shared event allocation. Notice and request tags expose
their concrete payload structs directly.

## Runtime performance model

The authoring API is dynamic only where the application asks for dynamism. The
runtime still keeps its compiled and bounded design:

- a stable 53-slot dense event table;
- exact command, interaction, and native-key indexes;
- pre-decode interest filtering;
- one shared `Arc<Event>` payload per admitted event;
- candidate merging in registration order without a temporary candidate list;
- on-demand extraction after matching;
- bounded ingress, executor, session, and API command queues;
- per-conversation virtual-actor ordering;
- broad message matching only for commands that request custom prefixes,
  no-prefix matching, or case-insensitive matching.

## Workspace

- `oxidebot-core`: complete 0.1.8 events, messages, content models, and API.
- `oxidebot-runtime`: Router, extractors, command parser, middleware, compiled
  routing, bounded queues, sessions, supervision, and metrics.
- `oxidebot-macros`: `CommandArgs` derive.
- `oxidebot`: batteries-included facade and prelude.
- `oxidebot-testkit`: deterministic adapters and API fixtures.
- `examples/minimal`: runnable Router/command example.

More detail is available in `docs/ROUTING.md`, `docs/COMMANDS.md`, and
`docs/MIGRATION.md`.

## License

Licensed under either Apache-2.0 or MIT, at your option.
