use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
use oxidebot::{command, BotId, EventId, Module, Outcome, OxideBot, PlatformId, RuntimeProfile};
use oxidebot_testkit::{ScriptStep, ScriptedAdapter, TestFrame};
use std::hint::black_box;

const EVENTS_PER_SAMPLE: usize = 1_024;

fn build_exact_command_application(handler_count: usize) -> oxidebot_runtime::Application<()> {
    let platform = PlatformId::new("bench").expect("static id");
    let bot = BotId::new("bot").expect("static id");
    let selected = handler_count.saturating_sub(1);
    let frames = (0..EVENTS_PER_SAMPLE).map(|event| {
        ScriptStep::Frame(TestFrame::message(
            EventId::new(format!("event:{handler_count}:{event}")).expect("bounded event id"),
            event as u64,
            "user",
            1_u64,
            format!("/handler-{selected}"),
        ))
    });
    let (adapter, _) = ScriptedAdapter::new(platform, bot, frames);
    let mut features = Module::new();
    for handler in 0..handler_count {
        features = features.command(command(format!("handler-{handler}")), || async {
            Outcome::continue_()
        });
    }
    OxideBot::new()
        .profile(RuntimeProfile::Throughput)
        .adapter(adapter)
        .include(features)
        .build()
        .expect("benchmark application builds")
}

/// Separates route-table compilation and runtime resource construction from
/// transport execution. It is a startup benchmark, not event throughput.
fn benchmark_application_build(c: &mut Criterion) {
    let mut group = c.benchmark_group("application_build");
    for handler_count in [1_usize, 100, 1_000, 10_000] {
        group.bench_with_input(
            BenchmarkId::from_parameter(handler_count),
            &handler_count,
            |bencher, &handler_count| {
                bencher.iter(|| black_box(build_exact_command_application(handler_count)));
            },
        );
    }
    group.finish();
}

/// Measures a complete finite transport run: adapter startup, routing,
/// executor work, and shutdown. It is intentionally labelled end-to-end,
/// rather than presented as warm routing throughput.
fn benchmark_exact_command_end_to_end(c: &mut Criterion) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("benchmark runtime");
    let mut group = c.benchmark_group("exact_command_end_to_end");
    group.throughput(Throughput::Elements(EVENTS_PER_SAMPLE as u64));

    for handler_count in [1_usize, 100, 1_000, 10_000] {
        group.bench_with_input(
            BenchmarkId::from_parameter(handler_count),
            &handler_count,
            |bencher, &handler_count| {
                bencher.to_async(&runtime).iter_batched(
                    || build_exact_command_application(handler_count),
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

criterion_group!(
    benches,
    benchmark_application_build,
    benchmark_exact_command_end_to_end
);
criterion_main!(benches);
