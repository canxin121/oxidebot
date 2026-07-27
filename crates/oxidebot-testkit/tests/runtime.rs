use async_trait::async_trait;
use oxidebot_core::{
    BotId, BotSlot, ConversationKey, EventBatch, EventBody, EventDraft, EventId, EventIndex,
    EventKind, MessageContent, MessageCreated, MessageOptions, MessageRef, MessageTarget,
    OutgoingMessage, PlatformId, UserKey,
};
use oxidebot_runtime::{
    message, on, Adapter, AdapterContext, AdapterError, AskOptions, BotDescriptor, BotServices,
    CommandError, Context, DecodeError, FrameIndex, InboundFrame, Outcome, OverloadPolicy,
    OxideBot, QueueBudget, RuntimeConfig, RuntimeError, RuntimeMetrics, RuntimeProfile, Service,
    ServiceContext, ServiceError,
};
use oxidebot_testkit::{ScriptStep, ScriptedAdapter, TestFrame};
use std::{
    future::pending,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime},
};

fn platform() -> PlatformId {
    PlatformId::new("test").expect("static platform is valid")
}

fn bot_id() -> BotId {
    BotId::new("bot").expect("static bot id is valid")
}

fn event_id(value: impl Into<Arc<str>>) -> EventId {
    EventId::new(value).expect("test event id is non-empty")
}

#[tokio::test]
async fn interest_gate_skips_full_decode_for_unrelated_commands() {
    let ignored = TestFrame::message(event_id("ignored"), "room", "user", 1_u64, "/other");
    let ignored_decodes = ignored.decode_counter();
    let accepted = TestFrame::message(event_id("accepted"), "room", "user", 2_u64, "/ping");
    let accepted_decodes = accepted.decode_counter();
    let (adapter, service) = ScriptedAdapter::new(
        platform(),
        bot_id(),
        [ScriptStep::Frame(ignored), ScriptStep::Frame(accepted)],
    );
    let metrics = Arc::new(RuntimeMetrics::default());

    OxideBot::new()
        .metrics(metrics.clone())
        .bot(adapter)
        .handler(on(
            message().command("ping"),
            |_context: Context<MessageCreated>| async move { Ok(Outcome::stop().reply("pong")) },
        ))
        .run_to_completion()
        .await
        .expect("runtime succeeds");

    assert_eq!(ignored_decodes.get(), 0);
    assert_eq!(accepted_decodes.get(), 1);
    assert_eq!(service.sent().len(), 1);
    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.ignored_frames, 1);
    assert_eq!(snapshot.decoded_events, 1);
}

#[tokio::test]
async fn compiled_router_only_visits_indexed_candidates() {
    let (adapter, _) = ScriptedAdapter::new(
        platform(),
        bot_id(),
        [ScriptStep::Frame(TestFrame::message(
            event_id("event"),
            "room",
            "user",
            1_u64,
            "/route-73",
        ))],
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let metrics = Arc::new(RuntimeMetrics::default());
    let mut app = OxideBot::new().metrics(metrics.clone()).bot(adapter);
    for index in 0..1_000 {
        let calls = calls.clone();
        app = app.handler(on(
            message().command(format!("route-{index}")),
            move |_context: Context<MessageCreated>| {
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::Relaxed);
                    Ok(Outcome::continue_())
                }
            },
        ));
    }

    app.run_to_completion().await.expect("runtime succeeds");
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(metrics.snapshot().route_candidates, 1);
}

#[tokio::test]
async fn inactive_sessions_do_not_round_trip_through_session_workers() {
    let frames = (0_u64..32).map(|value| {
        ScriptStep::Frame(TestFrame::message(
            event_id(format!("event:{value}")),
            "room",
            "user",
            value,
            value.to_string(),
        ))
    });
    let (adapter, _) = ScriptedAdapter::new(platform(), bot_id(), frames);
    let metrics = Arc::new(RuntimeMetrics::default());

    OxideBot::new()
        .metrics(metrics.clone())
        .bot(adapter)
        .handler(on(message(), |_context: Context<MessageCreated>| async {
            Ok(Outcome::continue_())
        }))
        .run_to_completion()
        .await
        .expect("runtime succeeds");

    assert_eq!(metrics.snapshot().session_fast_misses, 32);
}

#[tokio::test]
async fn one_conversation_is_strictly_ordered_end_to_end() {
    let frames = (0_u64..20).map(|value| {
        ScriptStep::Frame(TestFrame::message(
            event_id(format!("event:{value}")),
            "room",
            "user",
            value,
            value.to_string(),
        ))
    });
    let (adapter, service) = ScriptedAdapter::new(platform(), bot_id(), frames);
    let handled = Arc::new(Mutex::new(Vec::new()));
    let handled_for_route = handled.clone();

    OxideBot::new()
        .bot(adapter)
        .handler(on(message(), move |context: Context<MessageCreated>| {
            let handled = handled_for_route.clone();
            async move {
                let value = context
                    .text()
                    .expect("message has text")
                    .parse::<u64>()
                    .expect("text is numeric");
                tokio::time::sleep(Duration::from_millis(20 - value.min(19))).await;
                handled
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push(value);
                Ok(Outcome::continue_().reply(value.to_string()))
            }
        }))
        .run_to_completion()
        .await
        .expect("runtime succeeds");

    assert_eq!(
        *handled.lock().unwrap_or_else(|error| error.into_inner()),
        (0_u64..20).collect::<Vec<_>>()
    );
    let outbound = service
        .sent()
        .into_iter()
        .map(|record| match record.message.content.as_slice() {
            [MessageContent::Text(text)] => text.parse::<u64>().expect("numeric reply"),
            _ => panic!("expected one text item"),
        })
        .collect::<Vec<_>>();
    assert_eq!(outbound, (0_u64..20).collect::<Vec<_>>());
}

#[tokio::test]
async fn different_conversations_run_concurrently_inside_one_fixed_shard() {
    let (adapter, _) = ScriptedAdapter::new(
        platform(),
        bot_id(),
        [
            ScriptStep::Frame(TestFrame::message(
                event_id("a"),
                "room-a",
                "user-a",
                1_u64,
                "a",
            )),
            ScriptStep::Frame(TestFrame::message(
                event_id("b"),
                "room-b",
                "user-b",
                2_u64,
                "b",
            )),
        ],
    );
    let current = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let current_route = current.clone();
    let maximum_route = maximum.clone();

    OxideBot::new()
        .profile(RuntimeProfile::Eco)
        .bot(adapter)
        .handler(on(message(), move |_context: Context<MessageCreated>| {
            let current = current_route.clone();
            let maximum = maximum_route.clone();
            async move {
                let active = current.fetch_add(1, Ordering::SeqCst) + 1;
                maximum.fetch_max(active, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(40)).await;
                current.fetch_sub(1, Ordering::SeqCst);
                Ok(Outcome::continue_())
            }
        }))
        .run_to_completion()
        .await
        .expect("runtime succeeds");

    assert_eq!(maximum.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn exact_session_consumes_response_before_normal_routes() {
    let (adapter, service) = ScriptedAdapter::new(
        platform(),
        bot_id(),
        [
            ScriptStep::Frame(TestFrame::message(
                event_id("ask"),
                "room",
                "user",
                1_u64,
                "/ask",
            )),
            ScriptStep::Pause(Duration::from_millis(50)),
            ScriptStep::Frame(TestFrame::message(
                event_id("answer"),
                "room",
                "user",
                2_u64,
                "42",
            )),
        ],
    );
    let generic_calls = Arc::new(AtomicUsize::new(0));
    let generic_calls_route = generic_calls.clone();

    OxideBot::new()
        .bot(adapter)
        .handler(on(
            message().command("ask"),
            |context: Context<MessageCreated>| async move {
                let answer: u64 = context
                    .ask_parse("send a number", AskOptions::new(Duration::from_secs(1)))
                    .await?;
                Ok(Outcome::continue_().reply(format!("answer={answer}")))
            },
        ))
        .handler(on(message(), move |_context: Context<MessageCreated>| {
            let calls = generic_calls_route.clone();
            async move {
                calls.fetch_add(1, Ordering::Relaxed);
                Ok(Outcome::continue_())
            }
        }))
        .run_to_completion()
        .await
        .expect("runtime succeeds");

    assert_eq!(generic_calls.load(Ordering::Relaxed), 1);
    let sent = service.sent();
    assert_eq!(sent.len(), 2);
}

#[tokio::test]
async fn duplicate_event_ids_are_removed_before_dispatch() {
    let duplicate = event_id("same-id");
    let (adapter, _) = ScriptedAdapter::new(
        platform(),
        bot_id(),
        [
            ScriptStep::Frame(TestFrame::message(
                duplicate.clone(),
                "room",
                "user",
                1_u64,
                "first",
            )),
            ScriptStep::Frame(TestFrame::message(
                duplicate, "room", "user", 2_u64, "second",
            )),
        ],
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let route_calls = calls.clone();
    let metrics = Arc::new(RuntimeMetrics::default());

    OxideBot::new()
        .metrics(metrics.clone())
        .bot(adapter)
        .handler(on(message(), move |_context: Context<MessageCreated>| {
            let calls = route_calls.clone();
            async move {
                calls.fetch_add(1, Ordering::Relaxed);
                Ok(Outcome::continue_())
            }
        }))
        .run_to_completion()
        .await
        .expect("runtime succeeds");

    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(metrics.snapshot().duplicate_events, 1);
}

#[tokio::test]
async fn handler_panic_isolated_and_next_event_on_same_key_runs() {
    let (adapter, _) = ScriptedAdapter::new(
        platform(),
        bot_id(),
        [
            ScriptStep::Frame(TestFrame::message(
                event_id("panic"),
                "room",
                "user",
                1_u64,
                "panic",
            )),
            ScriptStep::Frame(TestFrame::message(
                event_id("after"),
                "room",
                "user",
                2_u64,
                "after",
            )),
        ],
    );
    let completed = Arc::new(AtomicUsize::new(0));
    let completed_route = completed.clone();
    let metrics = Arc::new(RuntimeMetrics::default());

    OxideBot::new()
        .metrics(metrics.clone())
        .bot(adapter)
        .handler(on(message(), move |context: Context<MessageCreated>| {
            let completed = completed_route.clone();
            async move {
                if context.text() == Some("panic") {
                    panic!("synthetic handler panic");
                }
                completed.fetch_add(1, Ordering::Relaxed);
                Ok(Outcome::continue_())
            }
        }))
        .run_to_completion()
        .await
        .expect("runtime succeeds");

    assert_eq!(completed.load(Ordering::Relaxed), 1);
    assert_eq!(metrics.snapshot().handler_panics, 1);
}

#[tokio::test]
async fn idempotent_message_retries_temporary_platform_failures() {
    let (adapter, service) = ScriptedAdapter::new(
        platform(),
        bot_id(),
        [ScriptStep::Frame(TestFrame::message(
            event_id("retry"),
            "room",
            "user",
            1_u64,
            "retry",
        ))],
    );
    service.fail_temporarily(2);

    OxideBot::new()
        .profile(RuntimeProfile::Eco)
        .bot(adapter)
        .handler(on(
            message(),
            |context: Context<MessageCreated>| async move {
                let message = OutgoingMessage::text("done").options(MessageOptions {
                    idempotency_key: Some(Arc::from("retry-1")),
                    ..MessageOptions::default()
                });
                context.reply(message).await?;
                Ok(Outcome::continue_())
            },
        ))
        .run_to_completion()
        .await
        .expect("runtime succeeds");

    assert_eq!(service.attempts(), 3);
    assert_eq!(service.sent().len(), 1);
}

#[tokio::test]
async fn bot_handle_rejects_cross_bot_targets_before_enqueue() {
    let (adapter, service) = ScriptedAdapter::new(
        platform(),
        bot_id(),
        [ScriptStep::Frame(TestFrame::message(
            event_id("wrong-bot"),
            "room",
            "user",
            1_u64,
            "message",
        ))],
    );
    let rejected = Arc::new(AtomicBool::new(false));
    let rejected_route = rejected.clone();

    OxideBot::new()
        .bot(adapter)
        .handler(on(message(), move |context: Context<MessageCreated>| {
            let rejected = rejected_route.clone();
            async move {
                let result = context
                    .bot()
                    .send(
                        MessageTarget::new(ConversationKey::new(BotSlot(99), "elsewhere")),
                        "nope",
                    )
                    .await;
                rejected.store(
                    matches!(result, Err(CommandError::WrongBot)),
                    Ordering::Release,
                );
                Ok(Outcome::continue_())
            }
        }))
        .run_to_completion()
        .await
        .expect("runtime succeeds");

    assert!(rejected.load(Ordering::Acquire));
    assert!(service.sent().is_empty());
}

struct CancellationProbe {
    started: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
}

#[async_trait]
impl Service<()> for CancellationProbe {
    async fn run(&self, context: ServiceContext<()>) -> std::result::Result<(), ServiceError> {
        self.started.store(true, Ordering::Release);
        context.shutdown().cancelled().await;
        self.stopped.store(true, Ordering::Release);
        Ok(())
    }
}

#[tokio::test]
async fn managed_service_is_cancelled_and_drained_after_finite_adapters() {
    let (adapter, _) = ScriptedAdapter::new(platform(), bot_id(), []);
    let started = Arc::new(AtomicBool::new(false));
    let stopped = Arc::new(AtomicBool::new(false));

    OxideBot::new()
        .bot(adapter)
        .service(CancellationProbe {
            started: started.clone(),
            stopped: stopped.clone(),
        })
        .run_to_completion()
        .await
        .expect("runtime succeeds");

    assert!(started.load(Ordering::Acquire));
    assert!(stopped.load(Ordering::Acquire));
}

#[tokio::test]
async fn tiny_blocking_budgets_backpressure_without_losing_events() {
    let frames = (0_u64..32).map(|value| {
        ScriptStep::Frame(TestFrame::message(
            event_id(format!("bounded:{value}")),
            "room",
            "user",
            value,
            value.to_string(),
        ))
    });
    let (adapter, _) = ScriptedAdapter::new(platform(), bot_id(), frames);
    let calls = Arc::new(AtomicUsize::new(0));
    let route_calls = calls.clone();
    let mut config = RuntimeConfig::for_profile(RuntimeProfile::Eco);
    config.ingress = QueueBudget::new(2, 8 * 1024);
    config.executor = QueueBudget::new(2, 8 * 1024);
    config.executor_per_bot = QueueBudget::new(2, 8 * 1024);
    config.command = QueueBudget::new(2, 8 * 1024);

    OxideBot::new()
        .config(config)
        .bot(adapter)
        .handler(on(message(), move |_context: Context<MessageCreated>| {
            let calls = route_calls.clone();
            async move {
                tokio::time::sleep(Duration::from_millis(2)).await;
                calls.fetch_add(1, Ordering::Relaxed);
                Ok(Outcome::continue_())
            }
        }))
        .run_to_completion()
        .await
        .expect("runtime succeeds");

    assert_eq!(calls.load(Ordering::Relaxed), 32);
}

#[tokio::test]
async fn drop_newest_is_explicit_and_observable() {
    let frames = (0_u64..32).map(|value| {
        ScriptStep::Frame(TestFrame::message(
            event_id(format!("drop:{value}")),
            format!("room-{value}"),
            "user",
            value,
            value.to_string(),
        ))
    });
    let (adapter, _) = ScriptedAdapter::new(platform(), bot_id(), frames);
    let calls = Arc::new(AtomicUsize::new(0));
    let route_calls = calls.clone();
    let metrics = Arc::new(RuntimeMetrics::default());
    let mut config = RuntimeConfig::for_profile(RuntimeProfile::Eco);
    config.executor = QueueBudget::new(1, 8 * 1024);
    config.executor_per_bot = QueueBudget::new(1, 8 * 1024);
    config.executor_overload = OverloadPolicy::DropNewest;

    OxideBot::new()
        .config(config)
        .metrics(metrics.clone())
        .bot(adapter)
        .handler(on(message(), move |_context: Context<MessageCreated>| {
            let calls = route_calls.clone();
            async move {
                tokio::time::sleep(Duration::from_millis(20)).await;
                calls.fetch_add(1, Ordering::Relaxed);
                Ok(Outcome::continue_())
            }
        }))
        .run_to_completion()
        .await
        .expect("runtime succeeds");

    assert!(calls.load(Ordering::Relaxed) < 32);
    assert!(metrics.snapshot().dropped_events > 0);
}

struct MalformedFrame;

impl InboundFrame for MalformedFrame {
    fn index(&self, bot: BotSlot, platform: &PlatformId) -> Result<FrameIndex, DecodeError> {
        let mut index = EventIndex::new(bot, platform.clone(), EventKind::MessageCreated);
        index.conversation = Some(ConversationKey::new(bot, "indexed-room"));
        index.actor = Some(UserKey::new(bot, "user"));
        Ok(FrameIndex::one(index, 8 * 1024))
    }

    fn decode(self, bot: BotSlot, platform: &PlatformId) -> Result<EventBatch, DecodeError> {
        let mut index = EventIndex::new(bot, platform.clone(), EventKind::MessageCreated);
        index.conversation = Some(ConversationKey::new(bot, "indexed-room"));
        index.actor = Some(UserKey::new(bot, "user"));
        Ok(EventBatch::new([EventDraft {
            id: event_id("malformed"),
            index,
            occurred_at: Some(SystemTime::now()),
            delivery_attempt: 0,
            body: EventBody::MessageCreated(Box::new(MessageCreated {
                reference: MessageRef::new(ConversationKey::new(bot, "other-room"), 1_u64),
                sender: Some(UserKey::new(bot, "user")),
                content: vec![MessageContent::Text(Arc::from("bad"))],
                text: Some(Arc::from("bad")),
                mentioned_bot: false,
            })),
        }]))
    }
}

struct MalformedAdapter;

#[async_trait]
impl Adapter for MalformedAdapter {
    fn descriptor(&self) -> BotDescriptor {
        BotDescriptor::new(platform(), bot_id())
    }

    fn services(&self) -> BotServices {
        let (_, service) = ScriptedAdapter::new(platform(), bot_id(), []);
        BotServices::messages(Arc::new(service))
    }

    fn mode(&self) -> oxidebot_runtime::AdapterMode {
        oxidebot_runtime::AdapterMode::Finite
    }

    async fn run(self: Box<Self>, context: AdapterContext) -> Result<(), AdapterError> {
        context.submit(MalformedFrame).await?;
        Ok(())
    }
}

#[tokio::test]
async fn adapter_body_must_match_its_routing_index() {
    let result = OxideBot::new()
        .bot(MalformedAdapter)
        .handler(on(message(), |_context: Context<MessageCreated>| async {
            Ok(Outcome::continue_())
        }))
        .run_to_completion()
        .await;
    assert!(matches!(result, Err(RuntimeError::Adapter(_))));
}

struct EarlyPersistentAdapter;

#[async_trait]
impl Adapter for EarlyPersistentAdapter {
    fn descriptor(&self) -> BotDescriptor {
        BotDescriptor::new(platform(), bot_id())
    }

    fn services(&self) -> BotServices {
        let (_, service) = ScriptedAdapter::new(platform(), bot_id(), []);
        BotServices::messages(Arc::new(service))
    }

    async fn run(self: Box<Self>, _context: AdapterContext) -> Result<(), AdapterError> {
        Ok(())
    }
}

#[tokio::test]
async fn persistent_adapter_normal_exit_is_fatal() {
    let result = OxideBot::new()
        .bot(EarlyPersistentAdapter)
        .build()
        .expect("build")
        .run_until(pending())
        .await;
    assert!(matches!(result, Err(RuntimeError::Channel(_))));
}
