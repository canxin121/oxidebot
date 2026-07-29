# Ergonomic bot authoring

This guide documents the authoring API implemented by this repository. Every
example is also represented by a compiled workspace example or test; proposed
APIs belong in an RFC rather than this guide.

OxideBot keeps one event model, one message IR, one command IR, and one
runtime. The authoring layer provides compile-time adapters over those same
primitives; it is not a second framework.

## Function commands

`#[oxidebot::command]` turns an async function into a feature value. Add that
value to a `Module` or directly to `OxideBot`:

```rust
use oxidebot::prelude::*;

#[oxidebot::command("ping")]
/// Checks whether the bot is alive.
async fn ping() -> &'static str {
    "pong"
}

let features = Module::new().add(ping).help();
```

Command parameters use `#[arg(...)]`. Scalar values are parsed through
`FromStr`; `Vec<T>` is used for repeated or rest values. Structs derived with
`CommandArgs` are useful when several fields belong together:

```rust
use oxidebot::prelude::*;

#[derive(Debug, CommandArgs)]
struct EchoArgs {
    /// Text to echo. Quotes and escaped spaces are preserved.
    #[arg(rest, required = true, prompt = "What should I echo?")]
    text: Vec<String>,

    /// Number of copies.
    #[arg(long, short = 'n', default = 1_usize, min = 1.0, max = 10.0)]
    times: usize,
}

#[oxidebot::command("echo")]
async fn echo(args: EchoArgs) -> Message {
    Message::text(
        std::iter::repeat_n(args.text.join(" "), args.times)
            .collect::<Vec<_>>()
            .join("\n"),
    )
}
```

For subcommands, derive `BotCommand` on an enum and attach handlers with
`#[oxidebot::branch(...)]`. The complete compiled example is
[`examples/minimal`](../examples/minimal/src/main.rs).

## State, event data, and immediate sends

Handlers ask only for the values they use. Common extractors include
`State<T>`, `Sender`, `Conversation`, `Context<S>`, `MessageContext`, `Messenger`,
`Dialogue`, and `I18n`:

```rust
use oxidebot::prelude::*;

#[derive(Clone, Default)]
struct GreetingService;

#[derive(BotState)]
struct AppState {
    #[state]
    greetings: GreetingService,
}

#[oxidebot::command("hello")]
async fn hello(
    Sender(user): Sender,
    State(_service): State<GreetingService>,
    messenger: Messenger,
) -> HandlerResult<()> {
    messenger
        .reply(Message::text(format!("Hello, {}", user.id)))
        .await?;
    Ok(())
}
```

Returning `String`, `Message`, or `Outcome` is the shortest deferred-reply
path. Use `Messenger` when the handler needs a `Receipt` before it returns,
wants to quote the incoming message, or sends to another address. The raw
`Bot`/`CallApiTrait` extractor is the explicit platform escape hatch; calls
made through it do not gain authoring middleware.

## Completion

Register a function with `#[oxidebot::completer]` and reference it from an
argument. `CompletionInput::partial` is a public field:

```rust
use oxidebot::{
    commands::prelude::{CompletionInput, CompletionItem, CompletionKind},
    prelude::*,
};

#[oxidebot::completer]
async fn complete_words(
    _context: Context,
    input: CompletionInput,
) -> HandlerResult<Vec<CompletionItem>> {
    Ok(["hello", "oxidebot", "world"]
        .into_iter()
        .filter(|value| value.starts_with(input.partial.as_ref()))
        .take(input.limit)
        .map(|value| CompletionItem::new(value, CompletionKind::Choice, input.replace))
        .collect())
}

#[oxidebot::command("echo")]
async fn echo(
    #[arg(rest, required = true, complete = complete_words)] words: Vec<String>,
) -> String {
    words.join(" ")
}
```

## Dialogue forms

`DialogueForm` currently supports prompts, retry/error text, confirmation
fields, static choices, and synchronous validators:

```rust
use oxidebot::prelude::*;

fn non_empty(value: &str) -> Result<(), &'static str> {
    (!value.trim().is_empty())
        .then_some(())
        .ok_or("Project name cannot be empty.")
}

#[derive(DialogueForm)]
struct SetupForm {
    #[dialogue(
        prompt = "What is the project name?",
        attempts = 3,
        error = "Please enter a non-empty project name.",
        validate = non_empty
    )]
    name: String,

    #[dialogue(
        prompt = "Choose an environment:",
        choice = "Development=dev",
        choice = "Staging=staging",
        choice = "Production=production"
    )]
    environment: String,

    #[dialogue(prompt = "Create this project?", confirm)]
    confirmed: bool,
}

#[oxidebot::command("setup")]
async fn setup(dialogue: Dialogue) -> HandlerResult<Message> {
    let form = dialogue.named("project-setup").form::<SetupForm>().await?;
    Ok(Message::text(format!(
        "{} / {} / {}",
        form.name, form.environment, form.confirmed
    )))
}
```

Dynamic choices, conditional fields, and nested forms are not part of the
current `DialogueForm` derive.

## Interactions

Register an exact interaction action ID with `Module::interaction`:

```rust
use oxidebot::prelude::*;

let interactions = Module::new().interaction("deploy.confirm", || async {
    Message::text("Deployment confirmed")
});
```

For explicit lifecycle control, extract `Responder`. All clones and automatic
returned messages share one acknowledgement state, so only one initial
response wins. Near a platform deadline, a still-pending interaction is
automatically deferred through the high-priority scheduler capacity:

```rust
use oxidebot::prelude::*;

async fn confirm(responder: Responder) -> HandlerResult<()> {
    responder.defer().await?;
    responder.edit_original("Deployment started").await?;
    responder.follow_up("Tracking is available in /status").await?;
    Ok(())
}
```

`Responder` supports `acknowledge`, `defer`, `respond`, `update`,
`edit_original`, `follow_up`, and `form_error`. Interactions without a response
handle cannot extract `Responder`; use the raw platform API only when the
portable lifecycle does not represent a platform-specific operation.

## Delivery policy and receipts

Messages are planned against adapter capabilities. `FallbackPolicy::Auto` is
the default. Use `Messenger::fallback` for strict or explicitly lossy
delivery:

```rust
use oxidebot::prelude::*;

async fn announce(messenger: Messenger) -> HandlerResult<()> {
    let receipt = messenger
        .clone()
        .fallback(FallbackPolicy::Strict)
        .send(Message::text("Deployment started"))
        .await?;
    receipt.edit("Deployment complete").await?;
    Ok(())
}
```

A logical message can become several physical messages. Use
`Receipt::references`, `Receipt::ids`, and `Receipt::report` when physical
message identity matters. `DeliveryReport::items` and
`PartialDeliveryError::report` retain successful physical message references
when a later send fails. Receipt bulk operations have `edit_report`,
`delete_report`, and `react_report` variants for the same partial-success
model.

Framework sends pass through the bounded per-bot scheduler. Queue budgets,
priority reserves, attempt/total timeouts, safe idempotent retries, rate-limit
cooldowns, panic isolation, and outbound metrics therefore apply consistently
to returned messages, `Messenger`, dialogues, cross-bot sends, receipts,
interactions, and command-definition publication. Raw `CallApiTrait` calls are
the intentional escape hatch and bypass authoring middleware.

## Runtime lifecycle

Install adapters and features on `OxideBot`. `Module::help()` installs the
built-in help command. Long-running background work implements `Service`, so
the runtime can supervise it and coordinate shutdown:

```rust,no_run
use oxidebot::prelude::*;

# async fn build(adapter: impl oxidebot::Adapter) -> oxidebot::Result<()> {
OxideBot::new()
    .adapter(adapter)
    .include(Module::new().help())
    .run()
    .await
# }
```

Use `run_to_completion()` for finite scripted adapters and tests.

## Tests and benchmarks

The high-level `BotTest` API drives ordinary messages and answerable button
interactions:

```rust
use oxidebot::prelude::*;
use oxidebot_testkit::BotTest;

#[oxidebot::command("ping")]
async fn ping() -> &'static str {
    "pong"
}

# async fn scenario() -> oxidebot::Result<()> {
BotTest::feature(ping)
    .message("/ping")
    .expect_reply("pong")
    .run()
    .await?;
# Ok(())
# }
```

Interaction scenarios use `.click(...)`, `.expect_interaction_ack()`,
`.expect_edit_contains(...)`, and `.expect_followup_contains(...)`.

Use `ScriptedAdapter` directly for protocol-level event control. Criterion is
used only by `oxidebot-testkit` benchmarks. If Gnuplot is not installed,
Criterion automatically uses its Plotters backend; that informational message
does not affect tests or runtime behavior.
