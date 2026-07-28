# OxideBot architecture

## Design rule: one semantic model

OxideBot exposes one event hierarchy: the restored 0.1.8 `Event`. It exposes one
adapter and bot API: `CallApiTrait`. The Router, command system, extractors,
middleware, sessions, and reply helpers are views and orchestration around those
same types; they do not introduce a second event body or a reduced API.

The runtime owns compact dispatch metadata, but that metadata only contains
routing keys, accounting information, identity, and a reference to the same
canonical event.

## Authoring pipeline

A matched Router endpoint receives a clone-cheap `Request<S>` containing:

- one `Arc<DispatchEnvelope>`;
- one `Arc<S>` application state;
- the current `BotHandle`;
- the shared bounded `SessionRegistry`;
- the shared shutdown signal;
- request-local typed `Extensions`;
- an optional lossless `CommandResult`.

Middleware consumes and returns that request through a `Next<S>` chain. The
endpoint then evaluates its `FromRequest<S>` extractors in function-argument
order and invokes an ordinary async function. Its return value is converted by
`IntoResponse`.

No extractor runs before the route has matched. No event payload is cloned to
create a request or context.

## Router compilation

`Router<S>` is a build-time composition structure. `merge`, `mount`, and
`plugin` flatten modules into one ordered endpoint list. Middleware layers are
baked onto endpoint definitions, and each endpoint becomes the existing
`ErasedHandler<S>` representation before application startup.

Routes use the existing indexes:

- exact `EventType` routes enter the 53-slot dense table;
- normal `/name` commands enter the exact command-key table;
- interaction IDs and platform-native names enter exact hash tables;
- commands using custom prefixes, no prefix, or case-insensitive matching enter
  the broad message candidate slot and perform their full match only after the
  message event is admitted.

Aliases sharing one root are de-duplicated before route registration. At
dispatch time, global, platform, and bot candidate slices are merged by route
ID, preserving registration order without allocating a temporary candidate
vector.

## Command parsing

A `Command` performs a lossless first-stage match over canonical message
segments. Text segments use a shell-like tokenizer; mentions, files, media, and
other segments remain typed `CommandValue` variants.

`CommandSchema` is shared by parsing, automatic help, validation, and
interactive completion. `#[derive(CommandArgs)]` generates only schema and
conversion code; it does not create a parallel runtime parser.

Interactive completion is entered only for `MissingArgument`. The completion
path registers an exact bounded session before sending a prompt, preventing a
fast reply from racing registration. Replies are re-tokenized losslessly and
inserted into the missing positional or named option. Unknown options, missing
option values, and invalid conversions are returned immediately.

## Response effects

`Response` contains a stop decision and zero or more deferred canonical message
segment lists. The compiled router enforces the configured reply-count limit
and sends those effects through the same `CallApiTrait` object exposed to
handlers.

`Reply` bypasses deferred effects for workflows requiring immediate API results.
It returns a `Receipt` over every `SendMessageResponse`, so message splitting by
a platform remains visible and safe.

## Two-stage admission

A platform frame first produces a compact `DispatchIndex` containing:

- bot and platform identity;
- the stable `EventType`;
- optional conversation and actor keys;
- optional normalized command, interaction, or native keys.

Static route interest and dynamic session interest are checked against this
index. An uninterested frame can be dropped before the complete event is
decoded. When interested, the adapter creates a `DispatchDraft` containing the
original 0.1.8 `Event`; the runtime verifies that the event type agrees with the
index.

## Shared ownership and ordering

A decoded event is retained once. Handlers, extractors, typed contexts, filters,
and sessions borrow from or share that allocation. Large messages, files,
layouts, polls, payments, and platform-native data are never duplicated per
candidate route.

Execution is partitioned by virtual actor. Conversation/thread keys are used
first, then actor keys, then the event ID fallback. One key executes serially,
while unrelated keys can run concurrently through fixed worker shards rather
than unbounded task creation.

## Bounded resources

Ingress, executor, session, and command queues enforce item and retained-byte
budgets. Global and per-bot permits are acquired together. Session namespaces,
route keys, decoded envelopes, raw payloads, and handler reply counts are also
bounded or validated before entering downstream work.

## Compatibility boundary

The pre-Router `on(matcher, handler)` and typed `Context` interfaces remain
available. They compile into the same `PreparedHandler` and route tables as new
Router endpoints. They are an explicit low-level escape hatch, not a second
framework layered beside Router.
