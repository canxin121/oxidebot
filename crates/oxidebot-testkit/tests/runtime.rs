use oxidebot::{
    event::tags, message, on, BotId, EventContext, EventId, MessageContext, Outcome, OxideBot,
    PlatformId, RuntimeMetrics,
};
use oxidebot_testkit::{ScriptStep, ScriptedAdapter, TestFrame};
use std::sync::Arc;

fn platform() -> PlatformId {
    PlatformId::new("test").expect("static platform is valid")
}

fn bot_id() -> BotId {
    BotId::new("bot").expect("static bot id is valid")
}

fn event_id(value: &str) -> EventId {
    EventId::new(value).expect("test event id is valid")
}

#[tokio::test]
async fn command_interest_skips_unrelated_full_decodes() {
    let ignored = TestFrame::message(event_id("ignored"), "room", "user", 1_u64, "/other");
    let ignored_decodes = ignored.decode_counter();
    let accepted = TestFrame::message(event_id("accepted"), "room", "user", 2_u64, "/ping");
    let accepted_decodes = accepted.decode_counter();
    let (adapter, service) = ScriptedAdapter::new(
        platform(),
        bot_id(),
        [ScriptStep::Frame(ignored), ScriptStep::Frame(accepted)],
    );

    OxideBot::new()
        .bot(adapter)
        .handler(on(
            message().command("ping"),
            |context: MessageContext| async move {
                assert_eq!(context.text(), "/ping");
                Ok(Outcome::stop().text("pong"))
            },
        ))
        .run_to_completion()
        .await
        .expect("runtime succeeds");

    assert_eq!(ignored_decodes.get(), 0);
    assert_eq!(accepted_decodes.get(), 1);
    assert_eq!(service.sent().len(), 1);
}

#[tokio::test]
async fn handlers_receive_the_original_018_event_model() {
    let (adapter, _) = ScriptedAdapter::new(
        platform(),
        bot_id(),
        [ScriptStep::Frame(TestFrame::message(
            event_id("event"),
            "room",
            "user",
            1_u64,
            "hello",
        ))],
    );

    OxideBot::new()
        .bot(adapter)
        .handler(on(
            oxidebot::event::<tags::Message>(),
            |context: EventContext<tags::Message>| async move {
                assert_eq!(context.event().sender.id, "user");
                assert_eq!(context.event().message.get_raw_text(), "hello");
                Ok(Outcome::continue_())
            },
        ))
        .run_to_completion()
        .await
        .expect("runtime succeeds");
}

#[tokio::test]
async fn metrics_still_report_indexed_dispatch() {
    let metrics = Arc::new(RuntimeMetrics::default());
    let (adapter, _) = ScriptedAdapter::new(
        platform(),
        bot_id(),
        [ScriptStep::Frame(TestFrame::message(
            event_id("event"),
            "room",
            "user",
            1_u64,
            "hello",
        ))],
    );

    OxideBot::new()
        .metrics(Arc::clone(&metrics))
        .bot(adapter)
        .handler(on(message(), |_context: MessageContext| async {
            Ok(Outcome::continue_())
        }))
        .run_to_completion()
        .await
        .expect("runtime succeeds");

    assert_eq!(metrics.snapshot().decoded_events, 1);
}
