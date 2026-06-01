//! **Sprint D1 — Production-strategies end-to-end bench.**
//!
//! Measures the **real** Mode-1 critical path that production gRPC
//! clients pay for, by reusing `efe_server::mode1::build_mvp_backend`.
//! No mocks: every component (`GenesisEngine`, `MvpCpuFusion`,
//! `MultiCriteriaDetector`, `WeightedMeanExtractor`,
//! `StrictAlignmentProjector`, …) is the same `Arc<dyn Trait>` that
//! `main.rs` wires.
//!
//! # Bench groups
//!
//! 1. `real_e2e_latency` — full request lifecycle on the 3 v2
//!    fixtures (small / medium / large). Headline numbers; these are
//!    what land in `BENCHMARKS.md` and gate releases via SLO targets.
//! 2. `real_e2e_throughput` — N concurrent requests on the small
//!    fixture (saturation behaviour, queue effects).
//! 3. `real_phase_genesis` — `GenesisStrategy::spawn` in isolation
//!    (faithful: GenesisEngine has no preceding state).
//! 4. `real_phase_alignment` — `AlignmentProjector::project` in
//!    isolation (faithful: pure CPU function, no setup).
//!
//! Per-phase fusion / dissolution isolation are **not measured here**:
//! both require a primed ELS+arena which is itself the output of
//! genesis. Re-spawning per iteration would dominate the measurement.
//! The per-round fusion cost is approximated downstream via the
//! `parse_criterion_report.py` helper (delta between rounds-1 and
//! rounds-N runs); the controller does not expose per-phase timers
//! publicly, by design.
//!
//! # BLAS vs naïve
//!
//! Group names are suffixed with the active linear-algebra backend:
//!
//! * `_naive` — default (no BLAS feature flag set on `efe-core`)
//! * `_blas`  — `--features efe-core/accelerate` or
//!              `--features efe-core/openblas` set on `efe-core`
//!
//! Enabling either flag at the workspace root produces a comparable
//! `_blas` report; the user diff-merges the two HTML reports
//! manually. We deliberately do not call both kernels at runtime
//! within one bench — the kernel selection is monomorphised at
//! compile time, so reusing the same scaffolding twice is the only
//! sensible way.
//!
//! # Concurrency model
//!
//! Every group uses `bencher.to_async(&rt)` against a single
//! multi-threaded tokio runtime built once per bench invocation.
//! Building the runtime inside `iter` would re-pay thread-pool
//! creation for every sample (≈ 5 ms overhead) and contaminate the
//! latency distribution.

#![allow(missing_docs)]

mod common;

use std::sync::Arc;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use tokio::runtime::Runtime;

use efe_core::dissolution::{
    alignment::{AlignmentProjector, StrictAlignmentProjector},
    config::ExtractionConfig,
};
use efe_core::genesis::{
    engine::GenesisEngine,
    registry::{StubTemplateRegistry, TemplateRegistry},
};
use efe_core::memory::{HostBackend, MemoryPool, PoolConfig};
use efe_core::streams::{StreamPool, StreamPoolConfig};
use efe_core::swarm::strategies::GenesisStrategy;

use common::{build_real_backend, build_real_request, load_realistic_v2_spec};

// =====================================================================
// BLAS / naïve cfg detection
// =====================================================================
//
// Suffix used in benchmark group names so `cargo bench -p efe-server
// --features accelerate` (or `openblas`) produces a distinct report
// from the default (naïve) build. The local `accelerate` /
// `openblas` features are defined in `efe-server/Cargo.toml` and
// forward to the matching `efe-core` features.
//
// Why we forward instead of checking `efe-core/<flag>` directly:
// `cfg(feature = "...")` only sees the features of the *current*
// crate (here, the `efe-server` bench binary). To branch on
// `efe-core` features we must declare a local mirror feature.

#[cfg(any(feature = "accelerate", feature = "openblas"))]
const BLAS_SUFFIX: &str = "_blas";
#[cfg(not(any(feature = "accelerate", feature = "openblas")))]
const BLAS_SUFFIX: &str = "_naive";

// =====================================================================
// Shared tokio runtime
// =====================================================================

/// Build a multi-threaded tokio runtime for the bench. Sized at 4
/// worker threads — matches the production server's typical RT and
/// keeps NUMA contention in check on bench hosts. Single allocation
/// per bench invocation; `Arc`-cloned into closures.
fn make_runtime() -> Arc<Runtime> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("tokio runtime");
    Arc::new(rt)
}

// =====================================================================
// 1. End-to-end latency
// =====================================================================
//
// Full request lifecycle: genesis + fusion (×N rounds until consensus)
// + dissolution + alignment + decode. Run sequentially (one outstanding
// request at a time) so we measure pure latency, not queue waits.
//
// Sample sizes are tuned per fixture:
//
// * small (8 agents, 128-dim, 8 rounds max) ≈ 1-3 ms ⇒ 50 samples
// * medium (32 agents, 256-dim, 8 rounds max) ≈ 5-15 ms ⇒ 30 samples
// * large (128 agents, 256-dim, 8 rounds max) ≈ 30-80 ms ⇒ 20 samples
//
// Lower sample counts on the slower fixtures keep wall time bounded
// (criterion runs ≈ sample_size + warmup iterations); CI budget is
// about 5 minutes for the whole bench.

fn bench_real_e2e_latency(c: &mut Criterion) {
    let rt = make_runtime();
    let group_name = format!("real_e2e_latency{BLAS_SUFFIX}");
    let mut group = c.benchmark_group(&group_name);
    group.throughput(Throughput::Elements(1));

    for fixture in ["small", "medium", "large"] {
        let spec = load_realistic_v2_spec(fixture);
        let backend = build_real_backend(&spec);

        let sample_size = match fixture {
            "small" => 50,
            "medium" => 30,
            _ => 20,
        };
        group.sample_size(sample_size);

        group.bench_with_input(BenchmarkId::from_parameter(fixture), fixture, |b, _| {
            let backend = backend.clone();
            let spec = spec.clone();
            b.to_async(&*rt).iter(|| {
                let backend = backend.clone();
                let req = build_real_request(&spec);
                async move {
                    let resp = backend.execute(req).await.expect("real backend execute");
                    // Force criterion to keep the response live so
                    // LLVM cannot eliminate the call.
                    black_box(resp);
                }
            });
        });
    }
    group.finish();
}

// =====================================================================
// 2. End-to-end throughput
// =====================================================================
//
// Drives 8 concurrent requests through the same backend. Saturates the
// memory pool's `max_arenas = 8` cap so we observe the steady-state
// throughput without artificial queueing.
//
// Why only the small fixture: throughput at medium / large would take
// minutes per sample with `iter_custom` semantics (8 × ~50 ms each ≈
// 400 ms/iter). Small (~2 ms/req) is fast enough to converge to a
// stable mean within criterion's default budget.

const THROUGHPUT_CONCURRENCY: usize = 8;

fn bench_real_e2e_throughput(c: &mut Criterion) {
    let rt = make_runtime();
    let group_name = format!("real_e2e_throughput{BLAS_SUFFIX}");
    let mut group = c.benchmark_group(&group_name);
    group.throughput(Throughput::Elements(THROUGHPUT_CONCURRENCY as u64));
    group.sample_size(20); // each iter dispatches 8 swarms ⇒ slower than e2e_latency.

    let spec = load_realistic_v2_spec("small");
    let backend = build_real_backend(&spec);

    group.bench_function(
        BenchmarkId::from_parameter(format!("c{THROUGHPUT_CONCURRENCY}")),
        |b| {
            let backend = backend.clone();
            let spec = spec.clone();
            b.to_async(&*rt).iter(|| {
                let backend = backend.clone();
                let spec = spec.clone();
                async move {
                    // Spawn N concurrent requests, await all.
                    let mut handles = Vec::with_capacity(THROUGHPUT_CONCURRENCY);
                    for _ in 0..THROUGHPUT_CONCURRENCY {
                        let backend = backend.clone();
                        let req = build_real_request(&spec);
                        handles.push(tokio::spawn(async move {
                            backend.execute(req).await.expect("execute")
                        }));
                    }
                    for h in handles {
                        let resp = h.await.expect("join");
                        black_box(resp);
                    }
                }
            });
        },
    );
    group.finish();
}

// =====================================================================
// 3. Per-phase isolation — Genesis
// =====================================================================
//
// Builds a fresh `GenesisEngine` from the same building blocks as
// `build_mvp_backend` (HostBackend + MemoryPool + StreamPool +
// StubTemplateRegistry::mvp_defaults), then times only the
// `spawn(&spec)` call. This is faithful: genesis has no upstream
// state — every request goes through this exact path.
//
// Notes:
//
// * Each iteration acquires a fresh arena from the pool. The pool is
//   pre-allocated (`prealloc_arenas = 4` here, larger than for the
//   e2e bench because we hammer it with fast iterations) so the
//   arena hand-out is amortised.
// * The engine is built ONCE in setup; the cost we report is the
//   per-call genesis overhead, not the engine assembly cost.

fn build_genesis_engine_for_dim(dim_latent: u32) -> GenesisEngine {
    let device_backend: Arc<dyn efe_core::memory::DeviceBackend> = Arc::new(HostBackend::new());

    let mut pool_cfg = PoolConfig::default();
    pool_cfg.prealloc_arenas = 4;
    pool_cfg.max_arenas = 8;
    let memory_pool =
        Arc::new(MemoryPool::new(pool_cfg, device_backend.clone()).expect("memory pool"));

    let mut stream_cfg = StreamPoolConfig::default();
    stream_cfg.size = 4;
    let stream_pool = Arc::new(StreamPool::new(stream_cfg).expect("stream pool"));

    let template_registry: Arc<dyn TemplateRegistry> =
        Arc::new(StubTemplateRegistry::mvp_defaults(dim_latent));

    GenesisEngine::builder()
        .build_mvp(memory_pool, template_registry, stream_pool)
        .expect("genesis engine")
}

fn bench_real_phase_genesis(c: &mut Criterion) {
    let rt = make_runtime();
    let group_name = format!("real_phase_genesis{BLAS_SUFFIX}");
    let mut group = c.benchmark_group(&group_name);
    group.throughput(Throughput::Elements(1));

    for fixture in ["small", "medium", "large"] {
        let spec = load_realistic_v2_spec(fixture);
        let engine = Arc::new(build_genesis_engine_for_dim(spec.dim_latent.get()));

        // Genesis is fast (sub-millisecond on small) and arena-bound;
        // we can afford 50 samples on every size.
        group.sample_size(50);

        group.bench_with_input(BenchmarkId::from_parameter(fixture), fixture, |b, _| {
            let engine = engine.clone();
            let spec = spec.clone();
            b.to_async(&*rt).iter(|| {
                let engine = engine.clone();
                let spec = spec.clone();
                async move {
                    let (batch, els) = engine.spawn(&spec).await.expect("genesis spawn");
                    // Drop the spawned ELS+batch immediately. The
                    // arena is returned to the pool when `batch`
                    // drops, keeping the pool from exhausting after
                    // ~max_arenas iterations.
                    black_box((batch, els));
                }
            });
        });
    }
    group.finish();
}

// =====================================================================
// 4. Per-phase isolation — Alignment
// =====================================================================
//
// `StrictAlignmentProjector::project` is a pure-CPU function with no
// async or arena state. We feed it a synthetic centroid (zeros) and
// the spec's anchors. Cost ∝ `n_anchors × dim_latent` — small but not
// negligible at large fixtures (16 anchors × 256 dim).
//
// We keep `_blas` / `_naive` suffixing for consistency, even though
// alignment uses no BLAS path today — projector arithmetic is a
// dot-product per anchor and a softmax. Future optimization may
// dispatch to BLAS at large `n_anchors`; the suffix lets us track
// that transition cleanly.

fn bench_real_phase_alignment(c: &mut Criterion) {
    let group_name = format!("real_phase_alignment{BLAS_SUFFIX}");
    let mut group = c.benchmark_group(&group_name);
    group.throughput(Throughput::Elements(1));
    group.sample_size(100); // alignment is microsecond-scale ⇒ many samples.

    let projector = StrictAlignmentProjector;
    let extraction_cfg = {
        let cfg = ExtractionConfig::default();
        cfg.validate().expect("extraction default valid");
        cfg
    };

    for fixture in ["small", "medium", "large"] {
        let spec = load_realistic_v2_spec(fixture);
        // Use the *first anchor* as the synthetic centroid. Anchors
        // are unit one-hot vectors at orthogonal indices, so the
        // strict projector's distance metric is well-defined and
        // *guaranteed inside the cone* (delta ≈ 0). The bench
        // measures the projection arithmetic cost — softmax over
        // anchor cosines + weighted sum + breach check — not the
        // failure path. A `vec![0.05; dim]` centroid would breach
        // the strict cone (the v2 fixtures use radii 0.20-0.30 to
        // stress the projector in production); the bench would
        // panic on the small fixture before recording any sample.
        let centroid: Vec<f32> = spec
            .alignment_cone
            .anchors
            .first()
            .expect("v2 fixture has at least one anchor")
            .clone();
        let anchors = spec.alignment_cone.anchors.clone();
        let radius = spec.alignment_cone.max_projection_distance;

        group.bench_with_input(BenchmarkId::from_parameter(fixture), fixture, |b, _| {
            b.iter(|| {
                let outcome = projector
                    .project(&centroid, &anchors, radius, &extraction_cfg)
                    .expect("alignment project");
                black_box(outcome);
            });
        });
    }
    group.finish();
}

// =====================================================================
// Criterion entry points
// =====================================================================

criterion_group!(
    name = real_strategies_e2e_benches;
    config = Criterion::default();
    targets =
        bench_real_e2e_latency,
        bench_real_e2e_throughput,
        bench_real_phase_genesis,
        bench_real_phase_alignment,
);
criterion_main!(real_strategies_e2e_benches);
