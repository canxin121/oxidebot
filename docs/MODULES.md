# Modules, handlers, and effects

OxideBot keeps the ergonomic parts of function-based frameworks while using
Bot-domain concepts throughout:

```text
Module -> match -> Context -> Extract -> async handler -> Outcome
```

There is one Bot-domain handler pipeline with no parallel request or routing
object model.

## Ordinary async handlers

A handler is an async function with zero to twelve typed arguments:

```rust
#[oxidebot::command("ping")]
async fn ping() -> &'static str {
    "pong"
}

async fn echo(Text(text): Text) -> Message {
    Message::text(text)
}

async fn inspect(
    Sender(user): Sender,
    MaybeConversation(conversation): MaybeConversation,
    State(state): State<AppState>,
) -> HandlerResult<String> {
    state
        .inspect(user.id, conversation)
        .await
        .internal("inspect account")
}
```

Arguments implement synchronous `Extract<S>`. Extraction happens after a
handler has matched and after module admission and command validation/completion.

## Handler kinds

```rust
let features = Module::new()
    .add(ping)
    .message(observe_message)
    .on(tags::FriendRequested, friend_request)
    .interaction("settings.save", save_settings)
    .native("vendor.event", native_event);
```

`on(tag, handler)` uses the exact canonical `EventTag` and the dense event
index. `message` is a convenience for `on(tags::Message, ...)`. Commands,
interactions, and native identifiers use their exact compiled indexes whenever
possible.

## Natural blocking defaults

Routing flow belongs to the matched handler kind, not the Rust return type:

| Handler kind | Default |
| --- | --- |
| command | stop after the handler |
| interaction | stop after the handler |
| ordinary event observer | continue |
| native event observer | continue |

Therefore both of these command handlers block later matching handlers:

```rust
async fn send_with_return() -> &'static str {
    "done"
}

async fn send_immediately(messenger: Messenger) -> HandlerResult<()> {
    messenger.send("done").await?;
    Ok(())
}
```

Likewise, returning a message from an ordinary event observer replies but still
uses the observer's continue default.

Override the default only when needed:

```rust
async fn auditing_observer() -> Outcome {
    Outcome::continue_().text("observed")
}

async fn consuming_observer() -> Outcome {
    Outcome::stop()
}
```

`Outcome::new()` inherits the natural default. It may contain zero or more
canonical messages:

```rust
Outcome::new()
    .reply("first")
    .reply(Message::text("second"))
```

## Context and extraction

`Context<S>` is the single clone-cheap runtime view. It contains shared handles
to the one `DispatchEnvelope`, root `Arc<S>`, current bot, bounded session
registry, shutdown signal, and optional lossless command match.

Common extractors are intentionally domain-specific:

```rust
Text
Segments
Sender
Conversation
MaybeConversation
MessageId
Target
State<S>
Bot
Reply
Dialogue
Args<T>
CommandMatch
EventContext<Tag>
BotIdentity
EventId
ShutdownSignal
Context<S>
```

There is no dynamic `Extensions` map. A handler's dependencies remain visible
in its function signature and root state. Synchronous derived values can use a
custom extractor:

```rust
struct Tenant(TenantId);

impl Extract<AppState> for Tenant {
    fn extract(context: &Context<AppState>) -> Result<Self, ExtractError> {
        context
            .state()
            .tenant_for(context.bot_identity())
            .map(Self)
            .ok_or_else(|| ExtractError::new("no tenant is configured for this bot"))
    }
}
```

Asynchronous authorization or lookup belongs in a guard or in the handler's
business logic, rather than a hidden asynchronous argument pipeline.

## One immediate-send facade

Returning `Message` is the simplest reply path. When a handler needs an
immediate receipt, proactive target, or progress edit, request `Messenger`:

```rust
use oxidebot::delivery::prelude::Address;

async fn deploy(messenger: Messenger) -> HandlerResult<()> {
    let progress = messenger.reply("Deploying...").await?;
    progress.edit("Deployment complete").await?;

    messenger
        .to(Address::group("operations"))?
        .send("A deployment completed")
        .await?;
    Ok(())
}
```

`Messenger` unifies current-target send/reply, explicit `Address` delivery,
fallback policy, deterministic bot selection, delivery middleware, and
`Receipt`. `Bot` remains the full low-level API escape hatch.

## Typed full-event access

```rust
async fn joined(
    event: EventContext<tags::GroupMemberJoined>,
) -> Message {
    Message::text("Welcome ")
        .at(event.user.id.clone())
        .then(format!(" to conversation {}", event.conversation.id))
}
```

`EventContext<Tag>` dereferences to `Tag::Event` and exposes event identity,
bot identity, the complete bot API, and shutdown. It does not copy the event
payload. Ask for `State<S>` or `Context<S>` separately when state is needed.

## Flat composition

```rust
fn todo_module() -> Module<AppState> {
    Module::new()
        .add(command("todo add").handle(add_todo))
        .add(command("todo list").handle(list_todos))
}

fn admin_module() -> Module<AppState> {
    Module::new().add(
        command("admin stats")
            .handle(admin_stats)
            .guard(admin_only),
    )
}

let application = Module::new()
    .include(todo_module())
    .include(admin_module())
    .on(tags::GroupMemberJoined, joined)
    .help();
```

`include` is a flat, ordered composition operation. It does not:

- prefix commands;
- create parent and child paths;
- alter event handlers;
- introduce a plugin object or lifecycle;
- create another state, runtime, scheduler, or event bus.

A reusable package exports an ordinary function returning `Module<S>`. No
separate plugin abstraction is introduced unless a package genuinely needs
additional service lifecycle or configuration concepts.

## Guards

Guards are async admission rules for Bot concerns:

```rust
async fn group_admin(context: Context<AppState>) -> GuardDecision {
    let Some(message) = context.message() else {
        return GuardDecision::skip();
    };

    if context.state().is_group_admin(&message.sender.id) {
        GuardDecision::allow()
    } else {
        GuardDecision::deny("This command requires a group administrator.")
    }
}

let admin = Module::new().add(
    command("admin purge")
        .handle(purge)
        .guard(group_admin),
);
```

Decisions are:

- `Allow`: continue into completion and extraction;
- `Skip`: leave the event available for later matching handlers;
- `Deny(Outcome)`: stop with explicit effects.

A guard runs before interactive command completion, so denied users are not
asked to provide missing arguments. Infallible guards may return
`GuardDecision` directly; fallible guards may return `GuardResult`.

## Before and after hooks

Hooks cover observation and effect transformation without a Web middleware
continuation object:

```rust
async fn record_start(context: Context<AppState>) {
    tracing::debug!(event = ?context.event_type(), "start");
}

async fn record_finish(
    context: Context<AppState>,
    outcome: Outcome,
) -> Outcome {
    tracing::debug!(
        event = ?context.event_type(),
        replies = outcome.replies().len(),
        "finish",
    );
    outcome
}

let features = todo_module()
    .before(record_start)
    .after(record_finish);
```

A `before` hook runs after command validation/completion and before handler
extraction. An `after` hook runs after the handler and can inspect or replace its
`Outcome`.
Infallible hooks return `()` or `Outcome` directly; fallible hooks return
`HandlerResult<()>` or `HandlerResult<Outcome>`.

Module guards and hooks apply to all direct and included handlers regardless of
method-call order. Child guards and before hooks run inside parent guards and
before hooks; child after hooks run before parent after hooks. This order is
fixed by structure.

## Platform and bot scope

A module may be limited explicitly:

```rust
let telegram = telegram_features().for_platform(telegram_platform);
let primary = private_features().for_bot(primary_bot_identity);
```

The restriction applies to every current and future handler in that module,
regardless of declaration order. Scope composition is an intersection rather
than an overwrite: a parent platform restriction never broadens a child that
is already limited to one bot. Incompatible platform or bot restrictions fail
the application build with an explicit route error.

## State

`#[derive(BotState)]` generates static, monomorphized projections for selected
services. There is no runtime type map:

```rust
#[derive(BotState)]
struct AppState {
    #[state]
    database: Database,

    #[state]
    projects: std::sync::Arc<ProjectStore>,
}

async fn handler(
    State(database): State<Database>,
    State(projects): State<ProjectStore>,
) {
    database.query().await;
    projects.refresh().await;
}
```

Requesting `State<AppState>` still exposes the root `Arc<AppState>`. A custom
`Extract<AppState>` remains appropriate for event-derived values rather than
state fields.

## Error semantics

A fallible handler uses an error that explicitly converts to `HandlerError`:

```rust
async fn handler() -> HandlerResult<String> {
    Err(HandlerError::user("Please select a project first."))
}
```

Only `HandlerError::User` becomes a reply. Internal command, session, API,
application, parse, timeout, and service failures are logged and do not leak
their `Display` text to users. User-facing and extractor errors inherit the matched handler
kind's normal propagation policy: failed commands and interactions block, while
failed observers leave later observers available. OxideBot deliberately has no
blanket `Result<T, E: Display>` reply conversion.

## Frozen authoring phases

Module guards and before/after hooks remain the normal endpoint phases. The
application builder also accepts narrow cross-cutting authoring phases:

```rust
OxideBot::with_state(state)
    .message_normalizer(normalize_message)
    .command_rewriter(rewrite_natural_language)
    .command_middleware(audit_parsed_command)
    .command_output_middleware(theme_help)
    .delivery_middleware(inspect_delivery)
    .command_renderer(render_commands)
    .locale_resolver(resolve_locale);
```

Async functions implement these traits through ownership-based blanket
implementations. The lists are frozen by `build`; there is no global extension
registry or load-order-dependent mutation after startup.

Use the narrowest phase:

- `MessageNormalizer` for canonical inbound-message cleanup;
- `CommandRewriter` for shortcut or natural-language input translation;
- `CommandMiddleware` for an already parsed `CommandMatch`;
- `CommandOutputMiddleware` for structured help, error, or completion output;
- `DeliveryMiddleware` for a bounded, capability-checked `DeliveryPlan`.

Endpoint-specific permission and rate-limit checks remain module guards.

## Standard modules

The built-in conveniences are ordinary modules and share the same state,
command IR, scheduler, and delivery pipeline:

```rust
use oxidebot::standard::{echo_module, language_module, AdminTools};

let features = Module::new()
    .include(echo_module())
    .include(language_module())
    .include(
        AdminTools::new()
            .prefix("oxidebot")
            .protected(admin_only),
    )
    .help();
```

`AdminTools` can independently enable command management, shortcut management,
and diagnostics. Its commands remain an ordinary protected `Module`.
