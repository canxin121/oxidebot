# Final source pass

This tree is the final source-only hardening pass based on GitHub `main` commit
`78b695c737fb0a815304a253f07cdeb8091e828f`.

The pass intentionally keeps `1.0.0-alpha.1` and adds no dependency. It closes
remaining edge cases around model validation, dedupe representability, route and
handler-effect fan-out, stale command deadlines, bot-wide cooldown publication,
and repeated event-size traversal.

Per the requested delivery mode, no Cargo compilation, formatting, Clippy,
tests, rustdoc, or benchmarks were executed after these local edits.

## Repository integration validation

After integration, the repository was formatted and verified with strict
Clippy, default/no-default/all-feature workspace checks and tests, Rust 1.85
(the declared MSRV), rustdoc warnings as errors, standard release builds, the
`release-small` profile, routing benchmark smoke runs, and repeated runtime
concurrency tests.
