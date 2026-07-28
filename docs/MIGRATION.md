# Migrating typed-context handlers to Router

The original API remains valid:

```rust
OxideBot::new().handler(on(
    message().command("ping"),
    |_context: MessageContext| async move {
        Ok(Outcome::stop().text("pong"))
    },
));
```

The equivalent Router handler is:

```rust
async fn ping() -> &'static str {
    "pong"
}

let routes = Router::new().command(command("ping"), ping);
OxideBot::new().router(routes);
```

## Replace context reads with extractors

Before:

```rust
async move {
    let user = context.event().sender.clone();
    let text = context.text();
    let state = context.state();
    // ...
}
```

After:

```rust
async fn handler(
    Sender(user): Sender,
    Text(text): Text,
    State(state): State<AppState>,
) {
    // ...
}
```

Keep `EventContext<tags::...>` alongside extractors when the complete original
payload is useful.

## Replace manual argument parsing

Move command fields into a `CommandArgs` struct, then extract `Parsed<T>`. This
centralizes parsing, errors, defaults, help, and optional dialogue completion.

## Replace deferred Outcome boilerplate

- `Ok(Outcome::stop().text(text))` becomes `text` or `Ok(text)`.
- `Ok(Outcome::continue_())` becomes `()` or `Response::continue_()`.
- Multiple deferred replies use `Response::stop().reply(a).reply(b)`.
- Workflows needing send IDs use `Reply` and `Receipt`.

## Compose feature modules

Return `Router<AppState>` from each feature instead of registering handlers into
a feature-owned runtime. Merge or mount those Routers in one root Router.

## Migrate incrementally

`Router::handler(old_handler)` accepts an existing `Handler<S>`, and
`OxideBot::handler` remains available. New and old handlers share the compiled
candidate tables and one canonical event allocation, so migration does not
require an all-at-once adapter or event-model rewrite.
