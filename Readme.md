# OxideBot

OxideBot is a platform-neutral, asynchronous chatbot framework for Rust. The 1.0 rewrite replaces the original inheritance-style framework with a small canonical data model, explicit adapter contracts, indexed routing, bounded queues, and structured shutdown.

> `1.0.0-alpha.1` is a deliberately incompatible preview. Existing 0.1 adapters and handlers must be ported to the new contracts.

## What changed in 1.0

- Incoming frames are indexed before they are fully decoded. Adapters can skip expensive decoding when no route or active session is interested.
- Every queue is bounded by both item count and estimated retained bytes. Overload behavior is explicit: backpressure or observable `DropNewest` shedding.
- Events from one conversation execute in order, while unrelated conversations can run concurrently.
- Platform transports, portable messaging, interactions, and native APIs are separate contracts instead of one large bot trait.
- One-shot dialogue sessions use exact `(conversation, actor, namespace)` keys and can either consume an event or tap it before normal routing.
- Duplicate event IDs are removed per bot before dispatch.
- Handler and platform-service panics are isolated. Idempotent commands can retry temporary platform failures with bounded exponential backoff.
- Adapters, command schedulers, session workers, executor shards, and user services participate in structured cancellation and draining.

## Workspace

| Crate | Purpose |
| --- | --- |
| `oxidebot-core` | Platform-neutral IDs, messages, native escape hatches, canonical events, and routing indexes |
| `oxidebot-runtime` | Adapter contracts, routing, sessions, bounded scheduling, retries, metrics, and shutdown |
| `oxidebot` | Convenience facade that re-exports the core and runtime APIs |
| `oxidebot-testkit` | Deterministic frames, scripted adapters, and observable message services for integration tests |

The public facade is usually the only dependency an application needs:

```toml
[dependencies]
oxidebot = "1.0.0-alpha.1"
```

## Routing

Applications register adapters and typed handlers through the chainable `OxideBot` builder:

```rust,ignore
use oxidebot::{message, on, Context, MessageCreated, Outcome, OxideBot};

#[tokio::main]
async fn main() -> oxidebot::Result<()> {
    let adapter = build_my_adapter();

    OxideBot::new()
        .bot(adapter)
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

`message()` registers interest in all message-created events. Adding `.command("ping")` creates an indexed command route; unrelated commands do not need to be fully decoded.

A handler can send immediately when it needs a receipt or error:

```rust,ignore
.handler(on(message(), |context: Context<MessageCreated>| async move {
    context.reply("accepted").await?;
    Ok(Outcome::continue_())
}))
```

Returning `Outcome::reply(...)` defers the send to the bot's bounded command scheduler. `Outcome::stop()` prevents later matching handlers from running for that event; `Outcome::continue_()` allows the route chain to continue.

## Dialogue sessions

Message contexts can register an exact one-shot session, send a prompt, and parse the next matching response:

```rust,ignore
use std::time::Duration;
use oxidebot::{message, on, AskOptions, Context, MessageCreated, Outcome};

on(message().command("age"), |context: Context<MessageCreated>| async move {
    let age: u8 = context
        .ask_parse("How old are you?", AskOptions::new(Duration::from_secs(30)))
        .await?;
    Ok(Outcome::stop().reply(format!("age={age}")))
})
```

Sessions consume the matching response by default, so it does not also reach generic message handlers. Use `SessionPolicy::Tap` when both the waiter and normal routes should receive it.

## Adapter contract

An adapter supplies immutable identity, outbound services, and an inbound transport:

```rust,ignore
use async_trait::async_trait;
use oxidebot::{Adapter, AdapterContext, AdapterError, BotDescriptor, BotServices};

#[async_trait]
impl Adapter for MyAdapter {
    fn descriptor(&self) -> BotDescriptor {
        self.descriptor.clone()
    }

    fn services(&self) -> BotServices {
        BotServices::messages(self.messages.clone())
    }

    async fn run(self: Box<Self>, context: AdapterContext) -> Result<(), AdapterError> {
        while let Some(frame) = self.transport.next().await {
            context.submit(frame?).await?;
        }
        Ok(())
    }
}
```

Each inbound frame implements two stages:

1. `InboundFrame::index` extracts only routing fields and returns a conservative retained-byte estimate.
2. `InboundFrame::decode` creates the canonical `EventBatch` exactly once, after interest gating and ingress admission.

The runtime validates that indexed and decoded events have identical routing metadata, belong to the adapter's bot slot and platform, and remain within the declared byte estimate.

Portable outgoing messages use ordered `MessageContent` plus message-wide `MessageOptions`. `NativeData` retains lossless platform JSON for fields or operations without an honest portable representation.

## Runtime profiles and observability

`RuntimeProfile::{Eco, Balanced, Throughput}` provides complete starting configurations. Every field remains adjustable through `RuntimeConfig`, including:

- ingress, executor, and command `QueueBudget`s;
- executor and command overload policies;
- shard counts and in-flight limits;
- session and deduplication capacities;
- handler and command timeouts;
- retry count/backoff and shutdown grace.

Pass an `Arc<RuntimeMetrics>` with `.metrics(...)` to observe admitted, ignored, decoded, duplicate, dispatched, dropped, and session-consumed events, handler panics, and command failures.

## Testing adapters and handlers

`oxidebot-testkit` supplies `TestFrame`, `ScriptedAdapter`, and `ScriptedMessageService`. It can verify routing and scheduling without network I/O:

```rust,ignore
let (adapter, service) = ScriptedAdapter::new(platform, bot_id, [
    ScriptStep::Frame(TestFrame::message(event_id, "room", "user", 1_u64, "/ping")),
]);

OxideBot::new()
    .bot(adapter)
    .handler(on(message().command("ping"), |_context| async {
        Ok(Outcome::stop().reply("pong"))
    }))
    .run_to_completion()
    .await?;

assert_eq!(service.sent().len(), 1);
```

## Development

The workspace requires Rust 1.85 or newer.

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo doc --workspace --no-deps
```

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-Apache-2.0.txt](LICENSE-Apache-2.0.txt))
- MIT License ([LICENSE-MIT.txt](LICENSE-MIT.txt))

at your option.
