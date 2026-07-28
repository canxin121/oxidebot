# Commands

## Derive a schema

```rust
#[derive(CommandArgs)]
#[command(description = "Ban a member", category = "moderation", alias = "block")]
struct BanArgs {
    #[arg(prompt = "Who should be banned?")]
    member: Mention,

    #[arg(long, short = 'r')]
    reason: Option<String>,

    #[arg(long, short = 'd', default = 0_u64)]
    delete_days: u64,

    #[arg(long, short = 'f')]
    force: bool,
}
```

Field documentation becomes help text unless `help = "..."` is supplied.
`Option<T>` is optional, `Vec<T>` is repeatable, and a `bool` with a long or
short option is a flag. A plain field is required unless it has a default.

Supported field attributes:

- `name`, `help`, `prompt`, and `value_name`;
- `long` or `long = "name"`;
- `short` or `short = 'n'`;
- `default` or `default = expression` for plain fields;
- `required = true|false`;
- `multiple` and `rest` on `Vec<T>`;
- `flag` on plain `bool`;
- `skip` for a `Default` field that is not part of parsing.

Conflicting combinations produce derive-time errors instead of malformed
runtime schemas.

## Command configuration

```rust
let command = BanArgs::command("moderation ban")
    .prefix("!")
    .case_insensitive()
    .example("!moderation ban @alice --reason spam")
    .completion(CompletionConfig::new());
```

`Router::mount("admin", routes)` prepends a command path to every command in the
mounted Router. It does not alter event routes.

## Accepted syntax

The parser recognizes:

```text
/search "rust async" --limit 20 -v tag-a tag-b
/search rust --limit=20
/search rust -vn20
/search -- -literal-positional
/scale -1.5
```

Single and double quotes, backslash escaping, empty quoted values, long-option
assignment, combined short flags, attached short values, and the `--` sentinel
are supported.

## Non-text arguments

Command tokenization preserves canonical segments:

```rust
#[derive(CommandArgs)]
struct UploadArgs {
    owner: Mention,
    file: File,
    #[arg(rest)]
    extra: Vec<MessageSegment>,
}
```

Implement `FromCommandValue` to parse a domain-specific value without changing
the command engine.

## Completion

Completion is opt-in per command. On `MissingArgument`, OxideBot registers an
exact conversation/actor session, sends the field prompt, and retries parsing.
A reply to a missing named option is inserted as that option. Cancellation
words and maximum rounds are bounded by `CompletionConfig`.

Type conversion failures, unknown options, and missing option values are not
completion candidates. They return an error plus the generated usage line.

## Automatic help

`Router::help()` adds `/help [command]`. The index and detail pages are generated
from mounted `Command` and `CommandSchema` values, so displayed aliases,
arguments, defaults, and examples use the same source as parsing.
