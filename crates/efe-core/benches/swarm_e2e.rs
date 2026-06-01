//! **Sprint C — End-to-end swarm benchmarks.**
//!
//! Run via:
//! ```bash
//! cargo bench --bench swarm_e2e
//! cargo bench --bench swarm_e2e -- --baseline v0.1.0   # vs v0.1.0
//! cargo bench --bench swarm_e2e --features accelerate  # BLAS path
//! ```
//!
//! ## Three sizes
//!
//! - **small**: 8 agents, dim=128, 4 anchors → SLO p95 < 50 ms
//! - **medium**: 32 agents, dim=256, 8 anchors → SLO p95 < 200 ms
//! - **large**: 128 agents, dim=256, 16 anchors → SLO p95 < 1 s
//!
//! ## Three axes
//!
//! - **Latency** : `swarm_e2e_<size>_latency` — p50, p95, p99
//! - **Throughput** : `swarm_e2e_<size>_throughput_Nc<n>` — req/s
//! - **BLAS comparison** : `swarm_e2e_blas_vs_naive` — speedup measure
//!
//! ## Caveats
//!
//! Benches use `BenchMock*` strategies (cf. `common/mod.rs`) that do
//! NOT execute real numerical work — they only stress the controller
//! plumbing (genesis allocation, round dispatch, telemetry, alignment
//! check, dissolution). To measure actual numerical throughput, see
//! `compute_sprint{2,3,4}.rs`. The combined picture (controller
//! overhead + compute) is the SLO target.

#![allow(clippy::cast_precision_loss)]

use std::sync::Arc;
use std::time::Duration;

use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput,
};

mod common;
use common::{build_bench_controller, build_request, load_realistic_spec};

// ====================================================================
// Latency benches
// ====================================================================

fn bench_e2e_latency(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("tokio runtime");

    for spec_name in ["small", "medium", "large"] {
        let spec = load_realistic_spec(spec_name);
        let controller = Arc::new(build_bench_controller(&spec, 1));
        let mut group = c.benchmark_group(format!("swarm_e2e_{spec_name}_latency"));
        // 50 samples is enough for stable p50/p95 with criterion's
        // built-in outlier rejection. Larger spec = more measurement
        // time to keep CI duration bounded.
        let (samples, time_secs) = match spec_name {
            "small" => (50, 15),
            "medium" => (40, 20),
            "large" => (20, 30),
            _ => unreachable!(),
        };
        group.sample_size(samples);
        group.measurement_time(Duration::from_secs(time_secs));
        group.bench_function("execute", |b| {
            let ctrl = controller.clone();
            let s = spec.clone();
            b.to_async(&rt).iter(|| {
                let ctrl = ctrl.clone();
                let s = s.clone();
                async move {
                    let req = build_request(&s);
                    ctrl.execute(black_box(req)).await.expect("execute ok")
                }
            });
        });
        group.finish();
    }
}

// ====================================================================
// Throughput benches
// ====================================================================

fn bench_e2e_throughput(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("tokio runtime");

    // Throughput with concurrency Nc ∈ {1, 4, 16}.
    // small + medium only — large is too slow to be meaningful in
    // a 30s measurement window with Nc>1 saturation.
    for &nc in &[1usize, 4, 16] {
        for spec_name in ["small", "medium"] {
            let spec = load_realistic_spec(spec_name);
            let controller = Arc::new(build_bench_controller(&spec, 1));
            let mut group = c.benchmark_group(format!(
                "swarm_e2e_{spec_name}_throughput"
            ));
            // Throughput::Elements(Nc) tells criterion to divide each
            // measured time by Nc to derive a per-element latency
            // (and a "elements/s" rate is computed automatically).
            group.throughput(Throughput::Elements(nc as u64));
            group.sample_size(20);
            group.measurement_time(Duration::from_secs(15));
            group.bench_with_input(
                BenchmarkId::from_parameter(format!("Nc{nc}")),
                &nc,
                |b, &nc| {
                    let ctrl = controller.clone();
                    let s = spec.clone();
                    b.to_async(&rt).iter(|| {
                        let ctrl = ctrl.clone();
                        let s = s.clone();
                        async move {
                            let mut handles = Vec::with_capacity(nc);
                            for _ in 0..nc {
                                let c2 = ctrl.clone();
                                let s2 = s.clone();
                                handles.push(tokio::spawn(async move {
                                    c2.execute(build_request(&s2))
                                        .await
                                        .expect("execute ok")
                                }));
                            }
                            for h in handles {
                                let _ = h.await.expect("join");
                            }
                        }
                    });
                },
            );
            group.finish();
        }
    }
}

// ====================================================================
// BLAS vs naïve comparison (medium spec, the sweet spot for
// detecting BLAS speedup on the pipeline)
// ====================================================================

/// Sentinel benchmark whose name encodes the active backend. Running
/// the suite twice (with and without `--features accelerate`) and
/// diffing the two reports gives the pipeline-level speedup.
///
/// Note: a single `cargo bench` invocation cannot exercise both
/// backends simultaneously (the feature gate is compile-time). The
/// driver script `scripts/bench_runner.sh` handles the dual run.
fn bench_e2e_blas_vs_naive(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("tokio runtime");
    let spec = load_realistic_spec("medium");
    let controller = Arc::new(build_bench_controller(&spec, 1));

    // The bench name includes the backend label so two runs (one per
    // build configuration) produce visibly distinct metrics in
    // criterion's output and in `BENCHMARKS.md`.
    let backend_label: &str = if cfg!(feature = "accelerate") {
        "accelerate"
    } else if cfg!(feature = "openblas") {
        "openblas"
    } else {
        "pure_rust"
    };

    let mut group = c.benchmark_group("swarm_e2e_blas_vs_naive");
    group.sample_size(40);
    group.measurement_time(Duration::from_secs(20));
    group.bench_function(format!("medium_{backend_label}"), |b| {
        let ctrl = controller.clone();
        let s = spec.clone();
        b.to_async(&rt).iter(|| {
            let ctrl = ctrl.clone();
            let s = s.clone();
            async move {
                ctrl.execute(black_box(build_request(&s)))
                    .await
                    .expect("execute ok")
            }
        });
    });
    group.finish();
}

// ====================================================================
// Criterion plumbing
// ====================================================================

criterion_group!(
    name = benches;
    config = Criterion::default()
        .significance_level(0.05)
        .noise_threshold(0.10);
    targets =
        bench_e2e_latency,
        bench_e2e_throughput,
        bench_e2e_blas_vs_naive
);
criterion_main!(benches);
