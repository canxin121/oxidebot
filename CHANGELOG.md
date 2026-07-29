# Changelog

All notable user-visible changes are documented here. OxideBot follows
[Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html). The stable
surface consists of every public item in each published OxideBot crate and the
documented facade exports in `oxidebot`.

## Unreleased

### Added

- Canonical `EventFrame` and `EventBatchFrame` adapter ingress APIs, including
  automatic routing-key derivation for portable messages, interactions, and
  native events.
- Target-aware outgoing message planning. Delivery planning now receives the
  exact `MessageTarget` before a physical plan is produced.
- Typed `CallError` / `CallResult` errors for every adapter API operation,
  including retryable, rate-limited, timeout, invalid-request, unsupported,
  planning, and partial-delivery outcomes.
- `DeliveryReportBuilder` and testkit delivery-contract assertions for adapter
  implementations that split one logical message into physical sends.
- `PluginBundle`, metadata, supervised plugin services, and build-time portable
  capability requirements.
- Separate application-build and end-to-end runtime benchmarks.

### Changed

- All portable state-change events now live under
  `Event::Lifecycle(LifecycleEvent::...)`.
- Bot-wide `BotCapabilities` are captured at adapter registration and reused in
  runtime hot paths.
- Runtime capability flags now represent scheduler guarantees only; portable
  feature support has one source of truth in `BotCapabilities`.

### Removed

- `Event::Notice`, `NoticeEvent`, and the `event::notice` module.
- The duplicate `InteractionCapabilities` model.
- Erased `anyhow` adapter-call results from `CallApiTrait`.

## 0.1.8

The last release of the pre-workspace, single-crate architecture. See
[MIGRATING_FROM_0_1.md](MIGRATING_FROM_0_1.md) before upgrading.
