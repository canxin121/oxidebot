use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use oxidebot_core::{BotId, EventId, MessageCreated, PlatformId};
use oxidebot_runtime::{message, on, Context, Outcome, OxideBot, RuntimeProfile};
use oxidebot_testkit::{ScriptStep, ScriptedAdapter, TestFrame};

fn benchmark_router(c: &mut Criterion) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("benchmark runtime");
    let mut group = c.benchmark_group("exact_command_dispatch");

    for route_count in [1_usize, 100, 1_000, 10_000] {
        group.bench_with_input(
            BenchmarkId::from_parameter(route_count),
            &route_count,
            |bencher, &route_count| {
                bencher.to_async(&runtime).iter(|| async move {
                    let platform = PlatformId::new("bench").expect("static id");
                    let bot = BotId::new("bot").expect("static id");
                    let selected = route_count.saturating_sub(1);
                    let (adapter, _) = ScriptedAdapter::new(
                        platform,
                        bot,
                        [ScriptStep::Frame(TestFrame::message(
                            EventId::new(format!("event:{route_count}")).expect("bounded event id"),
                            "room",
                            "user",
                            1_u64,
                            format!("/route-{selected}"),
                        ))],
                    );

                    let mut app = OxideBot::new().profile(RuntimeProfile::Eco).bot(adapter);
                    for route in 0..route_count {
                        app = app.handler(on(
                            message().command(format!("route-{route}")),
                            |_context: Context<MessageCreated>| async { Ok(Outcome::continue_()) },
                        ));
                    }
                    app.run_to_completion()
                        .await
                        .expect("benchmark application succeeds");
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, benchmark_router);
criterion_main!(benches);
