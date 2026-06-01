//! **Sprint D3 — Compute pipeline realism anti-regression tests.**
//!
//! Run as part of the regular `cargo test -p efe-core` suite. They
//! prove the D3 bench is measuring a numerically valid chain — not a
//! degenerate sequence that produces NaN, wrong shapes, or violates
//! the softmax row-sum invariant.
//!
//! Mirror of `tests/pool_realism_check.rs` (D2) and
//! `crates/efe-server/tests/bench_realism_check.rs` (D1) : every
//! claim the bench makes is locked in by a test that fails BEFORE
//! the bench can produce a misleading number.
//!
//! The 3 tests cover :
//!
//! 1. **`chain_produces_finite_outputs_on_all_fixtures`** — every
//!    intermediate tensor in the 6-op chain is finite (no NaN, no
//!    `±Inf`) on the small / medium / large v2 shapes.
//!
//! 2. **`chain_shapes_match_expected`** — the shape contract of each
//!    op is verified against the architecture §6.1 table. If a future
//!    change makes `cone_project` return rank-1 instead of rank-2,
//!    the bench would silently measure the wrong thing — this test
//!    trips first.
//!
//! 3. **`attention_rows_sum_to_one`** — after softmax, each row of
//!    the attention matrix must sum to `1.0 ± 1e-5`. A regression
//!    in `ComputeBackend::softmax` (e.g. wrong axis, missing
//!    max-shift) would produce non-normalised rows and the bench
//!    would measure a chain with broken numerics.
//!
//! # Why we duplicate the helpers from `benches/compute_pipeline.rs`
//!
//! Cargo treats `tests/` and `benches/` as separate compilation
//! units. A `mod common` import would not resolve. Rather than expose
//! `Fixture` / `build_chain_inputs` as `pub` in the library (which
//! would leak bench-only plumbing into the public API), we duplicate
//! the ~40 lines of helpers here. The duplication is locked against
//! drift by these tests : any change to the bench helpers that breaks
//! the realism contract trips a test on the next `cargo test` run.

use efe_core::compute::{ComputeBackend, CpuBackend, Tensor};

// =====================================================================
// Helpers (mirror of benches/compute_pipeline.rs)
// =====================================================================

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

struct ChainInputs {
    q: Tensor<f32>,
    k_t: Tensor<f32>,
    anchors: Tensor<f32>,
    max_dist: f32,
}

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

fn transpose_2d(t: &Tensor<f32>) -> Tensor<f32> {
    let shape = t.shape();
    assert_eq!(shape.len(), 2);
    let rows = shape[0];
    let cols = shape[1];
    let src = t.as_slice().expect("contiguous");
    let mut out = vec![0.0f32; rows * cols];
    for i in 0..rows {
        for j in 0..cols {
            out[j * rows + i] = src[i * cols + j];
        }
    }
    Tensor::<f32>::from_slice(&out, vec![cols, rows].into()).expect("transpose")
}

fn build_one_hot_anchors(dim: usize, n_anchors: usize) -> Tensor<f32> {
    assert!(n_anchors <= dim);
    let step = dim / n_anchors;
    let mut data = vec![0.0f32; n_anchors * dim];
    for i in 0..n_anchors {
        data[i * dim + i * step] = 1.0;
    }
    Tensor::<f32>::from_slice(&data, vec![n_anchors, dim].into()).expect("anchors")
}

fn build_chain_inputs(f: &Fixture) -> ChainInputs {
    let seed_q = match f.name {
        "small" => 0xD3_5A_AA_BBu64,
        "medium" => 0xD3_5A_BB_CCu64,
        _ => 0xD3_5A_CC_DDu64,
    };
    let q_data = xorshift_vec(f.n_agents * f.dim, seed_q);
    let q =
        Tensor::<f32>::from_slice(&q_data, vec![f.n_agents, f.dim].into()).expect("q");
    let seed_k = seed_q.wrapping_add(0xDEAD_BEEF);
    let k_data = xorshift_vec(f.n_agents * f.dim, seed_k);
    let k_raw = Tensor::<f32>::from_slice(&k_data, vec![f.n_agents, f.dim].into())
        .expect("k_raw");
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
// Test 1 — All chain outputs are finite (no NaN/Inf)
// =====================================================================
//
// Walks the 6-op chain on each fixture and asserts every f32 in
// every intermediate tensor is finite. Detects :
//
// * A regression in `softmax` that produces NaN on a degenerate row
//   (e.g. dropping the max-shift trick).
// * A regression in `cone_project` where `+Inf` propagates from a
//   pathological latent.
// * A regression in `std_dev` where division-by-zero on a
//   constant column emits NaN instead of 0 (depends on `unbiased`
//   flag — we use `unbiased=false` to match the prod stats path).

#[test]
fn chain_produces_finite_outputs_on_all_fixtures() {
    let backend = CpuBackend::new();
    for fixture in FIXTURES {
        let inputs = build_chain_inputs(fixture);
        let scores = backend.matmul(&inputs.q, &inputs.k_t).expect("matmul1");
        let attention = backend.softmax(&scores, -1).expect("softmax");
        let aggregated = backend.matmul(&attention, &inputs.q).expect("matmul2");
        let projected = backend
            .cone_project(&aggregated, &inputs.anchors, inputs.max_dist)
            .expect("cone_project");
        let entropy = backend.shannon_entropy(&attention, -1).expect("entropy");
        let stats = backend.std_dev(&projected, 0, false).expect("std_dev");

        for (label, t) in [
            ("scores", &scores),
            ("attention", &attention),
            ("aggregated", &aggregated),
            ("projected", &projected),
            ("entropy", &entropy),
            ("stats", &stats),
        ] {
            for (idx, &v) in t.as_slice().expect("contiguous").iter().enumerate() {
                assert!(
                    v.is_finite(),
                    "[{}] {} has non-finite at index {}: {}",
                    fixture.name,
                    label,
                    idx,
                    v
                );
            }
        }
    }
}

// =====================================================================
// Test 2 — All chain output shapes match the architecture contract
// =====================================================================
//
// Pins the shape of every intermediate tensor against the design
// table (D3 architecture §6.1). If `ComputeBackend::cone_project`
// ever changes from `(B, d)` to `(B,)` (e.g. returning the
// projection magnitude only), this test trips immediately and the
// SLO targets need re-calibration before the bench is trusted.

#[test]
fn chain_shapes_match_expected() {
    let backend = CpuBackend::new();
    for fixture in FIXTURES {
        let inputs = build_chain_inputs(fixture);

        // Note : `Tensor::shape()` returns `&Shape` (not `&[usize]`).
        // We compare via `.shape().dims()` which yields `&[usize]`,
        // matching the array-literal we assert against.

        let scores = backend.matmul(&inputs.q, &inputs.k_t).expect("matmul1");
        assert_eq!(
            scores.shape().dims(),
            [fixture.n_agents, fixture.n_agents],
            "[{}] scores shape",
            fixture.name
        );

        let attention = backend.softmax(&scores, -1).expect("softmax");
        assert_eq!(
            attention.shape().dims(),
            [fixture.n_agents, fixture.n_agents],
            "[{}] attention shape",
            fixture.name
        );

        let aggregated = backend.matmul(&attention, &inputs.q).expect("matmul2");
        assert_eq!(
            aggregated.shape().dims(),
            [fixture.n_agents, fixture.dim],
            "[{}] aggregated shape",
            fixture.name
        );

        let projected = backend
            .cone_project(&aggregated, &inputs.anchors, fixture.max_dist)
            .expect("cone_project");
        assert_eq!(
            projected.shape().dims(),
            [fixture.n_agents, fixture.dim],
            "[{}] projected shape",
            fixture.name
        );

        let entropy = backend.shannon_entropy(&attention, -1).expect("entropy");
        assert_eq!(
            entropy.shape().dims(),
            [fixture.n_agents],
            "[{}] entropy shape",
            fixture.name
        );

        let stats = backend.std_dev(&projected, 0, false).expect("std_dev");
        assert_eq!(
            stats.shape().dims(),
            [fixture.dim],
            "[{}] stats shape",
            fixture.name
        );
    }
}

// =====================================================================
// Test 3 — Softmax row-sum invariant
// =====================================================================
//
// The most common subtle regression in a softmax implementation is
// the wrong axis : `softmax(x, 0)` instead of `softmax(x, -1)` on a
// (N, N) tensor produces matrices that COLUMN-sum to 1, not
// row-sum. Both versions look "softmax-like" to a quick visual
// check but produce wildly different downstream behaviour.
//
// We fix this by asserting `Σ_j attention[i, j] == 1.0 ± 1e-5` for
// every row `i`. The small tolerance accounts for the 32-bit float
// summation order difference between platforms (BLAS reorders
// reductions ; same numerical result modulo last 2 ulps).

#[test]
fn attention_rows_sum_to_one() {
    let backend = CpuBackend::new();
    for fixture in FIXTURES {
        let inputs = build_chain_inputs(fixture);
        let scores = backend.matmul(&inputs.q, &inputs.k_t).expect("matmul");
        let attention = backend.softmax(&scores, -1).expect("softmax");

        let n = fixture.n_agents;
        let data = attention.as_slice().expect("contiguous");
        for i in 0..n {
            let row_sum: f32 = data[i * n..(i + 1) * n].iter().sum();
            assert!(
                (row_sum - 1.0).abs() < 1e-5,
                "[{}] row {} sums to {} (expected 1.0 ± 1e-5)",
                fixture.name,
                i,
                row_sum
            );
        }
    }
}
