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

async fn echo(Parsed(args): Parsed<EchoArgs>) -> String {
    std::iter::repeat_n(args.text.join(" "), args.times)
        .collect::<Vec<_>>()
        .join("\n")
}

async fn trace(request: Request, next: Next<()>) -> Response {
    let event_type = request.event_type();
    let response = next.run(request).await;
    println!("handled {event_type:?}; stopped={}", response.is_stopped());
    response
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

    let routes = Router::new()
        .command(
            command("ping").description("Check whether the bot is alive"),
            ping,
        )
        .command(EchoArgs::command("echo"), echo)
        .layer(from_fn(trace))
        .help();

    OxideBot::new()
        .bot(adapter)
        .router(routes)
        .run_to_completion()
        .await?;

    println!("sent {} message(s)", service.sent().len());
    Ok(())
}
