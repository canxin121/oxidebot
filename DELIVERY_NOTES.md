# Source delivery notes

This archive is based on repository commit
`b1784a41ddb1a24076605c7aeaa17f1e60f158a4` and keeps the workspace version at
`1.0.0-alpha.1`.

The source pass concentrates on the runtime kernel rather than adding platform
adapters. Major changes include:

- actual route candidate indexes instead of a full handler scan;
- typed contexts backed by one shared event envelope;
- strict adapter index/body and outbound ownership validation;
- exact session interest fast misses and cancellation-safe registration;
- global and per-bot executor/command resource envelopes;
- fixed virtual-actor execution with optional conversation/actor partitioning;
- weighted interaction-command priority without normal-command starvation;
- adapter-declared idempotency before automatic retries;
- command attempt and total deadlines, bounded backoff, jitter, and Retry-After
  handling;
- signed IDs, thread/topic addressing, bounded semantic IDs, and bounded
  deduplication by entries, bytes, TTL, and bot share;
- panic-unwind release behavior plus an explicit aborting small-binary profile;
- one absolute structured-shutdown deadline;
- lower-power default features without Tokio's multi-thread runtime.

Per the requested delivery mode, this packaging step intentionally did not run
Cargo compilation, formatting, Clippy, tests, documentation builds, or
benchmarks. The repository retains its existing source tests as review and
future validation material.

`Cargo.lock` was not included in the original archive because that delivery
environment did not run Cargo. During repository integration, the existing
lockfile was retained and regenerated for the new benchmark dependencies. The
integrated tree was then formatted and verified with strict Clippy, workspace
tests (including all targets and feature combinations), rustdoc warnings as
errors, release builds, the `release-small` profile, and the declared Rust 1.85
minimum toolchain.
