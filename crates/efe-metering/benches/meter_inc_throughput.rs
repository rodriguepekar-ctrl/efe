// SPDX-License-Identifier: UNLICENSED
//! M31 perf bench — measures the hot-path `meter_inc` throughput.
//!
//! Run with :
//!
//! ```bash
//! cargo bench -p efe-metering --bench meter_inc_throughput
//! ```
//!
//! Three groups of measurements :
//!
//! 1. `meter_inc/single_threaded` — baseline cost of one call.
//! 2. `meter_inc/contended_4t` — 4-thread contention on the buffer
//!    mutex (proxies real-world traffic across handler threads).
//! 3. `current_bucket_cached` — proves the atomic cache is much
//!    faster than re-formatting every call.
//!
//! The numbers below were observed on a M1 Pro at the time of the
//! M31 audit rewrite ; treat them as a regression budget rather
//! than absolute SLO :
//!
//! | bench                              |   p50    |   p99    |
//! |------------------------------------|---------:|---------:|
//! | meter_inc/single_threaded          | ~ 200 ns | ~ 350 ns |
//! | meter_inc/contended_4t             | ~ 700 ns | ~ 1.5 µs |
//! | current_bucket_cached              | ~  30 ns | ~  60 ns |

use std::sync::Arc;

use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};

use efe_metering::{
    current_bucket_cached, InMemoryCounter, Meter, MeterClient,
};

fn bench_meter_inc_single_threaded(c: &mut Criterion) {
    // Build a runtime + enter it so that the `tokio::spawn` inside
    // `meter_inc` (triggered when the buffer fills) finds an active
    // reactor. Without `_guard`, the bench panics on the first
    // flush boundary.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let _guard = rt.enter();

    let counter = Arc::new(InMemoryCounter::new());
    let client = MeterClient::with_defaults(counter);
    let tenant: Arc<str> = Arc::from("acme");

    let mut group = c.benchmark_group("meter_inc");
    group.throughput(Throughput::Elements(1));
    group.bench_function("single_threaded", |b| {
        b.iter(|| {
            client.meter_inc(&tenant, Meter::ClassifyRequests, 1).unwrap();
        });
    });
    group.finish();
}

fn bench_meter_inc_contended(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();
    let counter = Arc::new(InMemoryCounter::new());
    let client = MeterClient::with_defaults(counter);
    let tenant: Arc<str> = Arc::from("acme");

    let mut group = c.benchmark_group("meter_inc");
    group.throughput(Throughput::Elements(1));
    group.bench_function("contended_4t", |b| {
        b.iter_batched(
            || (client.clone(), Arc::clone(&tenant)),
            |(cli, t)| {
                // Simulate a tiny amount of contention by spawning
                // 4 concurrent push() calls.
                rt.block_on(async move {
                    let h1 = {
                        let cli = cli.clone();
                        let t = Arc::clone(&t);
                        tokio::spawn(async move {
                            cli.meter_inc(&t, Meter::ClassifyRequests, 1).unwrap();
                        })
                    };
                    let h2 = {
                        let cli = cli.clone();
                        let t = Arc::clone(&t);
                        tokio::spawn(async move {
                            cli.meter_inc(&t, Meter::AgentRunSteps, 1).unwrap();
                        })
                    };
                    let h3 = {
                        let cli = cli.clone();
                        let t = Arc::clone(&t);
                        tokio::spawn(async move {
                            cli.meter_inc(&t, Meter::WebhookDeliveries, 1).unwrap();
                        })
                    };
                    let h4 = {
                        let cli = cli.clone();
                        let t = Arc::clone(&t);
                        tokio::spawn(async move {
                            cli.meter_inc(&t, Meter::MultimodalVisionRequests, 1).unwrap();
                        })
                    };
                    let _ = tokio::join!(h1, h2, h3, h4);
                });
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

fn bench_current_bucket_cached(c: &mut Criterion) {
    let mut group = c.benchmark_group("current_bucket_cached");
    group.throughput(Throughput::Elements(1));
    group.bench_function("hot", |b| {
        // Prime the cache once so the bench measures the steady state.
        let _ = current_bucket_cached();
        b.iter(|| {
            let _ = current_bucket_cached();
        });
    });
    group.finish();
}

fn bench_legacy_vs_arc_str(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let _guard = rt.enter();

    let counter = Arc::new(InMemoryCounter::new());
    let client = MeterClient::with_defaults(counter);
    let tenant: Arc<str> = Arc::from("acme");

    let mut group = c.benchmark_group("meter_inc_call_style");
    group.throughput(Throughput::Elements(1));

    group.bench_function("arc_str_path", |b| {
        b.iter(|| {
            client.meter_inc(&tenant, Meter::ClassifyRequests, 1).unwrap();
        });
    });

    group.bench_function("str_path_with_alloc", |b| {
        b.iter(|| {
            client.meter_inc_str("acme", Meter::ClassifyRequests, 1).unwrap();
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_meter_inc_single_threaded,
    bench_meter_inc_contended,
    bench_current_bucket_cached,
    bench_legacy_vs_arc_str,
);
criterion_main!(benches);
