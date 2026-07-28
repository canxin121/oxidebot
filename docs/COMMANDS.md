# Typed commands

OxideBot commands are message-domain parsers, not nested routes. A command name
may contain multiple words directly:

```rust
command("admin ban")
command("account settings show")
```

There is no `mount` operation and no command-prefix tree. Writing the complete
command path makes help, aliases, permissions, and route indexing explicit.

## Derive one schema

```rust
use oxidebot::prelude::*;

#[derive(Debug, CommandArgs)]
#[command(
    description = "Create a release",
    category = "operations",
    alias = "release new"
)]
struct CreateReleaseArgs {
    /// Release name.
    #[arg(prompt = "What should the release be called?")]
    name: String,

    /// Target environment.
    #[arg(long, short = 'e', default = "staging".to_owned())]
    environment: String,

    /// Skip the confirmation stage.
    #[arg(long, short = 'f')]
    force: bool,

    /// Optional labels.
    #[arg(long, short = 't')]
    tags: Vec<String>,

    /// Remaining free-form notes.
    #[arg(rest)]
    notes: Vec<String>,
}

async fn create_release(
    Args(args): Args<CreateReleaseArgs>,
) -> String {
    format!("creating {} in {}", args.name, args.environment)
}

let features = Module::new().command(
    CreateReleaseArgs::command("release create")
        .example("/release create v1.2 -e production -t stable"),
    create_release,
);
```

The derive generates `CommandSchema` and conversion code. The same schema is
used by parsing, build-time validation, help, usage rendering, unknown-option
suggestions, and interactive completion.

## Field forms

A plain field is positional:

```rust
name: String
```

An `Option<T>` field is optional command data:

```rust
environment: Option<String>
```

This is different from a generic optional handler extractor, which OxideBot
does not provide.

A `bool` long or short option is a flag:

```rust
#[arg(long, short = 'f')]
force: bool
```

Repeated option values use `Vec<T>`:

```rust
#[arg(long, short = 't')]
tags: Vec<String>
```

A final positional `Vec<T>` may consume the remainder:

```rust
#[arg(rest)]
notes: Vec<String>
```

Defaults are generated into the schema:

```rust
#[arg(long, default = 10_u32)]
limit: u32
```

A required field may define a prompt used only when completion is enabled:

```rust
#[arg(prompt = "Which project?")]
project: String
```

## Accepted syntax

The tokenizer and schema parser support:

- single and double quotes;
- backslash escapes;
- empty quoted values (`""` and `''`);
- `--long value` and `--long=value`;
- short options and combined short flags;
- attached short-option values;
- `--` to end option parsing;
- negative numeric positional values;
- required, optional, defaulted, repeated, and rest arguments;
- exact aliases and multi-word command paths;
- multiple prefixes, no-prefix commands, and case-insensitive commands.

Canonical non-text segments are not flattened. Command fields can consume
`Mention`, `File`, `MessageSegment`, or a custom type implementing
`FromCommandValue`.

## Fast and broad command matching

The default command form:

```rust
command("ping")
```

uses the exact `/ping` pre-decode key. The runtime can discard an unrelated
message frame before constructing the complete event.

Commands enter the broad message candidate slot only when their matching rules
require it, such as:

```rust
command("hello").prefix("!")
command("help").no_prefix()
command("PING").case_insensitive()
```

Full parsing still occurs only after the message is admitted.

## Interactive completion

```rust
let deploy = DeployArgs::command("deploy").completion(
    CompletionConfig::new()
        .timeout(std::time::Duration::from_secs(60))
        .max_rounds(3)
        .cancel_words(["cancel", "stop", "取消"]),
);
```

The flow is:

```text
exact command match
-> module guards
-> schema validation
-> bounded dialogue only for MissingArgument
-> complete Args<T>
-> before hooks
-> handler extraction and execution
```

Unknown options, duplicate options, missing option values, extra values, and
invalid type conversions are returned immediately. They never start a prompt
loop. Answers are inserted into the exact missing positional or named field,
including long options.

Session registration happens before the prompt is sent, preventing a fast user
reply from racing the waiter. Cancellation and maximum rounds are bounded by
the command's `CompletionConfig`.

## Automatic help

```rust
let features = Module::new()
    .command(CreateReleaseArgs::command("release create"), create_release)
    .command(command("ping").description("Check liveness"), ping)
    .help();
```

`/help` renders the visible command catalog. `/help release create` renders the
same schema used by the parser, including usage fragments, descriptions,
defaults, aliases, categories, and examples.

Included modules contribute their commands to the same catalog. Calling
`help()` more than once through module composition does not create duplicate
help handlers.

## Raw command access

Most handlers should use `Args<T>`. `CommandResult` is available when a handler
needs the matched alias, original lossless command values, or custom dynamic
parsing:

```rust
async fn inspect(command: CommandResult) -> String {
    format!("matched {}", command.command().name())
}
```
