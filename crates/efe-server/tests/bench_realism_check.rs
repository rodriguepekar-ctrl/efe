//! **Sprint D1 — Bench realism anti-regression tests.**
//!
//! These tests run as part of the regular `cargo test -p efe-server`
//! suite. Their job is to **prove the bench is measuring the real
//! critical path**, not a degenerate fast path.
//!
//! Rationale: the Sprint C bench used mock strategies, and a
//! reviewer audit (cf. `docs/bench/d1_real_strategies_architecture.md`
//! §3) found the harness was exercising < 5 % of production code.
//! The Sprint D1 bench reuses `build_mvp_backend` (production
//! wiring), but we still need *test-time* guarantees that:
//!
//! 1. **Multi-round fusion really runs** — the consensus detector
//!    does not short-circuit on round 1 (which would silently make
//!    the bench measure single-round latency, drowning the actual
//!    fusion cost).
//! 2. **Output shape is realistic** — the dissolver returns a raw
//!    centroid of the right dimensionality (not an empty vec, not a
//!    text stub).
//! 3. **No memory leak across iterations** — N requests on the same
//!    backend do not grow the heap residue unboundedly.
//!
//! If any of these assumptions break, the bench numbers in
//! `BENCHMARKS.md` would be misleading at best, fraudulent at worst.
//! We assert them here so any regression trips the test suite
//! BEFORE the bench is ever run.
//!
//! # Why we re-implement the v2 fixture loader inline
//!
//! The fixture loader in
//! `crates/efe-server/benches/common/mod.rs` lives in a *bench*
//! compilation unit. Cargo treats `benches/`, `tests/`, and `src/`
//! as three separate compilation units, so a `mod` import would
//! not resolve from a test file. Rather than expose the loader as
//! `pub mod bench` in the library (which would leak bench plumbing
//! into the public API), we duplicate the ~30 lines of parser
//! here. The fixture schema is documented in `scripts/gen_fixtures_v2.py`
//! and the duplication is a minor maintenance cost vs the API
//! pollution alternative.

use std::sync::Arc;

use efe_core::swarm::request::{DecodedOutput, Request};
use efe_core::types::{
    AlignmentCone, LatentDim, PopulationMix, ResourceBudget, SwarmSpec, TemplateCode,
};
use efe_server::mode1::{build_mvp_backend, Mode1Config};
use efe_server::services::ExecuteBackend;

// =====================================================================
// Inline v2 fixture loader (mirror of benches/common/mod.rs)
// =====================================================================

#[derive(serde::Deserialize)]
struct FixtureWrapper {
    spec: FixtureSpec,
}

#[derive(serde::Deserialize)]
struct FixtureSpec {
    dim_latent: u32,
    problem_embedding: Vec<f32>,
    population: Vec<(String, u32)>,
    max_rounds: u32,
    alignment_cone: FixtureCone,
}

#[derive(serde::Deserialize)]
struct FixtureCone {
    anchors: Vec<Vec<f32>>,
    max_projection_distance: f32,
}

fn parse_template(s: &str) -> TemplateCode {
    match s {
        "NaSem" => TemplateCode::NaSem,
        "NaLog" => TemplateCode::NaLog,
        "NaCrit" => TemplateCode::NaCrit,
        other => panic!("unknown template_code in fixture: `{other}`"),
    }
}

fn load_v2_spec(name: &str) -> SwarmSpec {
    let crate_root = env!("CARGO_MANIFEST_DIR");
    let path = format!("{crate_root}/../../scripts/fixtures/realistic_{name}_v2.json");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let wrapper: FixtureWrapper =
        serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {path}: {e}"));
    let f = wrapper.spec;
    let dim = LatentDim::new(f.dim_latent).expect("dim");
    let pop = PopulationMix::new(
        f.population
            .into_iter()
            .map(|(s, n)| (parse_template(&s), n))
            .collect(),
    )
    .expect("pop");
    let cone = AlignmentCone::new(
        f.alignment_cone.anchors,
        f.alignment_cone.max_projection_distance,
    )
    .expect("cone");

    // Generous 90 s wall-clock budget — the default 500 ms is built
    // for production where most requests converge in < 100 ms ; in
    // debug `cargo test` mode a single large-fixture request can
    // legitimately take 1-3 s, and the four `realism_*` tests run
    // concurrently under `multi_thread`, compounding contention on
    // a 2-core CI runner. Mirrors the bench loader in
    // `crates/efe-server/benches/common/mod.rs`.
    //
    // **Φ-D-OPS audit** raised this from 30 000 → 90 000 ms after
    // observing CPU starvation on macOS arm64 debug builds running
    // the full workspace test suite.
    let mut budget = ResourceBudget::default();
    budget.max_wall_clock_ms = 90_000;

    SwarmSpec::builder()
        .problem_embedding(f.problem_embedding)
        .dim_latent(dim)
        .population(pop)
        .max_rounds(f.max_rounds)
        .expect("rounds")
        .alignment_cone(cone)
        .budget(budget)
        .build()
        .expect("spec")
}

fn build_backend_for(spec: &SwarmSpec) -> Arc<dyn ExecuteBackend> {
    let mut cfg = Mode1Config::for_dim(spec.dim_latent.get());
    cfg.prealloc_arenas = 2;
    cfg.max_arenas = 8;
    build_mvp_backend(cfg).expect("backend builds")
}

// =====================================================================
// 1. Multi-round fusion really runs
// =====================================================================
//
// The consensus detector defaults (Mode 1: confidence_threshold = 0.50,
// hysteresis_rounds = 2) require at least 2 rounds in a row meeting the
// threshold before emitting `Converged`. So `rounds_executed` MUST be
// ≥ 2 — if it ever returns 1, either the detector regressed (e.g. its
// hysteresis was disabled) or some upstream short-circuit started
// firing.
//
// We assert >= 2 (not >= 3 or higher) because:
// * Mode-1 spec is well-tuned for fast convergence.
// * On a perfectly aligned spec with no outliers, the essaim can
//   legitimately converge after exactly 2 rounds.
// * Asserting >= 3 would create false negatives on healthy runs.

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn realism_multi_round_fusion_runs_on_small() {
    let spec = load_v2_spec("small");
    let backend = build_backend_for(&spec);
    let req = Request::for_tests(spec.clone());
    let resp = backend.execute(req).await.expect("execute small v2");
    assert!(
        resp.rounds_executed >= 2,
        "small v2: rounds_executed = {}, expected >= 2 (consensus short-circuit?)",
        resp.rounds_executed
    );
    // Latency should also be > 0 — a 0-ms response is a sentinel that
    // the FusionAdapter timing instrumentation regressed.
    assert!(
        resp.total_ms > 0.0,
        "small v2: total_ms = {} (timing instrumentation broken?)",
        resp.total_ms
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn realism_multi_round_fusion_runs_on_medium() {
    let spec = load_v2_spec("medium");
    let backend = build_backend_for(&spec);
    let req = Request::for_tests(spec.clone());
    let resp = backend.execute(req).await.expect("execute medium v2");
    assert!(
        resp.rounds_executed >= 2,
        "medium v2: rounds_executed = {}, expected >= 2",
        resp.rounds_executed
    );
}

// =====================================================================
// 2. Output shape is realistic (raw centroid, right dimension)
// =====================================================================
//
// In Mode 1, the dissolver is `PassThroughDissolver` which returns a
// raw centroid of length `dim_latent`. If the bench accidentally hit
// a stub backend (`UnimplementedBackend` or similar), the output
// would be `Text("ok")` or empty Vec — both red flags.
//
// We check:
// * `output` is a `Raw(Vec<f32>)` variant.
// * The vector length equals `dim_latent`.
// * The vector is not all-zeros (would indicate the centroid was
//   never published by the FusionAdapter).

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn realism_output_is_realistic_centroid() {
    let spec = load_v2_spec("small");
    let dim = spec.dim_latent.get() as usize;
    let backend = build_backend_for(&spec);
    let req = Request::for_tests(spec);
    let resp = backend.execute(req).await.expect("execute");

    match resp.output {
        DecodedOutput::Raw(v) => {
            assert_eq!(
                v.len(),
                dim,
                "raw centroid len = {}, expected dim_latent = {dim}",
                v.len()
            );
            // Not all zeros: a single non-zero entry is enough proof
            // that the FusionAdapter actually published a centroid.
            let nonzero = v.iter().any(|x| x.abs() > 1e-9);
            assert!(
                nonzero,
                "raw centroid is all-zero — FusionAdapter::publish_centroid never ran?"
            );
        }
        other => panic!(
            "expected DecodedOutput::Raw (PassThroughDissolver invariant), got {:?}",
            other
        ),
    }
}

// =====================================================================
// 3. Bench loop does not leak heap memory
// =====================================================================
//
// Run N=20 requests sequentially against the same backend and check
// that the alloc residue (live bytes after GC) stays bounded. We do
// NOT install peak_alloc here (it pollutes the test crate's global
// allocator); instead we exploit the fact that a leaky path would
// blow up the system allocator's RSS. Since we cannot read RSS
// portably from a test, we settle for the weaker but reliable check:
// **all 20 executes complete successfully** with a confidence in
// the same ballpark, ruling out exhaustion of the memory pool.
//
// A real leak would either:
// * Exhaust the pool's `max_arenas = 8` after 9 iterations
//   (because arenas are not returned), causing a `MemoryPoolError`.
// * Cause `build_mvp_backend` to fail on the next test run because
//   the previous backend still holds arenas.
//
// Either signal trips this test. For a precise heap-bytes assertion,
// see the dedicated `real_memory.rs` bench, which DOES install
// peak_alloc.

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn realism_loop_does_not_leak_memory() {
    let spec = load_v2_spec("small");
    let backend = build_backend_for(&spec);
    const N: usize = 20;
    // Φ-D-OPS audit : tolerate ≤ 2 wall_clock budget exhaustions out
    // of N=20 iterations on slow / contended runners. The test's
    // intent — detect a real memory-pool leak — is preserved : a real
    // leak would either trigger `MemoryPoolError` (different
    // signature) OR cause far more than 2 iterations to fail (= every
    // iteration after the leak point would inherit the degraded
    // state). 2 ≪ 20 keeps the leak-detection signal strong.
    const MAX_BUDGET_FAILS: usize = 2;

    let mut confidences: Vec<f32> = Vec::with_capacity(N);
    let mut budget_fails = 0usize;
    for i in 0..N {
        let req = Request::for_tests(spec.clone());
        match backend.execute(req).await {
            Ok(resp) => confidences.push(resp.confidence),
            Err(e) => {
                let msg = format!("{e}");
                if msg.contains("wall_clock") || msg.contains("BudgetExhausted") {
                    budget_fails += 1;
                    assert!(
                        budget_fails <= MAX_BUDGET_FAILS,
                        "execute #{i} hit {budget_fails}-th wall_clock budget exhaustion (max {MAX_BUDGET_FAILS}); \
                         this is a real timing or scheduling issue, not transient flake",
                    );
                    continue;
                }
                panic!("execute #{i} failed: {e}");
            }
        }
    }

    assert!(
        confidences.len() >= N - MAX_BUDGET_FAILS,
        "loop completed only {} requests out of {N} (pool exhausted?)",
        confidences.len()
    );

    // Confidence should be reasonably stable — wild variance across
    // identical specs would suggest the consensus detector is reading
    // stale state (the per-request reset hook regressed).
    let n_obs = confidences.len() as f32;
    let mean: f32 = confidences.iter().sum::<f32>() / n_obs;
    let var: f32 = confidences.iter().map(|c| (c - mean).powi(2)).sum::<f32>() / n_obs;
    let stddev = var.sqrt();
    assert!(
        stddev < 0.20,
        "confidence stddev = {stddev:.3} across {N} identical requests — \
         per-request reset broken? mean = {mean:.3}, samples = {confidences:?}"
    );
}

// =====================================================================
// 4. Embedding norm is non-trivial (v2 fixtures sanity check)
// =====================================================================
//
// Belt-and-suspenders: if someone regenerates the v2 fixtures with a
// broken script (or copies the v1 zero-embedding fixture by accident),
// `HeuristicMetaAnalyzer::decompose` falls back to a uniform softmax
// and the bench measures a degenerate codepath. Refuse to run.

#[test]
fn realism_v2_fixtures_have_nontrivial_embeddings() {
    for fixture in ["small", "medium", "large"] {
        let spec = load_v2_spec(fixture);
        let norm: f32 = spec
            .problem_embedding
            .iter()
            .map(|v| v * v)
            .sum::<f32>()
            .sqrt();
        assert!(
            norm > 0.5,
            "{fixture} v2 fixture: embedding norm = {norm:.3}, expected > 0.5 \
             (regenerated with degenerate zero embedding?)"
        );
        assert!(
            norm < 10.0,
            "{fixture} v2 fixture: embedding norm = {norm:.3}, expected < 10 \
             (numerical instability risk in HeuristicMetaAnalyzer?)"
        );
    }
}
