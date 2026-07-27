# Source delivery notes

This repository snapshot is based on the GitHub `main` commit
`78b695c737fb0a815304a253f07cdeb8091e828f`
(`fix: harden runtime fairness and bounded reliability`) and keeps the workspace
version at `1.0.0-alpha.1`.

This additional kernel reliability pass concentrates on correctness under
saturation, multi-bot fairness, and conservative resource accounting. Major
changes include:

- real cross-shard per-bot permit wake-ups in the virtual-actor executor;
- `Block` semantics that reject unrepresentable events instead of silently
  dropping an accepted submission;
- dedupe commit after session consumption, executor admission, or an explicit
  `DropNewest` drop-and-ack decision;
- per-bot ingress quotas and fair dispatcher admission, including session delivery;
- platform- and exact-bot-scoped route and interest tables;
- build-time snapshots of adapter and handler metadata;
- queue and active-call reservations for high-priority interaction responses,
  preserved until ordinary capacity is saturated;
- command deadlines that include queueing, and global permits released between
  retry attempts and rate-limit cooldowns;
- reusable index-stage adapter decoding plus bounded frame expansion, frame size,
  event size, command size, shard fan-out, session
  command slots, IDs, message collections, rich-text structures, and metadata;
- `RetainedBytes` for explicit charging of shared byte backing allocations;
- clone-cheap `BotHandle` identity and scheduler state;
- bounded/lazy dedupe allocation with TTL, byte, item, and soft per-bot pressure
  controls;
- non-wrapping command/session sequence allocation and explicit clock-range failures;
- regression tests for scoped interest, cross-shard wake-up, multi-bot
  head-of-line isolation, oversized event rejection, and command size rejection.

The source-delivery environment did not run Cargo tooling. No external
dependency was added; this repository retains the existing `Cargo.lock`.

## Final completion pass

The final source pass additionally closes model and scheduling edge cases:

- location coordinates must be finite and within geographic bounds;
- metadata keys are individually bounded and cannot be empty;
- every valid maximum-length event ID fits the configured dedupe envelope;
- route population and per-handler deferred reply fan-out have hard limits;
- dedupe capacity provides at least one slot per registered bot;
- command attempt deadlines are recalculated after admission waits; and
- commands re-check a newly published bot cooldown after global capacity is acquired.

The source-delivery pass itself was not compiled or tested in its packaging
environment.

## Repository integration validation

After integration, the repository was formatted and verified with strict
Clippy, default/no-default/all-feature workspace checks and tests, Rust 1.85
(the declared MSRV), rustdoc warnings as errors, standard release builds, the
`release-small` profile, routing benchmark smoke runs, and repeated runtime
concurrency tests.
