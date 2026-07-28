# oxidebot-adapter-console

A finite stdin/stdout adapter for local OxideBot development, examples, and
small terminal bots. Each input line becomes a direct message from one
configurable user; outgoing messages are rendered to stdout.

```rust
use oxidebot::prelude::*;
use oxidebot_adapter_console::ConsoleAdapter;

#[oxidebot::command("ping")]
async fn ping() -> &'static str {
    "pong"
}

#[tokio::main]
async fn main() -> oxidebot::Result<()> {
    OxideBot::new()
        .adapter(ConsoleAdapter::development())
        .add(ping)
        .include(Module::new().help())
        .run()
        .await
}
```

The adapter uses the normal bounded runtime, unified message IR, command
parser, delivery planner, and receipts. End stdin with Ctrl-D or stop the
process with Ctrl-C.
