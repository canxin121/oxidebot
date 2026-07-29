# Migrating from OxideBot 0.1

OxideBot 1.0 is a deliberate redesign, not a source-compatible continuation of
the 0.1 API. Upgrade an application and each platform adapter as a migration;
do not try to mechanically rename imports.

## Crate topology

The old `oxidebot` crate has become a workspace with explicit layers:

| 0.1 usage | 1.0 replacement |
| --- | --- |
| `oxidebot` application API | `oxidebot` facade crate |
| Core events, messages, IDs and API types | `oxidebot-core` |
| Runtime, routing, sessions, services and adapter contract | `oxidebot-runtime` |
| Derive and command macros | `oxidebot-macros` (re-exported by `oxidebot`) |
| Deterministic fixtures | `oxidebot-testkit` |
| Local console transport | `oxidebot-adapter-console` |
| Telegram transport | `oxidebot-adapter-telegram` |

Ordinary applications should begin with `use oxidebot::prelude::*;`. Adapter
authors should use `oxidebot::adapter::prelude::*` and depend on the focused
core/runtime crates only when necessary.

## Events

There is now one canonical public event hierarchy:

```rust
Event::Message(..)
Event::Request(..)
Event::Interaction(..)
Event::Lifecycle(..)
Event::Meta(..)
Event::Native(..)
```

`NoticeEvent` and `Event::Notice` no longer exist. Member changes, message
edits/deletions, and reaction changes are `LifecycleEvent` variants. Replace
notice handlers with the corresponding `tags::*` marker or a
`LifecycleEvent` match.

## Adapters

The 0.1 transport model has been replaced with a bounded, two-stage ingress
contract.

- For normal adapters, normalize the wire payload and call
  `AdapterContext::submit_event(event_id, event)`.
- For a batch from one webhook delivery, call `submit_events` with
  `EventFrame` values.
- Implement `InboundFrame` only when a transport can cheaply pre-index its
  raw frame and reuse parsing work in `decode_indexed`.

Do not construct `DispatchIndex`, `DispatchDraft`, or `DispatchBatch` in a
normal adapter. Those kernel records are deliberately hidden implementation
details.

Every `CallApiTrait` method now returns `CallResult<T>`, not `anyhow::Result`.
Map platform failures to `CallError` explicitly so the runtime can distinguish
temporary failures, rate limits, timeouts, invalid requests, unsupported
features, not-found responses, and partial delivery.

Message planning is target-aware:

```rust
api.plan_outgoing_message(&target, &message, FallbackPolicy::Auto)
```

Use `DeliveryReportBuilder` when a logical message becomes several physical
sends. Its partial-delivery error preserves successful references and planner
degradations.

## Handlers and modules

The runtime no longer exposes the old matcher/manager model. Build a flat
`Module`, then install it on `OxideBot`:

```rust
let app = OxideBot::new()
    .adapter(adapter)
    .include(Module::new().command(command("ping"), || async { "pong" }));
```

For reusable features with handlers, metadata, services, and portable
capability prerequisites, use `PluginBundle` and install it with
`OxideBot::plugin`.

## Platform adapters

Adapters published for OxideBot 0.1 are not assumed compatible with 1.0. An
adapter is compatible only after it declares a `1.0` dependency range and
passes the 1.0 adapter contract and end-to-end compatibility suite. Until an
adapter publishes that evidence, keep the 0.1 integration on its old
application branch.

## Upgrade checklist

- [ ] Split dependencies according to the new crate topology.
- [ ] Replace old event/notice matches with the canonical hierarchy.
- [ ] Move adapter ingress to `submit_event` / `EventFrame`.
- [ ] Return typed `CallError` values from all platform API methods.
- [ ] Declare accurate `BotCapabilities`.
- [ ] Replace old routing/matcher setup with `Module` and `OxideBot`.
- [ ] Add `oxidebot-testkit` scenarios for every migrated adapter behavior.
- [ ] Verify a real platform sandbox before moving production traffic.
