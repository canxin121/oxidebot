use oxidebot::prelude::*;
use oxidebot_testkit::{ScriptStep, ScriptedAdapter, TestFrame};

#[oxidebot::command("ping")]
async fn ping() -> &'static str {
    "pong"
}

#[derive(Debug, CommandArgs)]
struct EchoArgs {
    /// Text to echo. Quotes and escaped spaces are preserved.
    #[arg(rest, required = true, prompt = "What should I echo?")]
    text: Vec<String>,

    /// Number of copies.
    #[arg(long, short = 'n', default = 1_usize, min = 1.0, max = 10.0)]
    times: usize,
}

#[derive(Debug, CommandArgs)]
struct AddArgs {
    #[arg(rest, required = true, prompt = "What should I add?")]
    text: Vec<String>,
}

#[derive(Debug, BotCommand)]
#[command(name = "tools", description = "Small example tools")]
#[expect(
    dead_code,
    reason = "the enum defines schemas consumed through generated branch tags"
)]
enum ToolsCommand {
    /// Echo text one or more times.
    Echo(EchoArgs),

    /// Demonstrate a second typed branch.
    Add(AddArgs),
}

async fn echo(BranchArgs(args): BranchArgs<tools_command_branches::Echo>) -> Message {
    Message::text(
        std::iter::repeat_n(args.text.join(" "), args.times)
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

async fn add(BranchArgs(args): BranchArgs<tools_command_branches::Add>) -> Message {
    Message::text(format!("added: {}", args.text.join(" ")))
}

async fn trace(context: Context, outcome: Outcome) -> Outcome {
    println!(
        "handled {:?}; propagation={:?}",
        context.event_type(),
        outcome.propagation(),
    );
    outcome
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> oxidebot::Result<()> {
    let platform = PlatformId::new("example").expect("static id");
    let bot = BotId::new("bot").expect("static id");
    let (adapter, service) = ScriptedAdapter::new(
        platform,
        bot,
        [ScriptStep::Frame(TestFrame::message(
            EventId::new("1").expect("static id"),
            "room",
            "user",
            1_u64,
            "/ping",
        ))],
    );

    let features = Module::new()
        .command(
            ping_command()
                .description("Check whether the bot is alive")
                .description_translation("zh-CN", "检查机器人是否在线"),
            ping,
        )
        .command_branch(tools_command_branches::Echo, echo)
        .command_branch(tools_command_branches::Add, add)
        .command_overlay(
            CommandOverlay::new("tools").shortcut(Shortcut::literal("repeat", "/tools echo")),
        )
        .after(trace)
        .help();

    OxideBot::new()
        .adapter(adapter)
        .include(features)
        .run_to_completion()
        .await?;

    println!("sent {} message(s)", service.sent().len());
    Ok(())
}
