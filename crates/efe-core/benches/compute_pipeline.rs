//! **Sprint D3 — Compute pipeline bench (real chain of 6 ops).**
//!
//! Measures the canonical compute sequence executed by
//! `MvpCpuFusion::run_round_sync` against the `ComputeBackend`
//! surface — without any swarm, ELS, or arena ceremony. The chain
//! mirrors production phase-for-phase :
//!
//! 1. `matmul(Q, K_T)`            — pre-softmax attention scores
//! 2. `softmax(scores, -1)`       — row-wise attention weights
//! 3. `matmul(attention, V)`      — aggregated latents
//! 4. `cone_project(agg, anchors)`— alignment cone projection
//! 5. `shannon_entropy(attn, -1)` — entropy stats over attention
//! 6. `std_dev(projected, 0)`     — post-round latent variance
//!
//! See `docs/bench/d3_compute_pipeline_architecture.md` for the full
//! design rationale (12 decisions, 10 risks, 6 SLO targets).
//!
//! # Three groups
//!
//! * **`compute_pipeline_chain`** — full chain × 3 fixtures
//!   (small / medium / large). Headline metric ; SLO `D3-1..3` (and
//!   `_blas` variants `D3-1B..3B`).
//! * **`compute_pipeline_per_phase`** — 6 sub-benches × 3 fixtures =
//!   18 sub-benches that attribute the chain cost per op. The sum
//!   should match the chain timing within ±15 % (the additivity
//!   hypothesis the bench validates implicitly).
//! * **`compute_pipeline_throughput`** — sweep `n_agents ∈ {8…512}`
//!   at fixed `dim=256`, `n_anchors=8`. Exposes the empirical
//!   scaling exponent of the chain.
//!
//! # BLAS toggle
//!
//! Group names carry a `_naive` / `_blas` suffix detected at compile
//! time via the `accelerate` / `openblas` features. Without the
//! feature, the dispatch threshold (`M·K·N ≥ 32 768`) keeps small
//! shapes on the naïve path even when BLAS is built in ; the suffix
//! is therefore a *build* identity, not a *runtime* identity.
//!
//! # Why no `peak_alloc`, no tokio runtime
//!
//! `ComputeBackend` is **synchronous** — no async = no runtime needed.
//! Heap profiling is deliberately deferred to a `compute_pipeline_peak`
//! follow-up so the timings here stay pristine (cf. D1's split between
//! `real_strategies_e2e.rs` and `real_memory.rs`).

#![allow(missing_docs)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]

use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput,
};
use efe_core::compute::{ComputeBackend, CpuBackend, Tensor};

// =====================================================================
// BLAS / naïve cfg detection
// =====================================================================
//
// Mirrors D1's pattern. `accelerate` / `openblas` are local features
// of `efe-core` that forward to the matmul BLAS dispatch. When
// neither is enabled, every matmul stays on the naïve `ikj` path
// regardless of size — the `_naive` suffix reflects that exactly.

#[cfg(any(feature = "accelerate", feature = "openblas"))]
const BLAS_SUFFIX: &str = "_blas";
#[cfg(not(any(feature = "accelerate", feature = "openblas")))]
const BLAS_SUFFIX: &str = "_naive";

// =====================================================================
// Fixtures (matched on D1 v2 sizes)
// =====================================================================
//
// `n_agents`, `dim` reflect the v2 fixtures
// (`scripts/fixtures/realistic_*_v2.json`); `n_anchors` and `max_dist`
// reflect the same fixtures' alignment cones. The bench thus measures
// shapes that production gRPC clients actually send.

#[derive(Clone, Copy)]
struct Fixture {
    name: &'static str,
    n_agents: usize,
    dim: usize,
    n_anchors: usize,
    max_dist: f32,
}

const FIXTURES: &[Fixture] = &[
    Fixture {
        name: "small",
        n_agents: 8,
        dim: 128,
        n_anchors: 4,
        max_dist: 0.30,
    },
    Fixture {
        name: "medium",
        n_agents: 32,
        dim: 256,
        n_anchors: 8,
        max_dist: 0.25,
    },
    Fixture {
        name: "large",
        n_agents: 128,
        dim: 256,
        n_anchors: 16,
        max_dist: 0.20,
    },
];

// =====================================================================
// Test inputs
// =====================================================================
//
// `q` represents both Q and V (the prod fusion uses the latent matrix
// as both query and value — this is the Mvp invariant). `k_t` is the
// pre-transposed K, materialised once in setup so the bench inner
// loop does plain `matmul(Q, K_T)` without paying for an in-iter
// transpose.

struct ChainInputs {
    /// Q : (n_agents, dim) — used as both Q AND V (latent reuse).
    q: Tensor<f32>,
    /// K_T : (dim, n_agents) — pre-transposed K.
    k_t: Tensor<f32>,
    /// Anchors : (n_anchors, dim) — one-hot orthogonal at evenly-spaced indices.
    anchors: Tensor<f32>,
    /// Cone radius for `cone_project`.
    max_dist: f32,
}

/// Deterministic xorshift uniform `[-1, 1]` generator, mirroring
/// `compute_sprint4.rs::xorshift_vec`. Reproducibility across hosts
/// is the point — the same fixture name produces the same input
/// bytes everywhere.
fn xorshift_vec(n: usize, seed: u64) -> Vec<f32> {
    let mut s = seed.max(1);
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            ((s & 0x00FF_FFFF) as f32 / 0x00FF_FFFF as f32) * 2.0 - 1.0
        })
        .collect()
}

/// Transpose a (rows, cols) row-major Tensor → (cols, rows) row-major
/// Tensor. Pre-bench setup helper, not on the timed path.
fn transpose_2d(t: &Tensor<f32>) -> Tensor<f32> {
    let shape = t.shape();
    debug_assert_eq!(shape.len(), 2, "transpose_2d expects rank-2 tensor");
    let rows = shape[0];
    let cols = shape[1];
    let src = t.as_slice().expect("contiguous slice");
    let mut out = vec![0.0f32; rows * cols];
    for i in 0..rows {
        for j in 0..cols {
            out[j * rows + i] = src[i * cols + j];
        }
    }
    Tensor::<f32>::from_slice(&out, vec![cols, rows].into())
        .expect("transpose Tensor build")
}

/// Build `n_anchors` orthogonal one-hot vectors in dim-`dim` space at
/// evenly-spaced indices. Mirrors `gen_fixtures_v2.py::evenly_spaced_anchors`
/// so the bench input matches what real gRPC requests carry.
fn build_one_hot_anchors(dim: usize, n_anchors: usize) -> Tensor<f32> {
    debug_assert!(n_anchors <= dim, "n_anchors must be <= dim");
    let step = dim / n_anchors;
    let mut data = vec![0.0f32; n_anchors * dim];
    for i in 0..n_anchors {
        data[i * dim + i * step] = 1.0;
    }
    Tensor::<f32>::from_slice(&data, vec![n_anchors, dim].into())
        .expect("anchors Tensor build")
}

/// Materialise the chain inputs once per fixture. Called at bench
/// group setup time, **outside** the timed region.
fn build_chain_inputs(f: &Fixture) -> ChainInputs {
    // Deterministic seed strategy: fixture name → unique seed. Using
    // distinct seeds for `q` and `k_raw` so the matmul Q·Kᵀ has a
    // non-degenerate score matrix (Q ≡ K would give a symmetric
    // positive semi-definite scores matrix — fine for math, but
    // dampens softmax dynamic range).
    let seed_q = match f.name {
        "small" => 0xD3_5A_AA_BBu64,
        "medium" => 0xD3_5A_BB_CCu64,
        "large" => 0xD3_5A_CC_DDu64,
        _ => 0xD3_5A_DD_EEu64, // fallback for the throughput sweep
    };
    let q_data = xorshift_vec(f.n_agents * f.dim, seed_q);
    let q = Tensor::<f32>::from_slice(&q_data, vec![f.n_agents, f.dim].into())
        .expect("q Tensor build");

    let seed_k = seed_q.wrapping_add(0xDEAD_BEEF);
    let k_data = xorshift_vec(f.n_agents * f.dim, seed_k);
    let k_raw = Tensor::<f32>::from_slice(&k_data, vec![f.n_agents, f.dim].into())
        .expect("k Tensor build");
    let k_t = transpose_2d(&k_raw);

    let anchors = build_one_hot_anchors(f.dim, f.n_anchors);
    ChainInputs {
        q,
        k_t,
        anchors,
        max_dist: f.max_dist,
    }
}

// =====================================================================
// 1. `compute_pipeline_chain` — the full chain, 3 sizes
// =====================================================================
//
// Setup : `CpuBackend` (cheap, stateless), `ChainInputs` materialised
// once per fixture (held by reference inside the closure).
// Iter  : execute the 6-op chain, black_box-ing every intermediate
// tensor so LLVM cannot dead-code-eliminate the chain or fuse
// adjacent ops illegally.
//
// Sample sizes adapted per fixture so wall time per group stays
// bounded (criterion auto-tunes iters/sample):
//
// * small (~10-20 µs/iter)  → 50 samples
// * medium (~150-200 µs/iter) → 50 samples
// * large (~5-15 ms/iter)   → 30 samples (large is BLAS-bound;
//                               on naïve path, we're closer to
//                               20 ms/iter, still tractable)
//
// SLO `D3-1`, `D3-2`, `D3-3` (and `_blas` variants).

fn bench_compute_pipeline_chain(c: &mut Criterion) {
    let backend = CpuBackend::new();
    let group_name = format!("compute_pipeline_chain{BLAS_SUFFIX}");
    let mut group = c.benchmark_group(&group_name);
    group.throughput(Throughput::Elements(1));

    for fixture in FIXTURES {
        let inputs = build_chain_inputs(fixture);
        let samples = match fixture.name {
            "small" | "medium" => 50,
            _ => 30,
        };
        group.sample_size(samples);

        group.bench_function(fixture.name, |b| {
            b.iter(|| {
                // PHASE 1 — scores = Q @ K_T  (matmul)
                let scores = backend
                    .matmul(black_box(&inputs.q), black_box(&inputs.k_t))
                    .expect("phase 1 matmul");
                // PHASE 2 — attention = softmax(scores, axis=-1)
                let attention = backend
                    .softmax(black_box(&scores), -1)
                    .expect("phase 2 softmax");
                // PHASE 3 — aggregated = attention @ Q   (Q ≡ V)
                let aggregated = backend
                    .matmul(black_box(&attention), black_box(&inputs.q))
                    .expect("phase 3 matmul");
                // PHASE 4 — projected = cone_project(aggregated, anchors)
                let projected = backend
                    .cone_project(
                        black_box(&aggregated),
                        black_box(&inputs.anchors),
                        inputs.max_dist,
                    )
                    .expect("phase 4 cone_project");
                // PHASE 5 — entropy = shannon_entropy(attention, axis=-1)
                let entropy = backend
                    .shannon_entropy(black_box(&attention), -1)
                    .expect("phase 5 shannon_entropy");
                // PHASE 6 — stats = std_dev(projected, axis=0, biased)
                //
                // `unbiased=false` matches the production `compute_stats`
                // (population variance over agents per dim). The bench
                // measures the same arithmetic.
                let stats = backend
                    .std_dev(black_box(&projected), 0, false)
                    .expect("phase 6 std_dev");

                black_box((scores, attention, aggregated, projected, entropy, stats));
            });
        });
    }
    group.finish();
}

// =====================================================================
// 2. `compute_pipeline_per_phase` — per-op attribution
// =====================================================================
//
// Each fixture pre-computes the intermediate tensors **once** in the
// setup region (outside the timed loop). The 6 sub-benches per
// fixture then re-execute their target op with the same inputs each
// iteration.
//
// **Important** : the input to each phase is the *real downstream*
// input from the chain, not synthetic random data. Phase 4's
// `cone_project` runs against the *aggregated* tensor (post-softmax,
// post-matmul), not a fresh xorshift Tensor. This is what makes the
// per-phase measurement representative of the chain and not a
// micro-benchmark in disguise.
//
// We allocate the intermediates once and reuse them across the 6
// sub-benches per fixture — `Tensor<f32>` cloning would defeat the
// purpose. The borrow lives across the bench_function calls because
// the `bencher` is sync and the closures are invoked in sequence.

fn bench_compute_pipeline_per_phase(c: &mut Criterion) {
    let backend = CpuBackend::new();
    let group_name = format!("compute_pipeline_per_phase{BLAS_SUFFIX}");
    let mut group = c.benchmark_group(&group_name);

    for fixture in FIXTURES {
        let inputs = build_chain_inputs(fixture);

        // Pre-compute the intermediate outputs (setup, off-timer).
        // Each downstream phase consumes one of these as its real
        // input — keeps the bench representative of the chain's
        // data flow and not a synthetic micro.
        let scores = backend
            .matmul(&inputs.q, &inputs.k_t)
            .expect("setup: matmul scores");
        let attention = backend
            .softmax(&scores, -1)
            .expect("setup: softmax");
        let aggregated = backend
            .matmul(&attention, &inputs.q)
            .expect("setup: matmul aggregate");
        let projected = backend
            .cone_project(&aggregated, &inputs.anchors, inputs.max_dist)
            .expect("setup: cone_project");

        // Sample sizes tuned so each sub-bench takes ~250-500 ms
        // wall time, regardless of the per-op nominal cost.
        let samples = match fixture.name {
            "small" | "medium" => 100,
            _ => 50,
        };
        group.sample_size(samples);

        // Phase 1 — scores = Q @ K_T
        group.bench_function(format!("{}/phase1_matmul_scores", fixture.name), |b| {
            b.iter(|| {
                let r = backend
                    .matmul(black_box(&inputs.q), black_box(&inputs.k_t))
                    .expect("matmul scores");
                black_box(r);
            });
        });

        // Phase 2 — attention = softmax(scores)
        group.bench_function(format!("{}/phase2_softmax", fixture.name), |b| {
            b.iter(|| {
                let r = backend
                    .softmax(black_box(&scores), -1)
                    .expect("softmax");
                black_box(r);
            });
        });

        // Phase 3 — aggregated = attention @ Q
        group.bench_function(format!("{}/phase3_matmul_aggregate", fixture.name), |b| {
            b.iter(|| {
                let r = backend
                    .matmul(black_box(&attention), black_box(&inputs.q))
                    .expect("matmul aggregate");
                black_box(r);
            });
        });

        // Phase 4 — projected = cone_project(aggregated)
        group.bench_function(format!("{}/phase4_cone_project", fixture.name), |b| {
            b.iter(|| {
                let r = backend
                    .cone_project(
                        black_box(&aggregated),
                        black_box(&inputs.anchors),
                        inputs.max_dist,
                    )
                    .expect("cone_project");
                black_box(r);
            });
        });

        // Phase 5 — entropy = shannon_entropy(attention)
        group.bench_function(format!("{}/phase5_shannon_entropy", fixture.name), |b| {
            b.iter(|| {
                let r = backend
                    .shannon_entropy(black_box(&attention), -1)
                    .expect("shannon_entropy");
                black_box(r);
            });
        });

        // Phase 6 — stats = std_dev(projected)
        group.bench_function(format!("{}/phase6_std_dev", fixture.name), |b| {
            b.iter(|| {
                let r = backend
                    .std_dev(black_box(&projected), 0, false)
                    .expect("std_dev");
                black_box(r);
            });
        });
    }
    group.finish();
}

// =====================================================================
// 3. `compute_pipeline_throughput` — n_agents sweep at fixed dim
// =====================================================================
//
// Sweeps `n_agents ∈ {8, 16, 32, 64, 128, 256}` at `dim = 256`,
// `n_anchors = 8`. Returns the chain timing per `n` value, with
// `Throughput::Elements(n²)` so criterion reports `ops/s` proportional
// to the scores matrix size.
//
// Use case : extract the empirical scaling exponent of the chain
// (expected `O(n²·d)` from the matmul phases). On a log-log plot
// (chain time vs n_agents), the slope between consecutive points
// should be close to 2 once `n` is large enough that the matmuls
// dominate.
//
// We cap at `n=256` rather than `n=512` because the architecture's
// goal is "expose the curve", not "max-out the host". 256 keeps the
// bench under 30 s wall time.

const SWEEP_DIM: usize = 256;
const SWEEP_N_ANCHORS: usize = 8;
const SWEEP_MAX_DIST: f32 = 0.25;

fn bench_compute_pipeline_throughput(c: &mut Criterion) {
    let backend = CpuBackend::new();
    let group_name = format!("compute_pipeline_throughput{BLAS_SUFFIX}");
    let mut group = c.benchmark_group(&group_name);
    // 30 samples is the right trade-off for a sweep — we want enough
    // points for a clean curve without burning wall time on a
    // group that is secondary to chain (the headline).
    group.sample_size(30);

    for &n in &[8usize, 16, 32, 64, 128, 256] {
        let fixture = Fixture {
            name: "sweep",
            n_agents: n,
            dim: SWEEP_DIM,
            n_anchors: SWEEP_N_ANCHORS,
            max_dist: SWEEP_MAX_DIST,
        };
        let inputs = build_chain_inputs(&fixture);

        // Throughput = n² (the dominant matmul shape size). At fixed
        // `dim`, this gives a clean ops/s metric that grows
        // quadratically — handy for the log-log plot.
        group.throughput(Throughput::Elements((n * n) as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(n),
            &n,
            |b, _| {
                b.iter(|| {
                    let scores = backend
                        .matmul(black_box(&inputs.q), black_box(&inputs.k_t))
                        .expect("matmul scores");
                    let attention = backend
                        .softmax(black_box(&scores), -1)
                        .expect("softmax");
                    let aggregated = backend
                        .matmul(black_box(&attention), black_box(&inputs.q))
                        .expect("matmul aggregate");
                    let projected = backend
                        .cone_project(
                            black_box(&aggregated),
                            black_box(&inputs.anchors),
                            inputs.max_dist,
                        )
                        .expect("cone_project");
                    let entropy = backend
                        .shannon_entropy(black_box(&attention), -1)
                        .expect("shannon_entropy");
                    let stats = backend
                        .std_dev(black_box(&projected), 0, false)
                        .expect("std_dev");
                    black_box((scores, attention, aggregated, projected, entropy, stats));
                });
            },
        );
    }
    group.finish();
}

// =====================================================================
// Criterion entry points
// =====================================================================
//
// Group order :
// 1. `chain` first — fastest to fail if the bench is broken.
// 2. `per_phase` — the longest single group; locating it second
//    keeps the wallclock visible early.
// 3. `throughput` — a sweep, mostly informational.

criterion_group!(
    name = compute_pipeline_benches;
    config = Criterion::default()
        .significance_level(0.05);
    targets =
        bench_compute_pipeline_chain,
        bench_compute_pipeline_per_phase,
        bench_compute_pipeline_throughput,
);
criterion_main!(compute_pipeline_benches);
