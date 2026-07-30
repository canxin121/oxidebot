//! Compile-time smoke crate that consumes OxideBot through a renamed facade dependency.

#![allow(
    dead_code,
    reason = "this crate exists only to compile public macros through a renamed dependency"
)]

use ob::prelude::*;

#[derive(CommandArgs)]
struct EchoArgs {
    #[arg(required = true)]
    text: String,
}

#[derive(CommandEnum)]
enum Environment {
    #[choice(value = "development", alias = "dev")]
    Development,
    #[choice(value = "production", alias = "prod")]
    Production,
}

#[derive(CommandArgs)]
#[command(interactive, group(name = "credential", exactly_one))]
struct DeployArgs {
    #[arg(value_enum)]
    environment: Environment,
    #[arg(long, group = "credential")]
    token: Option<String>,
    #[arg(long, group = "credential")]
    cookie: Option<String>,
}

#[derive(CommandArgs)]
struct TodoGlobalArgs {
    #[arg(long)]
    project: Option<String>,
}

#[derive(CommandArgs)]
struct TodoAddArgs {
    title: String,
}

#[derive(BotCommand)]
#[command(name = "todo", global_args = TodoGlobalArgs, interactive)]
enum TodoCommand {
    Add(TodoAddArgs),
}

#[ob::command("echo")]
async fn echo(#[arg(required = true)] text: String) -> String {
    text
}

#[derive(BotState)]
struct StateRoot {
    #[state]
    value: String,
}

#[derive(DialogueForm)]
struct Form {
    #[dialogue(prompt = "Name?")]
    name: String,
}

#[test]
fn renamed_facade_dependency_supports_all_public_macros() {
    let _module: Module<StateRoot> = Module::new().add(echo);
    let _ = <EchoArgs as ob::runtime::CommandArgs>::schema();
    let _ = <DeployArgs as ob::runtime::CommandArgs>::schema();
    let _ = <TodoCommand as ob::runtime::CommandTree>::command();
    assert!(matches!(
        <Environment as ob::runtime::FromCommandValue>::from_command_value(
            ob::runtime::CommandValue::Text("prod".into()),
        ),
        Ok(Environment::Production)
    ));
    let _ = std::mem::size_of::<Form>();
}
