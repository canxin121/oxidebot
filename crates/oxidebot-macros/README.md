# oxidebot-macros

`oxidebot-macros` provides the procedural macros behind OxideBot's typed,
declarative application API: `#[command]`, `CommandArgs`, `BotCommand`,
`BotState`, and `DialogueForm`.

Application authors should normally depend on
[`oxidebot`](https://docs.rs/oxidebot), which re-exports these macros with a
stable, ergonomic facade. Depend on this crate directly only when deliberately
using OxideBot's lower-level runtime crates.

```toml
[dependencies]
oxidebot = "1"
```

The generated code targets the public contract in
[`oxidebot-runtime`](https://docs.rs/oxidebot-runtime), including renamed
facade dependencies. See the [API reference](https://docs.rs/oxidebot-macros)
and the [OxideBot repository](https://github.com/canxin121/oxidebot) for usage
examples.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-Apache-2.0.txt)
or [MIT license](LICENSE-MIT.txt) at your option.
