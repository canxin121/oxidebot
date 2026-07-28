use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
use oxidebot::{
    message, on, BotId, EventId, MessageContext, Outcome, OxideBot, PlatformId, RuntimeProfile,
};
use oxidebot_testkit::{ScriptStep, ScriptedAdapter, TestFrame};

const EVENTS_PER_SAMPLE: usize = 1_024;

fn benchmark_exact_command_runtime(c: &mut Criterion) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("benchmark runtime");
    let mut group = c.benchmark_group("exact_command_runtime");
    group.throughput(Throughput::Elements(EVENTS_PER_SAMPLE as u64));

    for route_count in [1_usize, 100, 1_000, 10_000] {
        group.bench_with_input(
            BenchmarkId::from_parameter(route_count),
            &route_count,
            |bencher, &route_count| {
                bencher.to_async(&runtime).iter_batched(
                    || {
                        let platform = PlatformId::new("bench").expect("static id");
                        let bot = BotId::new("bot").expect("static id");
                        let selected = route_count.saturating_sub(1);
                        let frames = (0..EVENTS_PER_SAMPLE).map(|event| {
                            ScriptStep::Frame(TestFrame::message(
                                EventId::new(format!("event:{route_count}:{event}"))
                                    .expect("bounded event id"),
                                event as u64,
                                "user",
                                1_u64,
                                format!("/route-{selected}"),
                            ))
                        });
                        let (adapter, _) = ScriptedAdapter::new(platform, bot, frames);
                        let mut app = OxideBot::new()
                            .profile(RuntimeProfile::Throughput)
                            .bot(adapter);
                        for route in 0..route_count {
                            app = app.handler(on(
                                message().command(format!("route-{route}")),
                                |_context: MessageContext| async { Ok(Outcome::continue_()) },
                            ));
                        }
                        app.build().expect("benchmark application builds")
                    },
                    |app| async move {
                        app.run_to_completion()
                            .await
                            .expect("benchmark application succeeds");
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

criterion_group!(benches, benchmark_exact_command_runtime);
criterion_main!(benches);
