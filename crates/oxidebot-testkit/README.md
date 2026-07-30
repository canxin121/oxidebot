# oxidebot-testkit

`oxidebot-testkit` supplies deterministic adapters, scripted transport steps,
inbound frames, delivery-contract assertions, and a fluent `BotTest` scenario
API for testing OxideBot applications without a networked platform.

Add it as a development dependency alongside the public facade:

```toml
[dev-dependencies]
oxidebot = "1"
oxidebot-testkit = "1"
```

The [API reference](https://docs.rs/oxidebot-testkit) documents `BotTest`,
`ScriptedAdapter`, `ScriptedApi`, `ScriptStep`, and inbound test frames. See
the [OxideBot repository](https://github.com/canxin121/oxidebot) for executable
examples and the complete framework documentation.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-Apache-2.0.txt)
or [MIT license](LICENSE-MIT.txt) at your option.
