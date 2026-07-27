# OxideBot

OxideBot is a platform-neutral asynchronous chatbot runtime for Rust. The 1.0 rewrite keeps the familiar chainable application style while replacing the old broadcast-and-spawn execution model with a bounded, indexed, conversation-aware kernel.

> `1.0.0-alpha.1` is intentionally incompatible with OxideBot 0.1. The version is unchanged in this optimized source snapshot.

## Design goals

- **Decode only interested traffic.** Adapters extract a small `EventIndex` before full decoding. Frames that cannot reach a global, platform-scoped, bot-scoped route or an active session are discarded early.
- **Do not scan every handler.** Routes are compiled into dense event-kind tables and exact command, interaction, and native-event indexes.
- **Keep memory bounded.** Ingress, executor, command, session, and dedupe state all have explicit item and retained-byte ceilings.
- **Keep task counts bounded.** Fixed executor shards drive virtual conversation actors. Events do not create detached Tokio tasks.
- **Preserve useful ordering.** A conversation or `(conversation, actor)` key is serialized while unrelated keys execute concurrently.
- **Prevent noisy neighbours.** Dispatcher, executor, and command paths enforce per-bot fairness together with process-wide and per-bot budgets and concurrency limits.
- **Make shutdown structural.** Adapters, services, sessions, executor shards, command schedulers, and the dispatcher are supervised and drained against one shutdown deadline.
- **Keep the model free of I/O.** Core message and media values describe data; adapters or services perform network and filesystem work.

## Workspace

| Crate | Purpose |
| --- | --- |
| `oxidebot-core` | Validated IDs, canonical events, portable messages, raw/native escape hatches, retained-size accounting |
| `oxidebot-runtime` | Adapter contracts, compiled routing, exact sessions, bounded executors, command scheduling, metrics, supervision |
| `oxidebot` | Convenience facade re-exporting the core and runtime APIs |
| `oxidebot-testkit` | Deterministic adapter/service fixtures and routing benchmarks |

Most applications only depend on the facade:

```toml
[dependencies]
oxidebot = "1.0.0-alpha.1"
```

## Application style

```rust,ignore
use oxidebot::{message, on, Context, MessageCreated, Outcome, OxideBot};

#[tokio::main]
async fn main() -> oxidebot::Result<()> {
    let adapter = build_my_adapter();

    OxideBot::new()
        .bot(adapter)
        .filter(|event, _state| !event.id.as_str().starts_with("ignored:"))
        .handler(on(
            message().command("ping"),
            |_context: Context<MessageCreated>| async move {
                Ok(Outcome::stop().reply("pong"))
            },
        ))
        .run()
        .await
}
```

The surface remains close to the original framework:

```text
OxideBot::new()
    .bot(...)
    .filter(...)
    .handler(...)
    .service(...)
    .run()
```

Internally, however, an exact `/ping` route is reached through the command index. Unrelated handlers are neither scanned nor instantiated as futures.

Routes may also be restricted to one platform or one exact bot. The scope is compiled into both the adapter interest plan and the runtime route table, so an unrelated adapter does not fully decode a command merely because another bot registered the same command:

```rust,ignore
use oxidebot::{BotIdentity, BotId, PlatformId};

let telegram = PlatformId::new("telegram")?;
let production = BotIdentity::new(
    telegram.clone(),
    BotId::new("production-bot")?,
);

OxideBot::new()
    .bot(adapter)
    .handler(on(message().command("admin"), admin).platform(telegram))
    .handler(on(message().command("status"), status).bot(production));
```

Build validation rejects a route scoped to a platform or bot that is not registered in the application.

## Typed context without event cloning

`Context<E, S>` owns an `Arc<EventEnvelope>` and exposes `E` as a borrowed view of the single canonical event body:

```rust,ignore
.handler(on(message(), |context: Context<MessageCreated>| async move {
    if let Some(text) = context.text() {
        context.reply(format!("received: {text}")).await?;
    }
    Ok(Outcome::continue_())
}))
```

`Context` is intentionally not `Clone`. This makes retaining an event beyond a handler an explicit application decision rather than an accidental way to escape runtime memory accounting.

## Dialogue sessions

A one-shot session is registered before its prompt is sent, preventing a fast reply from racing registration:

```rust,ignore
use std::time::Duration;
use oxidebot::{message, on, AskOptions, Context, MessageCreated, Outcome};

on(message().command("age"), |context: Context<MessageCreated>| async move {
    let age: u8 = context
        .ask_parse(
            "How old are you?",
            AskOptions::new(Duration::from_secs(30)),
        )
        .await?;

    Ok(Outcome::stop().reply(format!("age={age}")))
})
```

Sessions use an exact `(bot, conversation/thread, actor)` lookup. When no exact session is active, ordinary messages do not allocate a session command or a oneshot channel. Dropping a waiter synchronously marks it cancelled, so a failed best-effort cleanup command cannot cause a later message to be consumed accidentally.

A scope supports one exclusive waiter. `SessionNamespace` identifies ownership and safe cancellation; it does not multiplex several consumers of the same “next message.” Use `SessionPolicy::Tap` when the response should also continue through normal routing.

For long-lived or high-volume workflows, prefer a persistent dialogue state machine in application state instead of holding one handler future open for minutes. `RuntimeConfig::message_execution_partition` can be changed to `ConversationActor` when independent users in one group should proceed concurrently.

## Adapter contract

An adapter provides immutable identity, outbound services, and an inbound transport:

```rust,ignore
use async_trait::async_trait;
use oxidebot::{Adapter, AdapterContext, AdapterError, AdapterMode, BotDescriptor, BotServices};

#[async_trait]
impl Adapter for MyAdapter {
    fn descriptor(&self) -> BotDescriptor {
        self.descriptor.clone()
    }

    fn services(&self) -> BotServices {
        BotServices::messages(self.messages.clone())
    }

    fn mode(&self) -> AdapterMode {
        AdapterMode::Persistent
    }

    async fn run(mut self: Box<Self>, context: AdapterContext) -> Result<(), AdapterError> {
        loop {
            tokio::select! {
                _ = context.shutdown().cancelled() => {
                    return Err(AdapterError::cancelled("adapter stopped"));
                }
                frame = self.transport.next_frame() => {
                    context.submit(frame?).await?;
                }
            }
        }
    }
}
```

Each frame has two stages:

1. `InboundFrame::index` extracts only fields needed by interest routing and provides a conservative decoded-byte upper bound.
2. `InboundFrame::decode_indexed` creates one canonical `EventBatch` after admission and can reuse the validated index-stage work; the simpler `decode` method remains the default fallback.

The runtime validates the complete decoded body against the pre-decode index, including bot ownership, platform ownership, conversation, actor, command, interaction ID, native event type, nested message references, mentions, rich-text ranges, metadata bounds, and native data.

One raw payload belongs to the frame and is retained once even when the frame expands into several canonical events. `RuntimeConfig::max_frame_bytes`, `max_frame_events`, and `max_event_bytes` reject pathological frame size, expansion, and per-event retained state before it reaches the executor.

Arbitrary `bytes::Bytes` views are represented by `RetainedBytes`. The default constructor compacts a slice to its visible range; adapters that intentionally preserve a larger shared backing allocation must provide an explicit retained-memory charge with `RetainedBytes::with_charge`.

## Outbound services and command scheduling

Portable messaging, interaction responses, and native calls are separate service contracts:

```text
MessageService
InteractionService
NativeService
```

Each bot gets a keyed command scheduler with:

- per-conversation message ordering;
- per-interaction response ordering;
- weighted high-priority scheduling for interaction acknowledgements;
- queue and service-capacity reservations that normal traffic cannot consume;
- per-bot and process-wide queue budgets;
- per-bot and process-wide in-flight limits;
- attempt and total deadlines;
- bounded exponential backoff with deterministic jitter;
- per-bot rate-limit cooldowns, while global service permits are released between retry attempts;
- panic isolation;
- early removal of abandoned request/response commands.

Automatic send retries occur only when an idempotency key is present **and** the adapter declares `AdapterEmulated` or `PlatformNative` idempotency. Delete retries require an explicit adapter guarantee; a subsequent platform `NotFound` is then treated as successful completion.

## Runtime configuration

`RuntimeProfile::{Eco, Balanced, Throughput}` supplies complete starting envelopes. Every field remains adjustable through `RuntimeConfig`, including:

- global and per-bot ingress item/byte budgets plus a maximum frame size;
- maximum events per frame and maximum incremental bytes per event;
- global and per-bot executor budgets;
- global and per-bot command budgets plus a maximum command size;
- global and per-bot in-flight limits;
- executor and command overload policies;
- message ordering partition;
- session capacity and shard command capacity;
- dedupe item, byte, and TTL bounds;
- handler timeout;
- per-handler deferred-reply fan-out;
- command attempt timeout and total deadline;
- retry count, base delay, maximum delay, and high-priority burst;
- one absolute shutdown grace period.

`OverloadPolicy::Block` waits for ordinary capacity and never reports an event as accepted when it cannot fit the configured executor envelope. `DropNewest` performs an explicit drop-and-ack shed and records it in metrics. Oversized frames, events, and commands are rejected as errors rather than being silently converted into loss.

A handler returning more than `RuntimeConfig::max_handler_replies` deferred replies has its entire reply batch rejected, preserving deterministic command-queue bounds and avoiding partial side effects. Direct `Context::reply` calls remain explicit synchronous command operations.

The dispatcher keeps one admission future per bot and advances ready bots round-robin. A bot blocked on its local executor budget or an active session cannot head-of-line block unrelated bots. Event IDs enter the dedupe history only after a session consumes the event, the executor accepts it, or `DropNewest` intentionally sheds it.

## Low-power deployment

The runtime crate separates Tokio features:

```toml
oxidebot-runtime = { version = "1.0.0-alpha.1", default-features = false }
```

- enable `signal` when `Application::run()` should listen for Ctrl-C;
- enable `multi-thread` for Tokio's multi-thread scheduler;
- use a current-thread runtime with the Eco profile for small, mostly idle bots.

The core contains no polling loop. When there is no network activity, timer deadline, retry, or queued work, workers sleep on channels or timers.

## Panic strategy

The default release profile uses unwinding so handler and platform-service panics can be isolated with `catch_unwind`.

A separate `release-small` profile uses `panic = "abort"` for deployments that explicitly prefer a smaller binary and accept process termination on panic:

```text
cargo build --profile release-small
```

## Observability

`RuntimeMetrics` reports ingress, ignored frames, decoded and duplicate events, uncacheable dedupe IDs, validation failures, dispatched, dropped, and rejected events, session fast misses and consumption, route candidate and handler counts, panics, timeouts, rejected handler-effect batches, commands, abandoned commands, and command errors.

Counters are cache-line isolated to avoid false sharing between ingress, executor, and command workers.

## Repository layout

```text
oxidebot/
├── crates/
│   ├── oxidebot-core/
│   ├── oxidebot-runtime/
│   ├── oxidebot/
│   └── oxidebot-testkit/
├── examples/
│   └── minimal/
├── ARCHITECTURE.md
├── Cargo.toml
└── README.md
```

## Development commands

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo doc --workspace --no-deps
cargo bench -p oxidebot-testkit --bench router
```

## License

Licensed under either of:

- Apache License, Version 2.0 (`LICENSE-Apache-2.0.txt`)
- MIT License (`LICENSE-MIT.txt`)
