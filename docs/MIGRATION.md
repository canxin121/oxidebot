# Migrating from the Web-shaped draft API

This release deliberately removes the parts of the earlier 1.0 draft that
modeled a bot event as an HTTP request. The change is intentionally breaking:
there is one recommended authoring path rather than compatibility aliases for
two competing mental models.

## Name mapping

| Earlier draft | Current API |
| --- | --- |
| `Router<S>` | `Module<S>` |
| `router.merge(other)` | `module.include(other)` |
| `router.mount("admin", child)` | explicit command names such as `"admin ban"` |
| `Plugin` / `router.plugin(...)` | a function returning `Module<S>` + `include` |
| `Request<S>` | `Context<S>` |
| `Response` | `Outcome` |
| `IntoResponse` | `IntoOutcome` |
| `FromRequest<S>` | synchronous `Extract<S>` |
| `Parsed<T>` | `Args<T>` |
| `Extensions` / `Extension<T>` | root `State<S>`, explicit arguments, or custom `Extract` |
| `FromRef<S>` | `#[derive(BotState)]` + focused `State<Service>` |
| `layer(from_fn(...))` | `guard`, `before`, and `after` |
| `Next<S>` | removed; hooks have direct Bot-domain phases |
| `route_layer` | removed; module hooks are order-independent |
| `Router::handler` / `on(matcher, ...)` | `Module::on`, `message`, `command`, `interaction`, `native` |
| `OxideBot::bot(adapter)` | `OxideBot::adapter(adapter)` |
| `OxideBot::router(routes)` | `OxideBot::include(module)` |

## Minimal command

Before:

```rust
async fn ping() -> &'static str {
    "pong"
}

let routes = Router::new()
    .command(command("ping"), ping);

OxideBot::new()
    .bot(adapter)
    .router(routes);
```

After:

```rust
#[oxidebot::command("ping")]
async fn ping() -> &'static str {
    "pong"
}

OxideBot::new()
    .adapter(adapter)
    .add(ping);
```

The generated feature carries the schema and handler together; no separate
`ping_command()` value must be paired with `ping`.

## Replace nested-route vocabulary

Before:

```rust
let features = Router::new()
    .mount("admin", Router::new()
        .command(command("ban"), ban)
        .command(command("mute"), mute));
```

After:

```rust
let features = Module::new()
    .add(command("admin ban").handle(ban))
    .add(command("admin mute").handle(mute));
```

The old `mount` only rewrote command strings and did nothing meaningful to
event routes. Multi-word commands now state their real syntax directly.

## Replace request extraction

Before:

```rust
async fn handler(
    Parsed(args): Parsed<MyArgs>,
    State(database): State<Database>,
    Extension(identity): Extension<Identity>,
) -> String {
    // ...
}
```

After:

```rust
#[derive(BotState)]
struct AppState {
    #[state]
    database: Database,
}

async fn handler(
    Args(args): Args<MyArgs>,
    State(database): State<Database>,
    Sender(user): Sender,
) -> HandlerResult<String> {
    database
        .run(identity_for(&user), args)
        .await
        .internal("run command")
}
```

`BotState` generates static projections; there is no runtime extension or
substate map. Ask for `State<AppState>` when the complete root is genuinely
needed. For asynchronous authorization, use a guard or ordinary business logic.

## Replace middleware

Before:

```rust
async fn auth(
    mut request: Request<AppState>,
    next: Next<AppState>,
) -> Response {
    // inspect, mutate Extensions, call next
}

let routes = routes.layer(from_fn(auth));
```

After, admission is a guard:

```rust
async fn auth(context: Context<AppState>) -> GuardDecision {
    if context.state().allows(context.event()) {
        GuardDecision::allow()
    } else {
        GuardDecision::deny("Permission denied.")
    }
}

let features = features.guard(auth);
```

Observation is a phase hook:

```rust
async fn trace_start(context: Context<AppState>) {
    tracing::debug!(event = ?context.event_type(), "start");
}

async fn trace_end(
    context: Context<AppState>,
    outcome: Outcome,
) -> Outcome {
    tracing::debug!(
        event = ?context.event_type(),
        replies = outcome.replies().len(),
        "end",
    );
    outcome
}

let features = features
    .before(trace_start)
    .after(trace_end);
```

Hooks apply to the entire module regardless of declaration order. There is no
`layer` versus `route_layer` distinction.

## Replace response flow assumptions

Earlier, the return type implicitly controlled routing:

```text
String -> reply and stop
()     -> continue
None   -> continue
```

Now handler kind controls the natural default:

```text
command / interaction -> stop
ordinary event / native observer -> continue
```

Returning a message controls only the reply. `()` is therefore safe for a
command that already sent through `Reply`; it still blocks as a command.

Use explicit overrides only when necessary:

```rust
Outcome::continue_().reply("allow another handler")
Outcome::stop().reply("consume this event")
```

`Option<T>: IntoOutcome` remains available for optional effects. `None` produces
no deferred reply, while the matched handler kind still supplies the routing
default, so absence cannot silently change command-versus-observer flow.

## Replace error conversion

The earlier blanket `Result<T, E: Display>` conversion could leak database,
filesystem, token, or platform details to a chat. Current fallible handlers
require `E: Into<HandlerError>`.

Use a deliberate user error:

```rust
Err(HandlerError::user("That project does not exist."))
```

Map internal failures to an internal variant:

```rust
operation()
    .await
    .internal("perform operation")?;
```

Only `HandlerError::User` is replied to. Other variants are logged.

## Replace legacy matcher registration

Before:

```rust
OxideBot::new().handler(on(
    message().command("ping"),
    |context: MessageContext| async move {
        Ok(Outcome::stop().text("pong"))
    },
));
```

After:

```rust
#[oxidebot::command("ping")]
async fn ping(context: MessageContext) -> &'static str {
    assert_eq!(context.text(), "/ping");
    "pong"
}

OxideBot::new()
    .adapter(adapter)
    .add(ping);
```

The complete event remains available through `EventContext<Tag>`. Removing the
legacy registration API does not remove any 0.1.8 event or `CallApiTrait`
capability; it removes only the second way to feed the same compiled dispatch
tables.
