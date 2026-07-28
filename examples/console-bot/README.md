# OxideBot console example

Run from the repository root:

```bash
cargo run -p oxidebot-console-example
```

Then type:

```text
/ping
/echo hello OxideBot
/hello
/setup
/help
```

The example uses the same `Message`, command IR, bounded runtime, delivery
planner, and receipts as a network adapter. End stdin with Ctrl-D, or stop the
process with Ctrl-C.
