# Routing and extraction

## Ordinary functions

A Router handler is an async function with zero to twelve extractor arguments.
The function does not implement a framework trait and does not receive a large
mandatory context object.

```rust
async fn health() -> &'static str {
    "ok"
}

async fn profile(
    Sender(user): Sender,
    State(state): State<AppState>,
) -> Result<Message, AppError> {
    Ok(state.profile_message(user.id).await?)
}
```

Return values implement `IntoResponse`. Message-like values reply and stop;
`()` and `None` continue; `Response` controls both effects explicitly.

## Route kinds

```rust
let router = Router::new()
    .command(command("ping"), ping)
    .message(observe_all_messages)
    .event(tags::FriendAdd, friend_request)
    .interaction("settings.save", save_settings)
    .native("vendor.event", native_event);
```

`route(matcher, handler)` accepts the original compile-time matcher API. The
specialized methods are conveniences around the same route metadata.

## State projection

Root state is stored once in `Arc<S>`. A focused extractor can be produced
without cloning the entire root state:

```rust
#[derive(Clone)]
struct Database(Arc<Pool>);

impl FromRef<AppState> for Database {
    fn from_ref(state: &AppState) -> Self {
        state.database.clone()
    }
}

async fn handler(State(database): State<Database>) { /* ... */ }
```

## Extensions

Middleware may inject request-local typed values:

```rust
async fn auth(
    mut request: Request<AppState>,
    next: Next<AppState>,
) -> Result<Response, AuthError> {
    let identity = authenticate(request.event()).await?;
    request.extensions_mut().insert(identity);
    Ok(next.run(request).await)
}

async fn protected(Extension(identity): Extension<Identity>) -> String {
    format!("hello {}", identity.name)
}
```

`Extension<T>` clones only the requested value. Use `Arc<T>` for large values.

## Middleware ordering

`route_layer` applies only to endpoints already present. `layer` is baked onto
all endpoints in that Router, including endpoints merged afterward.

```rust
let router = public_routes()
    .route_layer(from_fn(rate_limit))
    .merge(admin_routes())
    .layer(from_fn(tracing));
```

The original typed-context handlers registered through `Router::handler` remain
low-level compatibility handlers and are not transformed into extractor
requests.

## Plugin modules

A plugin is simply a Router or a function returning one:

```rust
fn audit_plugin() -> Router<AppState> {
    Router::new()
        .event(tags::MessageDeleted, audit_delete)
        .command(command("audit status"), audit_status)
}

let app = Router::new().plugin(audit_plugin);
```

Because plugin routes are flattened, they share the application's state,
queues, sessions, middleware model, event allocation, and API handles.

## Full event access

Use `EventContext<Tag>` when a handler needs the exact 0.1.8 payload:

```rust
async fn request(
    context: EventContext<tags::FriendAdd>,
    bot: Bot,
) -> HandlerResult<()> {
    context
        .event()
        .approve(bot.into_inner())
        .await
        .map_err(|error| HandlerError::Api(error.to_string()))
}
```

Notice and request tags return their concrete payload structs. Other tags keep
the canonical category type when the original variant has an inline or
heterogeneous shape.
