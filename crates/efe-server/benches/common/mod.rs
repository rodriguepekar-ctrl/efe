//! **Sprint D1 — Real-strategies bench common helpers.**
//!
//! Shared by `real_strategies_e2e.rs` (latency, throughput, per-phase,
//! BLAS vs naïve) and `real_memory.rs` (heap profiling). The two bench
//! binaries live in distinct compilation units, so any code they share
//! has to live in this `common` module — Cargo will compile it once
//! per bench (a few hundred ms; negligible).
//!
//! # Why these helpers exist
//!
//! Sprint C's bench harness in `efe-core/benches/common/mod.rs`
//! deliberately wires *mock* strategies (`BenchMockGenesis`,
//! `BenchMockFusion`, …) — they exist to measure the controller's
//! orchestration overhead, not the cost of real numerical work. The
//! Sprint D1 bench measures the **production wiring**
//! (`efe_server::mode1::build_mvp_backend`) as it actually runs
//! against gRPC requests in production. To do that, the bench needs:
//!
//! 1. **Realistic v2 fixtures** with non-degenerate problem
//!    embeddings — the v1 fixtures had `[0.0; dim]` problem vectors
//!    which collapse `HeuristicMetaAnalyzer::decompose` into a uniform
//!    softmax (every task receives equal weight, the bench measures a
//!    pathological branch). The v2 fixtures use Box-Muller Gaussians
//!    (see `scripts/gen_fixtures_v2.py`) so the meta-analyzer produces
//!    a realistic task DAG.
//!
//! 2. **A pre-tuned `Mode1Config`** with the **D6-hardened consensus
//!    knobs** (`confidence_threshold=0.65`, `hysteresis_rounds=3`,
//!    `variance_threshold=0.02`, etc.) plus the `controller.min_rounds=4`
//!    floor that guarantees the centroid is swept at least 4 rounds
//!    before any `Converged` verdict is honoured. See
//!    `Mode1Config::default()` for the full Sprint D6 rationale.
//!    Pre-D6, this config "loosened" the thresholds — that produced
//!    the 0-round-consensus / accuracy-50 % bug. The current bench
//!    measures the post-D6 multi-round behaviour faithfully.
//!
//! 3. **A factory** that returns a fresh, fully-wired
//!    `Arc<dyn ExecuteBackend>` for each bench iteration — this is the
//!    same call (`build_mvp_backend(cfg)`) that the production binary
//!    in `main.rs` makes, so the bench faithfully reproduces the cost
//!    of the real critical path.
//!
//! # Path resolution
//!
//! `CARGO_MANIFEST_DIR` is the directory of `crates/efe-server/Cargo.toml`,
//! so fixtures resolve via `../../scripts/fixtures/realistic_*_v2.json`.
//! The `_v2` suffix is *deliberate* — Sprint C benches keep using the
//! v1 fixtures so their numbers stay comparable to the v0.1.0 baseline
//! recorded in `bench_baselines/v0.1.0/`.

#![allow(dead_code)] // Some helpers are only used by one of the two bench binaries.

use std::sync::Arc;

use efe_core::swarm::Request;
use efe_core::types::{
    AlignmentCone, LatentDim, PopulationMix, ResourceBudget, SwarmSpec, TemplateCode,
};

use efe_server::mode1::{build_mvp_backend, Mode1Config};
use efe_server::services::ExecuteBackend;

// =====================================================================
// Fixture I/O
// =====================================================================

/// Wrapper that mirrors the top-level `{"spec": …}` shape of the
/// v2 JSON fixtures. Keeping the JSON schema unchanged across v1 and
/// v2 lets us reuse the same loader logic with a single suffix
/// difference; only the *contents* of `problem_embedding` change.
#[derive(serde::Deserialize)]
struct FixtureWrapper {
    spec: FixtureSpec,
}

/// Per-fixture body. `id`, `convergence`, and `budget` are intentionally
/// absent — they are runtime-only fields filled in by `SwarmSpec::builder`.
/// Stable, human-readable JSON ⇒ no churn when the controller version-bumps
/// its budget defaults.
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

/// Map a `"NaSem"` / `"NaLog"` / `"NaCrit"` string to the corresponding
/// [`TemplateCode`] variant. Any other value triggers an immediate
/// panic — a corrupted fixture should fail loudly, not produce silent
/// garbage in a benchmark report.
fn parse_template(s: &str) -> TemplateCode {
    match s {
        "NaSem" => TemplateCode::NaSem,
        "NaLog" => TemplateCode::NaLog,
        "NaCrit" => TemplateCode::NaCrit,
        other => panic!("unknown template_code in v2 fixture: `{other}`"),
    }
}

/// Load a realistic v2 SwarmSpec from `scripts/fixtures/realistic_{name}_v2.json`.
///
/// Valid `name` values are `"small"`, `"medium"`, `"large"` (matching
/// the artefacts produced by `scripts/gen_fixtures_v2.py`).
///
/// The loader:
///
/// 1. Reads the JSON file via `CARGO_MANIFEST_DIR`-relative path
///    resolution.
/// 2. Parses it into a partial `FixtureSpec`.
/// 3. Builds a `SwarmSpec` via the public builder, so the runtime-only
///    fields (`id`, `convergence`, `budget`) get the same defaults as
///    a request that arrived through gRPC.
/// 4. Calls `SwarmSpec::validate()` to fail fast if the fixture has
///    drifted away from the validation rules.
///
/// # Panics
///
/// Panics on any I/O, parse, builder, or validation failure. Benches
/// must not silently paper over fixture errors — a measurement built
/// on a broken fixture is worse than no measurement at all.
pub fn load_realistic_v2_spec(name: &str) -> SwarmSpec {
    let crate_root = env!("CARGO_MANIFEST_DIR");
    let path = format!("{crate_root}/../../scripts/fixtures/realistic_{name}_v2.json");
    let raw =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read v2 fixture {path}: {e}"));
    let wrapper: FixtureWrapper =
        serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse v2 fixture {path}: {e}"));
    let f = wrapper.spec;

    let dim = LatentDim::new(f.dim_latent)
        .unwrap_or_else(|e| panic!("invalid dim_latent in {path}: {e}"));
    let pop_pairs: Vec<(TemplateCode, u32)> = f
        .population
        .into_iter()
        .map(|(s, n)| (parse_template(&s), n))
        .collect();
    let pop = PopulationMix::new(pop_pairs)
        .unwrap_or_else(|e| panic!("invalid population in {path}: {e}"));
    let cone = AlignmentCone::new(
        f.alignment_cone.anchors,
        f.alignment_cone.max_projection_distance,
    )
    .unwrap_or_else(|e| panic!("invalid alignment_cone in {path}: {e}"));

    // Override the default `ResourceBudget` (500 ms wall-clock) with
    // a generous 30 s. Rationale:
    //
    // * The bench runs in **release** mode (~10× faster than debug),
    //   but the large v2 fixture still needs ≈ 50-200 ms per request
    //   on a typical CPU bench host. 500 ms leaves no headroom.
    // * The realism check tests (`tests/bench_realism_check.rs`)
    //   reuse this loader transitively. They run in **debug** mode
    //   where a single request can stretch to 1-3 s. 30 s eliminates
    //   any flakiness without hiding actual hangs (the controller's
    //   `max_rounds` cap fires long before 30 s).
    // * 30 s is consistent with `tests/mode1_live.rs` which uses a
    //   10 s wall-clock budget — bench fixtures are larger so we
    //   use 3× that.
    let mut budget = ResourceBudget::default();
    budget.max_wall_clock_ms = 30_000;

    let spec = SwarmSpec::builder()
        .problem_embedding(f.problem_embedding)
        .dim_latent(dim)
        .population(pop)
        .max_rounds(f.max_rounds)
        .unwrap_or_else(|e| panic!("invalid max_rounds in {path}: {e}"))
        .alignment_cone(cone)
        .budget(budget)
        .build()
        .unwrap_or_else(|e| panic!("build SwarmSpec from {path}: {e}"));
    spec.validate()
        .unwrap_or_else(|e| panic!("v2 fixture {path} fails validate: {e}"));
    spec
}

// =====================================================================
// Mode1Config — bench-tuned defaults
// =====================================================================

/// Build a `Mode1Config` for a given `dim_latent`, with **memory-pool
/// sizing tuned for benches**.
///
/// Why we override `prealloc_arenas` / `max_arenas`:
///
/// * The **production default** (`prealloc_arenas = 4`,
///   `max_arenas = 8`) is sized for an MVP server that handles a
///   moderate concurrency level on a small box. The bench measures a
///   *single sequential request at a time* in the latency benches, so
///   pre-allocating 4 arenas is wasteful (each arena is 1 GiB on host
///   memory).
/// * For the **throughput** bench we drive 8 concurrent swarms; we
///   stay at the production max of 8 to mirror the real cap.
///
/// We compromise on a single config (`prealloc_arenas = 2,
/// max_arenas = 8`) that:
///
/// * Pre-allocates exactly enough to amortise the genesis cost across
///   the warm-up + first measured iteration.
/// * Leaves the throughput cap at 8 so the throughput bench saturates.
/// * Doesn't blow up RSS on a 16-GB CI runner (2 GiB peak vs 4 GiB at
///   prod default).
///
/// All other knobs (consensus tuning, fusion config, stream pool size)
/// are inherited from `Mode1Config::default()` — the bench measures the
/// production-tuned regime, not a custom setup.
#[must_use]
pub fn default_mode1_config(dim_latent: u32) -> Mode1Config {
    // `Mode1Config` is `#[non_exhaustive]` but provides a public
    // `for_dim(u32)` factory that internally fills in defaults. The
    // factory is the *only* way to build one from outside `efe-server`;
    // we mutate the resulting fields (which are `pub`) to override the
    // pool sizing. This pattern is recommended by the Mode1Config doc
    // comment: see `for_dim` in `crates/efe-server/src/mode1.rs`.
    let mut cfg = Mode1Config::for_dim(dim_latent);
    cfg.prealloc_arenas = 2;
    cfg.max_arenas = 8;
    // n_streams: keep at 4 (production default) — the CPU stream pool
    // is essentially a tokio::Semaphore on host backend, so tuning has
    // negligible bench impact.
    cfg
}

// =====================================================================
// Backend factory
// =====================================================================

/// Construct a fresh, fully-wired [`ExecuteBackend`] for a given
/// `SwarmSpec`. This is *the* call the production binary makes in
/// `main.rs`; the bench reuses it verbatim so we measure the real
/// critical path and not a custom assembly.
///
/// Returned as `Arc<dyn ExecuteBackend>` (the production polymorphic
/// type) so the bench code can treat it identically to what the gRPC
/// service handler holds.
///
/// # Panics
///
/// Panics if `build_mvp_backend` fails. This should not happen at
/// realistic fixture sizes — every code path that can fail there
/// (memory pool init, template registry build, controller wiring) has
/// dedicated unit tests in `mode1::tests`. A panic here means the
/// bench itself is broken; the operator should investigate before
/// trusting any number coming out of this run.
#[must_use]
pub fn build_real_backend(spec: &SwarmSpec) -> Arc<dyn ExecuteBackend> {
    let cfg = default_mode1_config(spec.dim_latent.get());
    build_mvp_backend(cfg).unwrap_or_else(|e| {
        panic!(
            "build_mvp_backend failed for dim={} : {e}",
            spec.dim_latent.get()
        )
    })
}

/// Build a fresh `Request` carrying a *clone* of the bench spec.
///
/// The bench measures **per-request execution cost**, so each
/// iteration receives its own request id (UUID v4) — this avoids
/// idempotency-cache short-circuits if such a cache is wired in front
/// of the backend in a future build, and matches what real clients
/// produce.
///
/// `Request::for_tests` constructs the rest of the fields with sane
/// defaults: `tenant_id = "test"`, `decode_mode = PassThrough`,
/// `raw_prompt = None`, `deadline = None`, `trace_context = None` —
/// all aligned with the steady-state Mode-1 traffic that
/// `build_mvp_backend` is wired for.
#[must_use]
pub fn build_real_request(spec: &SwarmSpec) -> Request {
    Request::for_tests(spec.clone())
}

// =====================================================================
// Sanity-check helpers (used by `tests/bench_realism_check.rs`)
// =====================================================================

/// Inspect the L2 norm of `spec.problem_embedding`. The v2 fixtures
/// target `‖e‖₂ ≈ √dim · 0.1` (≈ 1.13 at dim=128, ≈ 1.6 at dim=256);
/// any drift below 0.1 means we accidentally regenerated with v1-style
/// zero embeddings, and the bench would be measuring a degenerate
/// codepath. Used by the realism check tests, but exposed as a `pub`
/// helper so external sanity scripts can poke at the same number.
#[must_use]
pub fn problem_embedding_norm(spec: &SwarmSpec) -> f32 {
    let emb = &spec.problem_embedding;
    let sum_sq: f32 = emb.iter().map(|v| v * v).sum();
    sum_sq.sqrt()
}

/// Inspect the population total. Same rationale: a fixture-cropping
/// regression would silently shrink the bench workload to something
/// trivial, and the latency numbers would look great for entirely the
/// wrong reason.
#[must_use]
pub fn population_total(spec: &SwarmSpec) -> u32 {
    spec.population.total()
}

#[cfg(test)]
mod tests {
    //! Smoke tests for the helpers themselves — they run as part of
    //! `cargo test -p efe-server` (no `--bench` needed).
    //!
    //! `allow(unused_imports)`: in bench compilation
    //! (`cargo bench`/`cargo check --benches`), `#[test]` functions
    //! are stripped before the body is type-checked but the module
    //! still compiles. The result is that `use super::*` looks
    //! unused. Suppressing the warning is preferable to making each
    //! import explicit, since it is genuinely *used* in test runs.

    #[allow(unused_imports)]
    use super::*;

    #[test]
    fn small_v2_loads_and_validates() {
        let spec = load_realistic_v2_spec("small");
        assert_eq!(spec.dim_latent.get(), 128);
        assert_eq!(spec.population.total(), 8);
        assert_eq!(spec.alignment_cone.anchors.len(), 4);
        // Norm sanity: v2 fixtures target ~1.0–1.5; any value below
        // 0.1 means we regenerated with v1-style zero embeddings.
        let n = problem_embedding_norm(&spec);
        assert!(n > 0.5, "v2 small embedding norm unexpectedly low: {n}");
    }

    #[test]
    fn medium_v2_loads_and_validates() {
        let spec = load_realistic_v2_spec("medium");
        assert_eq!(spec.dim_latent.get(), 256);
        assert_eq!(spec.population.total(), 32);
    }

    #[test]
    fn large_v2_loads_and_validates() {
        let spec = load_realistic_v2_spec("large");
        assert_eq!(spec.dim_latent.get(), 256);
        assert_eq!(spec.population.total(), 128);
    }

    #[test]
    fn default_mode1_config_overrides_pool_sizing() {
        let cfg = default_mode1_config(256);
        assert_eq!(cfg.dim_latent, 256);
        assert_eq!(cfg.prealloc_arenas, 2);
        assert_eq!(cfg.max_arenas, 8);
    }

    #[tokio::test]
    async fn real_backend_executes_one_round() {
        // End-to-end smoke: load fixture → build backend → execute once.
        // If any wiring step regresses, this test trips immediately
        // rather than producing a misleading bench number.
        let spec = load_realistic_v2_spec("small");
        let backend = build_real_backend(&spec);
        let req = build_real_request(&spec);
        let resp = backend
            .execute(req)
            .await
            .expect("real backend must execute small v2 fixture");
        assert!(
            resp.rounds_executed >= 1,
            "real backend executed 0 rounds — controller wiring is broken"
        );
        assert!(
            (0.0..=1.0).contains(&resp.confidence),
            "confidence out of [0, 1]: {}",
            resp.confidence
        );
    }
}
