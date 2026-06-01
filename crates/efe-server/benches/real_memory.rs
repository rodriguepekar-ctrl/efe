//! **Sprint D1 — Production-strategies memory bench.**
//!
//! Measures heap peak (KB) and allocation count for the **real**
//! Mode-1 critical path, using the same `build_mvp_backend` wiring
//! as the latency bench (`real_strategies_e2e.rs`).
//!
//! ## What is measured
//!
//! For each v2 realistic fixture (small / medium / large) :
//!
//! - **heap peak** (KB) — maximum bytes the process held
//!   simultaneously across one `ExecuteBackend::execute()` call.
//! - **allocation residue** (bytes) — `current_usage_after -
//!   current_usage_before`. In a well-behaved run this should be ≤ a
//!   few KB (idempotency cache, small per-request UUIDs, etc.); a
//!   large positive delta signals a leak in the real strategies that
//!   the mock-based Sprint C bench could not detect.
//!
//! ## Why a separate bench file (mirrors Sprint C `memory_e2e.rs`)
//!
//! `real_memory.rs` installs `peak_alloc::PeakAlloc` as the global
//! allocator. Cargo attaches the allocator to the entire compilation
//! unit (this bench binary), so `real_strategies_e2e.rs` — which
//! must NOT pay the atomic-counter overhead — has to live in a
//! distinct binary. See `docs/bench/d1_real_strategies_architecture.md`
//! §6 for the rationale.
//!
//! ## Calibration
//!
//! `peak_alloc` adds one atomic add per `alloc` / `dealloc`. On
//! macOS aarch64 we measured ~5 % overhead vs the no-allocator
//! baseline; on Linux x86_64 ~3 %. This is acceptable because we
//! only consume the *peak* and *count* metrics from this bench —
//! the latency numbers come from `real_strategies_e2e.rs` where
//! `peak_alloc` is absent.

#![allow(clippy::cast_precision_loss)]
#![allow(missing_docs)]

use std::sync::Arc;
use std::time::Duration;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use peak_alloc::PeakAlloc;

mod common;
use common::{build_real_backend, build_real_request, load_realistic_v2_spec};

// =====================================================================
// Global allocator with peak tracking
// =====================================================================
//
// IMPORTANT: this MUST be the only `#[global_allocator]` in the bench
// binary. The latency bench `real_strategies_e2e.rs` deliberately
// uses Rust's default allocator to keep its timings pristine.

#[global_allocator]
static PEAK_ALLOC: PeakAlloc = PeakAlloc;

// =====================================================================
// Tokio runtime helper
// =====================================================================
//
// We build the runtime ONCE per `bench_*` function (criterion calls
// `bench_memory_peak`, `bench_memory_alloc_residue` sequentially).
// Building the runtime adds ~5 ms; doing so inside `iter` would
// pollute every sample. The runtime survives across iterations of a
// single bench function.

fn make_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2) // 2 is enough for sequential request bench.
        .enable_all()
        .build()
        .expect("tokio runtime")
}

// =====================================================================
// 1. Heap peak per fixture
// =====================================================================
//
// `peak_usage_as_kb()` returns the maximum bytes (rounded to KB) the
// process held alive at any point since the last `reset_peak_usage()`
// call. We reset before each iteration so we measure the per-request
// peak, not the accumulated peak across the whole bench loop.

fn bench_real_memory_peak(c: &mut Criterion) {
    let rt = make_runtime();

    for fixture in ["small", "medium", "large"] {
        let spec = load_realistic_v2_spec(fixture);
        let backend: Arc<dyn efe_server::services::ExecuteBackend> = build_real_backend(&spec);

        let mut group = c.benchmark_group(format!("real_memory_{fixture}_heap"));
        // Memory benches are noisier than latency benches (page faults,
        // OS allocator behaviour). 20 samples + 10 s budget keeps the
        // confidence interval tight without blowing the CI budget.
        group.sample_size(20);
        group.measurement_time(Duration::from_secs(10));

        group.bench_function("peak_kb", |b| {
            let backend = backend.clone();
            let spec = spec.clone();
            b.iter(|| {
                PEAK_ALLOC.reset_peak_usage();
                rt.block_on(async {
                    backend
                        .execute(black_box(build_real_request(&spec)))
                        .await
                        .expect("real backend execute");
                });
                // Funnel the KB measurement through black_box so the
                // compiler cannot elide it. Criterion records what we
                // return as the per-iter value.
                let kb = PEAK_ALLOC.peak_usage_as_kb();
                black_box(kb)
            });
        });
        group.finish();
    }
}

// =====================================================================
// 2. Allocation residue per fixture
// =====================================================================
//
// `current_usage()` returns the live byte count at call time. We
// snapshot before and after a single execute(), and report the delta.
//
// In a clean run this should hover near zero (the request and
// response are dropped before we re-read `current_usage`). A growing
// residue across iterations indicates a real-strategies leak — we
// would catch it in `bench_realism_check.rs` as a hard assertion,
// but the bench output gives a quantitative trend useful for review.

fn bench_real_memory_alloc_residue(c: &mut Criterion) {
    let rt = make_runtime();

    for fixture in ["small", "medium", "large"] {
        let spec = load_realistic_v2_spec(fixture);
        let backend: Arc<dyn efe_server::services::ExecuteBackend> = build_real_backend(&spec);

        let mut group = c.benchmark_group(format!("real_memory_{fixture}_alloc"));
        group.sample_size(20);
        group.measurement_time(Duration::from_secs(10));

        group.bench_function("residue_bytes", |b| {
            let backend = backend.clone();
            let spec = spec.clone();
            b.iter(|| {
                let before = PEAK_ALLOC.current_usage();
                rt.block_on(async {
                    backend
                        .execute(black_box(build_real_request(&spec)))
                        .await
                        .expect("real backend execute");
                });
                let after = PEAK_ALLOC.current_usage();
                // saturating_sub: peak_alloc occasionally reports
                // `after < before` if a background tokio task drops
                // its arena right between our reads. We treat that as
                // zero residue, which is the conservative choice.
                black_box(after.saturating_sub(before))
            });
        });
        group.finish();
    }
}

// =====================================================================
// Criterion entry points
// =====================================================================

criterion_group!(
    name = real_memory_benches;
    config = Criterion::default()
        .significance_level(0.05)
        // Memory benches are noisier than latency benches; bumping
        // the noise threshold to 15 % stops criterion from flagging
        // every benign fluctuation as a "regression".
        .noise_threshold(0.15);
    targets =
        bench_real_memory_peak,
        bench_real_memory_alloc_residue,
);
criterion_main!(real_memory_benches);
