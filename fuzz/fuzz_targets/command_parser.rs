#![no_main]

use libfuzzer_sys::fuzz_target;
use oxidebot_core::Message;
use oxidebot_runtime::{
    command, ArgumentAction, ArgumentChoice, ArgumentGroup, ArgumentSpec, Command,
    CommandBranch, CommandSchema,
};
use std::sync::OnceLock;

fn grammar() -> &'static Command {
    static COMMAND: OnceLock<Command> = OnceLock::new();
    COMMAND.get_or_init(|| {
        command("fuzz")
            .global_schema(
                CommandSchema::new()
                    .argument(ArgumentSpec::new("project").long("project"))
                    .argument(
                        ArgumentSpec::new("verbose")
                            .short('v')
                            .action(ArgumentAction::Count),
                    ),
            )
            .subcommand(
                CommandBranch::new("add").schema(
                    CommandSchema::new()
                        .argument(ArgumentSpec::new("title"))
                        .argument(
                            ArgumentSpec::new("priority")
                                .long("priority")
                                .choice(ArgumentChoice::new("Low", "low"))
                                .choice(ArgumentChoice::new("High", "high")),
                        ),
                ),
            )
            .subcommand(
                CommandBranch::new("login").schema(
                    CommandSchema::new()
                        .argument(ArgumentSpec::new("token").long("token").required(false))
                        .argument(ArgumentSpec::new("cookie").long("cookie").required(false))
                        .group(
                            ArgumentGroup::new("credential")
                                .arguments(["token", "cookie"])
                                .exactly_one(),
                        ),
                ),
            )
    })
}

fuzz_target!(|data: &[u8]| {
    if let Ok(input) = std::str::from_utf8(data) {
        let _ = grammar().parse_message(&Message::text(input));
    }
});
