use oxidebot::{
    command, event::tags, BotId, EventContext, EventId, MessageContext, Module, Outcome, OxideBot,
    PlatformId, Responder, RuntimeMetrics,
};
use oxidebot_testkit::{BotTest, ScriptStep, ScriptedAdapter, TestFrame};
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

    let features = Module::new().command(command("ping"), |context: MessageContext| async move {
        assert_eq!(context.text(), "/ping");
        Outcome::new().text("pong")
    });

    OxideBot::new()
        .adapter(adapter)
        .include(features)
        .run_to_completion()
        .await
        .expect("runtime succeeds");

    assert_eq!(ignored_decodes.get(), 0);
    assert_eq!(accepted_decodes.get(), 1);
    assert_eq!(service.sent().len(), 1);
}

#[tokio::test]
async fn handlers_receive_the_canonical_event_model() {
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

    let features = Module::new().on(
        tags::Message,
        |context: EventContext<tags::Message>| async move {
            assert_eq!(context.event().sender.id, "user".into());
            assert_eq!(context.event().conversation.id, "room".into());
            assert_eq!(context.event().message.get_raw_text(), "hello");
            Outcome::continue_()
        },
    );

    OxideBot::new()
        .adapter(adapter)
        .include(features)
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

    let features = Module::new().message(|| async { Outcome::continue_() });

    OxideBot::new()
        .metrics(Arc::clone(&metrics))
        .adapter(adapter)
        .include(features)
        .run_to_completion()
        .await
        .expect("runtime succeeds");

    assert_eq!(metrics.snapshot().decoded_events, 1);
}

#[tokio::test]
async fn framework_replies_use_the_bounded_scheduler_and_retry_idempotently() {
    let metrics = Arc::new(RuntimeMetrics::default());
    let (adapter, service) = ScriptedAdapter::new(
        platform(),
        bot_id(),
        [ScriptStep::Frame(TestFrame::message(
            event_id("retry"),
            "room",
            "user",
            1_u64,
            "/ping",
        ))],
    );
    service.fail_temporarily(1);

    OxideBot::new()
        .metrics(Arc::clone(&metrics))
        .adapter(adapter)
        .include(Module::new().command(command("ping"), || async { "pong" }))
        .run_to_completion()
        .await
        .expect("temporary send failure is retried");

    assert_eq!(service.attempts(), 2);
    assert_eq!(service.sent_count(), 1);
    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.commands, 1);
    assert_eq!(snapshot.command_retries, 1);
    assert_eq!(snapshot.command_errors, 0);
    assert_eq!(snapshot.outbound_queue_depth, 0);
    assert_eq!(snapshot.outbound_in_flight, 0);
}

#[tokio::test]
async fn returned_interaction_message_is_an_ack_not_a_chat_message() {
    let module = Module::new().interaction("confirm", || async { "confirmed" });
    let report = BotTest::new(module)
        .click("confirm")
        .expect_interaction_ack()
        .run()
        .await
        .expect("interaction scenario succeeds");

    assert!(report.sent.is_empty());
    assert_eq!(report.interactions.len(), 1);
}

#[tokio::test]
async fn responder_defer_and_edit_share_one_acknowledgement_state() {
    let module = Module::new().interaction("confirm", |responder: Responder| async move {
        responder.defer().await?;
        responder.edit_original("started").await?;
        Ok::<(), oxidebot::HandlerError>(())
    });
    BotTest::new(module)
        .click("confirm")
        .expect_interaction_ack()
        .expect_edit_contains("started")
        .run()
        .await
        .expect("explicit interaction lifecycle succeeds");
}

#[tokio::test]
async fn delivery_middleware_panic_isolated_from_executor_and_application() {
    let metrics = Arc::new(RuntimeMetrics::default());
    let (adapter, service) = ScriptedAdapter::new(
        platform(),
        bot_id(),
        [
            ScriptStep::Frame(TestFrame::message(
                event_id("panic-1"),
                "room",
                "user",
                1_u64,
                "/ping",
            )),
            ScriptStep::Frame(TestFrame::message(
                event_id("panic-2"),
                "room",
                "user",
                2_u64,
                "/ping",
            )),
        ],
    );

    OxideBot::new()
        .metrics(Arc::clone(&metrics))
        .delivery_middleware(|_, _, _| async move {
            panic!("scripted delivery middleware panic");
        })
        .adapter(adapter)
        .include(Module::new().command(command("ping"), || async { "pong" }))
        .run_to_completion()
        .await
        .expect("delivery panic does not kill executor or application");

    assert_eq!(service.sent_count(), 0);
    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.handler_calls, 2);
    assert_eq!(snapshot.delivery_panics, 2);
    assert_eq!(snapshot.active_handlers, 0);
}

#[tokio::test]
async fn slow_interaction_is_deferred_before_deadline_then_uses_followup() {
    let module = Module::new().interaction("slow", || async {
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        "finished"
    });

    BotTest::new(module)
        .click_with_deadline("slow", std::time::Duration::from_millis(20))
        .expect_interaction_ack()
        .expect_followup_contains("finished")
        .run()
        .await
        .expect("slow interaction auto-defers and follows up");
}
