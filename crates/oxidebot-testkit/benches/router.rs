use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
use oxidebot::{command, BotId, EventId, Module, Outcome, OxideBot, PlatformId, RuntimeProfile};
use oxidebot_testkit::{ScriptStep, ScriptedAdapter, TestFrame};

const EVENTS_PER_SAMPLE: usize = 1_024;

fn benchmark_exact_command_runtime(c: &mut Criterion) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("benchmark runtime");
    let mut group = c.benchmark_group("exact_command_runtime");
    group.throughput(Throughput::Elements(EVENTS_PER_SAMPLE as u64));

    for handler_count in [1_usize, 100, 1_000, 10_000] {
        group.bench_with_input(
            BenchmarkId::from_parameter(handler_count),
            &handler_count,
            |bencher, &handler_count| {
                bencher.to_async(&runtime).iter_batched(
                    || {
                        let platform = PlatformId::new("bench").expect("static id");
                        let bot = BotId::new("bot").expect("static id");
                        let selected = handler_count.saturating_sub(1);
                        let frames = (0..EVENTS_PER_SAMPLE).map(|event| {
                            ScriptStep::Frame(TestFrame::message(
                                EventId::new(format!("event:{handler_count}:{event}"))
                                    .expect("bounded event id"),
                                event as u64,
                                "user",
                                1_u64,
                                format!("/handler-{selected}"),
                            ))
                        });
                        let (adapter, _) = ScriptedAdapter::new(platform, bot, frames);
                        let mut features = Module::new();
                        for handler in 0..handler_count {
                            features = features
                                .command(command(format!("handler-{handler}")), || async {
                                    Outcome::continue_()
                                });
                        }
                        OxideBot::new()
                            .profile(RuntimeProfile::Throughput)
                            .adapter(adapter)
                            .include(features)
                            .build()
                            .expect("benchmark application builds")
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
