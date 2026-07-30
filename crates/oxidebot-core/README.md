# oxidebot-core

`oxidebot-core` is OxideBot's platform-neutral model layer. It defines the
canonical event hierarchy, message intermediate representation, interaction
schemas, identifiers, adapter API contracts, capabilities, and delivery
reports shared by every runtime and adapter.

Most application authors should depend on the [`oxidebot`](https://docs.rs/oxidebot)
facade instead. Depend on this crate directly when implementing an adapter,
building a reusable protocol integration, or consuming OxideBot's model without
its runtime.

```toml
[dependencies]
oxidebot-core = "1"
```

The API reference is available on [docs.rs](https://docs.rs/oxidebot-core).
For the framework overview, adapters, and application examples, see the
[OxideBot repository](https://github.com/canxin121/oxidebot).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-Apache-2.0.txt)
or [MIT license](LICENSE-MIT.txt) at your option.
