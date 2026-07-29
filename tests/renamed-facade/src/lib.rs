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
    let _ = std::mem::size_of::<Form>();
}
