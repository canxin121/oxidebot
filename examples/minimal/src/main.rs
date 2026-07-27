use oxidebot::{
    message, on, BotId, Context, EventId, MessageCreated, Outcome, OxideBot, PlatformId,
};
use oxidebot_testkit::{ScriptStep, ScriptedAdapter, TestFrame};

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

    OxideBot::new()
        .bot(adapter)
        .handler(on(
            message().command("ping"),
            |_context: Context<MessageCreated>| async move { Ok(Outcome::stop().reply("pong")) },
        ))
        .run_to_completion()
        .await?;

    println!("sent {} message(s)", service.sent().len());
    Ok(())
}
