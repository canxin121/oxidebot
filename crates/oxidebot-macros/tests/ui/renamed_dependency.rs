#![allow(
    dead_code,
    reason = "trybuild validates generated types without running an application"
)]

extern crate self as oxidebot;

pub mod runtime {
    pub use oxidebot_runtime::*;
}
pub use oxidebot_runtime::*;

use oxidebot_macros::{command, BotState, CommandArgs, DialogueForm};

#[derive(CommandArgs)]
struct EchoArgs {
    #[arg(required = true)]
    text: String,
}

#[command("echo")]
async fn echo() -> String {
    "hello".into()
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

fn main() {
    let _module: Module<StateRoot> = Module::new().add(echo);
    let _ = <EchoArgs as oxidebot::CommandArgs>::schema();
}
