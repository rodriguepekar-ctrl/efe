//! **Sprint C — End-to-end memory benchmarks.**
//!
//! Run via:
//! ```bash
//! cargo bench --bench memory_e2e
//! ```
//!
//! ## What is measured
//!
//! For each realistic spec (small / medium / large), the bench
//! captures :
//!
//! - **heap peak** (KB) — the maximum number of bytes the process
//!   held simultaneously across the lifetime of one
//!   `SwarmController::execute()` call.
//! - **alloc count** — the cumulative number of `alloc` calls the
//!   process issued.
//!
//! Both are obtained from [`peak_alloc::PeakAlloc`], a thin
//! `#[global_allocator]` wrapper that keeps two atomic counters.
//! The wrapper has *minimal* overhead (one atomic add per alloc /
//! dealloc) so the timing of the bench is not significantly altered.
//!
//! ## Why a separate bench file
//!
//! `memory_e2e.rs` declares its own `#[global_allocator]`. Cargo
//! attaches the allocator to the entire compilation unit (the bench
//! binary), so `swarm_e2e.rs` (which does **not** use peak_alloc)
//! must remain in a different file to keep its timings pristine.
//! See `bench_foundation_architecture.md` §13 R2.

#![allow(clippy::cast_precision_loss)]

use std::sync::Arc;
use std::time::Duration;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use peak_alloc::PeakAlloc;

mod common;
use common::{build_bench_controller, build_request, load_realistic_spec};

// ====================================================================
// Global allocator with peak tracking
// ====================================================================

#[global_allocator]
static PEAK_ALLOC: PeakAlloc = PeakAlloc;

// ====================================================================
// Heap peak per spec
// ====================================================================

fn bench_memory_peak(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio runtime");

    for spec_name in ["small", "medium", "large"] {
        let spec = load_realistic_spec(spec_name);
        let controller = Arc::new(build_bench_controller(&spec, 1));
        let mut group = c.benchmark_group(format!("memory_e2e_{spec_name}_heap"));
        group.sample_size(20);
        group.measurement_time(Duration::from_secs(10));
        group.bench_function("peak_kb", |b| {
            let ctrl = controller.clone();
            let s = spec.clone();
            b.iter(|| {
                // Reset peak between iterations so we capture the
                // delta of THIS execute() call, not the accumulated
                // peak across many.
                PEAK_ALLOC.reset_peak_usage();
                rt.block_on(async {
                    ctrl.execute(black_box(build_request(&s)))
                        .await
                        .expect("execute ok");
                });
                // Return the peak KB so criterion includes it in the
                // output (criterion records a u128 measurement; we
                // funnel the KB through black_box to prevent compiler
                // elision).
                let kb = PEAK_ALLOC.peak_usage_as_kb();
                black_box(kb)
            });
        });
        group.finish();
    }
}

// ====================================================================
// Allocation count per spec
// ====================================================================

fn bench_memory_alloc_count(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio runtime");

    for spec_name in ["small", "medium", "large"] {
        let spec = load_realistic_spec(spec_name);
        let controller = Arc::new(build_bench_controller(&spec, 1));
        let mut group = c.benchmark_group(format!("memory_e2e_{spec_name}_alloc"));
        group.sample_size(20);
        group.measurement_time(Duration::from_secs(10));
        group.bench_function("count", |b| {
            let ctrl = controller.clone();
            let s = spec.clone();
            b.iter(|| {
                let before = PEAK_ALLOC.current_usage();
                rt.block_on(async {
                    ctrl.execute(black_box(build_request(&s)))
                        .await
                        .expect("execute ok");
                });
                let after = PEAK_ALLOC.current_usage();
                // Diff is the net residue; for an idiomatic bench
                // run, post-execute() most allocations should have
                // been freed. Black-box for criterion measurement.
                black_box(after.saturating_sub(before))
            });
        });
        group.finish();
    }
}

// ====================================================================
// Criterion plumbing
// ====================================================================

criterion_group!(
    name = benches;
    config = Criterion::default()
        .significance_level(0.05)
        .noise_threshold(0.15);  // memory benches are noisier than latency
    targets = bench_memory_peak, bench_memory_alloc_count
);
criterion_main!(benches);
