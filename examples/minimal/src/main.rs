use oxidebot::prelude::*;
use oxidebot_testkit::{ScriptStep, ScriptedAdapter, TestFrame};

async fn ping() -> &'static str {
    "pong"
}

#[derive(Debug, CommandArgs)]
#[command(description = "Echo text one or more times", alias = "say")]
struct EchoArgs {
    /// Text to echo. Quotes and escaped spaces are preserved.
    #[arg(rest, required = true, prompt = "What should I echo?")]
    text: Vec<String>,

    /// Number of copies.
    #[arg(long, short = 'n', default = 1_usize)]
    times: usize,
}

async fn echo(Args(args): Args<EchoArgs>) -> String {
    std::iter::repeat_n(args.text.join(" "), args.times)
        .collect::<Vec<_>>()
        .join("\n")
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
            command("ping").description("Check whether the bot is alive"),
            ping,
        )
        .command(EchoArgs::command("echo"), echo)
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
