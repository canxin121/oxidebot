# oxidebot-runtime

`oxidebot-runtime` is OxideBot's bounded, structured-concurrency execution
layer. It provides application assembly, handler routing, typed extraction,
command parsing and completion, dialogue recovery, lifecycle supervision, and
the adapter runtime contract.

Most bots should use the [`oxidebot`](https://docs.rs/oxidebot) facade instead;
it re-exports the supported authoring surface together with its derive macros.
Use this crate directly for advanced framework integrations, adapters, or a
carefully scoped custom facade.

```toml
[dependencies]
oxidebot = "1"
```

Consult the [API reference](https://docs.rs/oxidebot-runtime) for lower-level
extension points and the [OxideBot repository](https://github.com/canxin121/oxidebot)
for end-to-end examples.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-Apache-2.0.txt)
or [MIT license](LICENSE-MIT.txt) at your option.
