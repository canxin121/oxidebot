# Source delivery notes

This repository snapshot is based on the GitHub `main` commit
`c9f63db895ad91f01482823e94aeae6cec47c288` (`feat: optimize v1 runtime kernel`)
and keeps the workspace version at `1.0.0-alpha.1`.

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

## Repository integration validation

After integration, the repository was formatted and verified with strict
Clippy, default/no-default/all-feature workspace checks and tests, Rust 1.85
(the declared MSRV), rustdoc warnings as errors, standard release builds, the
`release-small` profile, routing benchmark smoke runs, and repeated runtime
concurrency tests.
