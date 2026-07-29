# OxideBot architecture

## One semantic model

OxideBot exposes one canonical `Event` hierarchy and one platform-neutral bot
API: `CallApiTrait`. Commands, modules, extractors,
dialogues, and reply helpers are orchestration around those same types; they do
not introduce a reduced event body or second API.

The runtime keeps compact dispatch metadata for admission and indexing, but a
decoded event payload is retained once and shared by every candidate through
`Arc<DispatchEnvelope>`.

## Authoring pipeline

The public pipeline is:

```text
Module candidate match
-> Context<S>
-> MessageNormalizer / bounded Shortcut / CommandRewriter
-> one command-tree parse into CommandMatch
-> feature-local and module guards
-> CommandMiddleware / static, dynamic, or interactive completion
-> feature-local and module before hooks
-> synchronous Extract<S> / Args<T> / generated branch wrapper
-> ordinary async function
-> IntoOutcome
-> feature-local and module after hooks
-> CommandOutputMiddleware / localized CommandRenderer where applicable
-> handler-kind blocking default
-> Messenger / DeliveryMiddleware
-> capability-aware DeliveryPlan
-> adapter transport / DeliveryReport
```

`Context<S>` is the single common runtime view. It contains shared handles to:

- the one dispatch envelope and canonical event;
- root `Arc<S>` application state;
- current `BotHandle` and complete API object;
- the bounded `SessionRegistry`;
- shutdown;
- an optional lossless `CommandMatch`.

There is no parallel request object, dynamic request extensions, or per-handler
event copy.

`EventContext<Tag>` is a state-independent typed view over the same envelope.
State remains an explicit `State<S>` or `Context<S>` handler argument.

## Flat module compilation

`Module<S>` is a build-time collection of handler definitions, guards, and
hooks. `include` flattens another module in registration order. It does not
construct a path tree or another runtime.

`Feature<S>` is the single-handler authoring unit. A generated command value,
manual command, event, interaction, or native handler can keep its guard, hooks,
platform/bot scope, propagation override, shortcuts, interactive completion,
and typed dynamic completers together before being installed with
`Module::add`. This prevents configuration for one behavior from being spread
across the application builder and several small modules.

Module-wide guards and before hooks are prepended to included handlers. Module
`after` hooks are appended, so child after hooks run before parent after hooks.
The result is deterministic and independent of method-call order.

Platform and bot restrictions also compose structurally. Inclusion intersects
scopes instead of overwriting them, so a parent cannot accidentally broaden a
child module's bot restriction; incompatible scopes are rejected during build.

At build time every definition becomes one or more internal `ErasedHandler<S>`
values and then `PreparedHandler<S>` entries. Public function-handler
ergonomics stop at this boundary; runtime indexes remain specialized and
compact.

## Compiled candidate indexes

Routes use existing indexes:

- exact `EventType` handlers enter the 52-slot dense table;
- normal `/name` commands enter the exact command-root table;
- interaction IDs and platform-native names enter exact hash tables;
- custom-prefix, no-prefix, case-insensitive commands, and applications with
  arbitrary message normalization or command rewriting enter the broad message
  candidate slot and perform a full match only after admission; static/runtime
  shortcuts instead select only their owning command IDs before parsing.

Global, platform, and bot-scoped candidate slices are merged by monotonically
assigned route ID. Registration order is preserved without allocating a
temporary candidate vector.

## Extraction

`Extract<S>` is synchronous and monomorphized. Built-in extraction consists of
matching or borrowing the canonical event and cloning only small owned values
or clone-cheap handles requested by the handler. `#[derive(BotState)]` generates
static `FromState<Root>` mappings, so handlers can request `State<Service>`
without a runtime type map or repetitive extractor implementations.

No extractor runs before a handler matches. Interactive completion is not an
extractor: the command endpoint performs that workflow once, then `Args<T>`
parses synchronously from the completed `CommandMatch`.

The lack of a generic optional extractor is intentional. Absence is expressed
by domain types such as `MaybeConversation`, while parse, permission, API, and
configuration errors remain visible.

## Commands

A `Command` performs a lossless first-stage match over canonical message
segments. Text segments use a shell-like tokenizer; mentions, files, media, and
other segments remain typed `CommandValue` variants.

`CommandSchema` is shared by parsing, validation, automatic help, usage output,
interactive completion, and platform-native publication. `#[derive(CommandArgs)]`
generates schema and conversion code rather than a parallel parser.
`#[oxidebot::command]` and `#[oxidebot::branch]` generate installable feature
values, so a command or branch is declared once and added with `Module::add`.
`#[oxidebot::completer]` gives a state-aware completion function a static provider
type; `#[arg(complete = provider)]` binds it beside the owning field.

Completion is entered only for `MissingArgument`, after module guards. It
registers an exact bounded session before sending a prompt, preventing a fast
reply from racing registration. Replies are tokenized losslessly and inserted
into the precise missing positional or named field. Other parse failures return
immediately.

## Effects and routing flow

`Outcome` contains canonical deferred replies and a propagation decision.
It is not an HTTP response.

The matched handler kind supplies the default when no override exists:

- command and interaction handlers stop;
- ordinary event and native observers continue.

This separates effects from dispatch policy. A command that sends immediately
and returns `()` still blocks naturally; an event observer that returns a
message still continues naturally. Explicit `Outcome::stop()` and
`Outcome::continue_()` are reserved for true overrides.

The compiled dispatcher enforces the configured deferred-reply count and
submits delivery through the selected bot's bounded command scheduler.
`CallApiTrait` is invoked only inside that scheduler for framework-managed
operations; the raw `Bot` extractor remains an intentional escape hatch.

Answerable interactions create one shared `Responder` state for explicit
handler calls, automatic returned messages, and deadline protection. Initial
acknowledgement is atomic, near-deadline work is automatically deferred through
reserved high-priority capacity, and later messages become follow-ups rather
than unrelated conversation sends.

`Messenger` is the ordinary immediate-send facade. It unifies natural replies,
current-conversation sends, explicit proactive `Address` targets, deterministic
bot selection, fallback policy, and receipts. `Reply` remains the smaller
current-conversation primitive used internally and as an advanced extractor.
Both return a `Receipt` over every `SendMessageResponse`, preserving platform
message splitting.

## Guards and hooks

Guards model admission concerns such as permissions, rate limits, chat type,
and feature flags. They run before command completion and extraction. `Skip`
continues candidate dispatch; `Deny` emits explicit effects and stops.

Before hooks run after completion and before extraction. After hooks receive the
handler's unresolved `Outcome` and may observe or transform it. The natural
handler-kind blocking default is resolved only after after hooks complete.

There is no continuation object and no mutable type map. Handler dependencies
remain in function signatures or root state.

## Error boundary

Function handlers accept successful values implementing `IntoOutcome` and
fallible values whose error explicitly converts to `HandlerError`.

Only `HandlerError::User` becomes a user reply. Command, session, API,
application, parse, timeout, and service errors are logged internally. A failed
command or interaction still keeps its natural blocking boundary, while a failed observer
leaves later observers available. Failure therefore cannot silently change the
matched handler kind's propagation policy. The runtime never sends an arbitrary
`Display` representation to a chat.

Extractor errors are separately constrained to safe contract messages. Their
reply inherits the matched handler kind's normal propagation policy, just like
an explicitly user-facing handler error.

## Two-stage admission

A platform frame first produces a compact `DispatchIndex` containing:

- bot and platform identity;
- stable `EventType`;
- optional conversation and actor keys;
- optional normalized command, interaction, or native keys.

Static route interest and dynamic session interest are checked against this
index. An uninterested frame can be dropped before full event decode. An
admitted adapter frame becomes a `DispatchDraft`; runtime validation confirms
that the decoded event agrees with the compact index.

## Ownership and ordering

A decoded event is retained once. Contexts, typed event views, filters,
handlers, and sessions borrow from or share that allocation. Large messages,
files, layouts, polls, payments, and native data are not duplicated per route.

Execution is partitioned by virtual actor: conversation/thread key first, actor
key next, event ID fallback last. One key executes serially, while unrelated
keys run concurrently through fixed worker shards rather than unbounded task
creation.

## Bounded resources

Ingress, executor, session, and API command queues enforce item and
retained-byte budgets. Global and per-bot permits are acquired together.
Session namespaces, route keys, decoded envelopes (including owned profile
strings), raw payloads, completion rounds, and handler reply counts are bounded
or validated before downstream work. Translation catalogs, shortcut
registries, and target aliases additionally enforce total retained-byte limits;
shortcut matching releases its registry lock before running a capped scan.

## Frozen authoring extensions

Message normalization, command rewriting, parsed-command transformation, output
rendering, locale resolution, dynamic completion, and delivery transformation
are narrow traits stored in `AuthoringRuntime<S>`. The application builder
freezes them before adapters start. Ordinary async functions implement these
traits through ownership-based blanket implementations, so extension code does
not borrow a transient request object or require dynamic parameter injection.

`TranslationCatalog` loads bounded structure-preserving JSON resources
atomically. The same catalog may back handler-local `I18n` messages and the
catalog-aware command renderer. Missing framework templates fall back to the
built-in renderer rather than making command errors or help delivery fallible.

Runtime shortcuts and command enablement live in a bounded `CommandRegistry`.
Its desired, published, and pending revisions are explicit. A failed platform
publication remains pending, and a later refresh or repeated no-op toggle
retries the complete idempotent definition set. Static shortcuts opt only
matching command IDs into dynamic candidate lookup; arbitrary rewriters or
normalizers deliberately select the broader message candidate path. Both
remain cold paths and preserve the exact root-command index for ordinary
commands.

## Proactive delivery and media

An `Address` combines a canonical `MessageTarget` with deterministic bot
selection. Exact bot selection is preferred; platform selection rejects
ambiguity rather than choosing randomly. `TargetDirectory` is an item- and
byte-bounded application alias map, not a background global target crawler.

Media access is explicit. `LocalMediaResolver` streams at most `limit + 1`
bytes from local files and bounds retained base64 data;
network fetching and hosting are caller-supplied `MediaFetcher` and `MediaHost`
services. This keeps network access, credentials, file retention, and byte
budgets outside the core parser and dispatcher.

## Authoring and adapter conveniences

The convenience layer is intentionally compile-time or cold-path only:

- `#[derive(DialogueForm)]` compiles bounded questions, retries, validation,
  choices, and confirmation into the existing session registry;
- `DialogueQuestion<T>` validates and retries without changing normal dispatch;
- `ResultExt` and `OptionExt` preserve the explicit user/internal/API error
  boundary while removing repetitive `map_err` blocks;
- focused preludes keep ordinary IDE completion small; adapter and framework
  work imports the focused facade module or explicit core/runtime exports;
- `MessageFrame` and `AdapterContext::submit_text` provide a conservative
  canonical-frame path for simple transports, while high-throughput adapters
  retain direct `InboundFrame` control;
- `BotTest` adds deterministic send barriers over `ScriptedAdapter`, and
  `CommandTest` parses the same immutable command IR without starting runtime
  workers.

The workspace console adapter is a real finite stdin/stdout transport used by a
copyable interactive example. It exercises the same event, command, message,
delivery, and receipt paths as network adapters rather than providing a second
mock authoring API.
