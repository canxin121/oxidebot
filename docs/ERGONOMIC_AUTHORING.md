# Ergonomic bot authoring

OxideBot keeps one event model, one message IR, one command IR, and one runtime. The ergonomic authoring layer is a set of compile-time adapters over those same primitives; it is not a second framework.

## Commands and command values

A small command is a feature value and is added once:

```rust
#[oxidebot::command(
    "deploy",
    alias = "ship",
    category = "operations",
    description = "commands.deploy.description",
)]
async fn deploy(
    #[arg(complete = complete_projects, resolve = resolve_project)] project: Project,
    #[arg(long, short = 'e')] environment: Environment,
    State(service): State<DeploymentService>,
    messenger: Messenger,
) -> HandlerResult<()> {
    let progress = messenger.reply("deploying …").await?;
    service.deploy(project, environment).await.internal("deploy project")?;
    progress.edit("deployed").await?;
    Ok(())
}

let features = Module::new().add(deploy);
```

Domain enums derive all command-facing metadata once:

```rust
#[derive(CommandValue)]
enum Environment {
    #[value(alias = "dev", label = "Development")]
    Development,
    #[value(alias = "stage", alias = "staging", label = "Staging")]
    Staging,
    #[value(alias = "prod", label = "Production")]
    Production,
}
```

The generated value parser, choices, help, completion, and native-command choices share the same declaration.

## Interactions

Interactions are first-class features. A returned message is converted to the platform interaction response instead of being sent as an unrelated conversation message. `Responder` shares the same acknowledgement state with automatic responses, so an interaction is acknowledged at most once.

```rust
#[oxidebot::interaction("deploy.confirm")]
async fn confirm(
    Action(payload): Action<DeployConfirmation>,
    responder: Responder,
    State(service): State<DeploymentService>,
) -> HandlerResult<()> {
    responder.defer().await?;
    service.confirm(payload.id).await.internal("confirm deployment")?;
    responder.edit_original("Deployment started").await?;
    Ok(())
}
```

The typed extractors cover button actions, select values, modal fields, and native command interactions. Use `Responder::defer`, `respond`, `update`, `edit_original`, `follow_up`, or `form_error` when explicit interaction control is required.

## Guards, hooks, and completers

All author functions use the same extractor model as handlers:

```rust
#[oxidebot::guard]
async fn can_deploy(
    Sender(user): Sender,
    State(permissions): State<PermissionService>,
) -> GuardDecision {
    permissions.check(&user.id, "deploy").await.into()
}

#[oxidebot::completer]
async fn complete_projects(
    State(projects): State<ProjectStore>,
    Sender(user): Sender,
    input: CompletionInput,
) -> HandlerResult<Vec<CompletionItem>> {
    projects.complete_for(&user.id, input.partial()).await
}
```

`before` and `after` hooks can likewise request only the values they use. Observer-style after hooks may borrow the final `Outcome` without rebuilding it.

Common rules are normal bounded guards: group-only, private-only, mention-required, reply-to-bot, allow/deny user sets, permission checks, and user/conversation cooldowns. They compose on a feature without a dynamic rule engine.

## Dialogue forms

Dialogue forms support localized prompt/error keys, optional fields, static or dynamic choices, safe field validation errors, conditional fields, and nested forms:

```rust
#[derive(DialogueForm)]
struct Setup {
    #[dialogue(prompt_key = "setup.project", choices = projects_for_user)]
    project: ProjectId,

    #[dialogue(prompt_key = "setup.advanced", confirm)]
    advanced: bool,

    #[dialogue(when = advanced, nested)]
    options: Option<AdvancedOptions>,
}

let setup = dialogue.named("setup").form::<Setup>().await?;
```

Every dynamic choice collection is bounded. Optional values can be skipped explicitly; invalid input is retried according to the field policy and only safe, localizable messages are shown to users.

## Delivery policy

Returning `Message` remains the default. Use a delivery wrapper or `Outcome` when one reply needs explicit fallback semantics:

```rust
async fn menu() -> Delivery<Message> {
    Delivery::strict(
        Message::text("Choose")
            .button_action("Confirm", "deploy.confirm"),
    )
}
```

Immediate and proactive sends go through `Messenger`; raw `Bot`/`CallApiTrait` remains the complete escape hatch.

## Lifecycle

Common lifecycle work does not require a custom service type:

```rust
OxideBot::with_state(state)
    .adapter(adapter)
    .add(deploy.guard(can_deploy))
    .help()
    .on_startup(initialize)
    .task(queue_worker)
    .interval(Duration::from_secs(60), check_jobs)
    .on_shutdown(save_state)
    .run()
    .await?;
```

Tasks receive bounded runtime context and shutdown signalling. The lower-level `Service` trait remains available for custom supervision.

## Tests

The high-level test scenario supports ordinary messages, button clicks, modal submissions, native commands, autocomplete requests, notices and requests. Expectations include replies, interaction acknowledgements, follow-ups, edits, deletes, reactions, and delivery degradation:

```rust
BotTest::feature(deploy)
    .message("/deploy oxidebot")
    .expect_reply_contains("Confirm")
    .click("deploy.confirm")
    .expect_interaction_ack()
    .expect_edit_contains("started")
    .run()
    .await?;
```

Use `ScriptedAdapter` directly only when a test needs protocol-level control.
