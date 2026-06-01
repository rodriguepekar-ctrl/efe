//! Shared helpers for the Sprint C benches (`swarm_e2e`, `memory_e2e`).
//!
//! Provides:
//! - `load_realistic_spec(name)` — read JSON fixtures from
//!   `scripts/fixtures/realistic_{name}.json`
//! - `BenchMockGenesis`, `BenchMockFusion`, `BenchMockDissolver`,
//!   `BenchMockAligner`, `ConstDetector` — minimal mocks usable
//!   from `benches/`. We re-implement them here (rather than reuse
//!   `tests/mocks.rs`) because Cargo treats `tests/` and `benches/`
//!   as separate compilation units and a `mod tests::mocks` import
//!   would not resolve from a bench.
//!
//! The mocks are **deliberately minimalist**: they execute only the
//! controller/strategy plumbing, not real numerical work. This keeps
//! the bench focused on the orchestration overhead. To benchmark the
//! actual compute kernels, use `compute_sprint{2,3,4}.rs`.

#![allow(dead_code)]

use std::sync::Arc;

use async_trait::async_trait;
use efe_core::els::EpistemicLatentSpace;
use efe_core::errors::Result;
use efe_core::fusion::FusionStats;
use efe_core::memory::NanoAgentBatch;
use efe_core::swarm::{
    AlignmentProjector, ConsensusDetector, ControllerConfig, DecodeMode, DecodedOutput,
    Dissolver, FusionStrategy, GenesisStrategy, NullTelemetryCollector, Request, SwarmArbiter,
    SwarmController, Verdict,
};
use efe_core::types::{
    AlignmentCone, LatentDim, PopulationMix, SwarmSpec, TemplateCode,
};

// ====================================================================
// Spec loader
// ====================================================================

/// Bench-friendly partial spec read from JSON, then built via the
/// [`SwarmSpec::builder`] (which fills in `id`, `convergence`,
/// `budget` defaults).
///
/// The fixture JSON deliberately omits the runtime-only fields
/// (`id`, `convergence`, `budget`) — they are not part of what an
/// external user provides ; the controller fills them in. By
/// deserializing a partial then building, we keep the fixtures
/// human-readable and stable across runs.
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

/// Map a `"NaSem"` / `"NaLog"` / `"NaCrit"` string to the matching
/// [`TemplateCode`] variant.
fn parse_template(s: &str) -> TemplateCode {
    match s {
        "NaSem" => TemplateCode::NaSem,
        "NaLog" => TemplateCode::NaLog,
        "NaCrit" => TemplateCode::NaCrit,
        other => panic!("unknown template_code in fixture: `{other}`"),
    }
}

/// Load a realistic SwarmSpec from `scripts/fixtures/realistic_{name}.json`.
///
/// Path resolution: `CARGO_MANIFEST_DIR` is `<repo>/crates/efe-core`,
/// and the fixtures live at `<repo>/scripts/fixtures/`.
///
/// # Panics
///
/// Panics if the JSON cannot be read, parsed, or built into a valid
/// SwarmSpec. Benches must not silently paper over fixture errors.
pub fn load_realistic_spec(name: &str) -> SwarmSpec {
    let crate_root = env!("CARGO_MANIFEST_DIR");
    let path = format!(
        "{crate_root}/../../scripts/fixtures/realistic_{name}.json"
    );
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read fixture {path}: {e}"));
    let wrapper: FixtureWrapper = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("parse fixture {path}: {e}"));
    let f = wrapper.spec;

    // Build via the builder so SwarmId / convergence / budget defaults
    // are filled in by the crate.
    let dim = LatentDim::new(f.dim_latent)
        .unwrap_or_else(|e| panic!("invalid dim_latent in {path}: {e}"));
    let pop_pairs: Vec<(TemplateCode, u32)> = f
        .population
        .into_iter()
        .map(|(s, n)| (parse_template(&s), n))
        .collect();
    let pop = PopulationMix::new(pop_pairs)
        .unwrap_or_else(|e| panic!("invalid population in {path}: {e}"));
    let cone = AlignmentCone::new(f.alignment_cone.anchors, f.alignment_cone.max_projection_distance)
        .unwrap_or_else(|e| panic!("invalid alignment_cone in {path}: {e}"));

    let spec = SwarmSpec::builder()
        .problem_embedding(f.problem_embedding)
        .dim_latent(dim)
        .population(pop)
        .max_rounds(f.max_rounds)
        .unwrap_or_else(|e| panic!("invalid max_rounds in {path}: {e}"))
        .alignment_cone(cone)
        .build()
        .unwrap_or_else(|e| panic!("build SwarmSpec from {path}: {e}"));
    spec.validate()
        .unwrap_or_else(|e| panic!("fixture {path} fails validate: {e}"));
    spec
}

// ====================================================================
// Bench-local mocks
// ====================================================================

/// Genesis mock that returns a stub batch + ELS sized from the spec.
pub struct BenchMockGenesis {
    pub dim: LatentDim,
    pub count: u32,
}

#[async_trait]
impl GenesisStrategy for BenchMockGenesis {
    async fn spawn(
        &self,
        spec: &SwarmSpec,
    ) -> Result<(NanoAgentBatch, EpistemicLatentSpace)> {
        let batch = NanoAgentBatch::new_stub(spec.id, self.count);
        let els = EpistemicLatentSpace::new_stub(spec.id, self.dim, self.count);
        Ok((batch, els))
    }
    async fn respawn_diversify(
        &self,
        _: &mut NanoAgentBatch,
        _: u32,
    ) -> Result<()> {
        Ok(())
    }
}

/// Fusion mock that always converges with a fixed `FusionStats`.
pub struct BenchMockFusion {
    stats: FusionStats,
}

impl BenchMockFusion {
    pub fn new(dim: usize) -> Self {
        Self {
            stats: FusionStats::synthetic_converging(1, dim),
        }
    }
}

#[async_trait]
impl FusionStrategy for BenchMockFusion {
    async fn run_round(
        &self,
        _: &mut EpistemicLatentSpace,
        _: &NanoAgentBatch,
        round: u32,
    ) -> Result<FusionStats> {
        let mut s = self.stats.clone();
        s.round = round;
        Ok(s)
    }
    async fn inject_noise(&self, _: &mut EpistemicLatentSpace, _: f32) -> Result<()> {
        Ok(())
    }
}

/// Dissolver mock that returns a fixed centroid.
pub struct BenchMockDissolver {
    pub centroid: Vec<f32>,
}

impl BenchMockDissolver {
    pub fn new(dim: usize) -> Self {
        Self {
            centroid: vec![0.05; dim],
        }
    }
}

#[async_trait]
impl Dissolver for BenchMockDissolver {
    async fn extract_centroid(&self, _: &EpistemicLatentSpace) -> Result<Vec<f32>> {
        Ok(self.centroid.clone())
    }
    async fn decode(&self, c: &[f32], _: &DecodeMode) -> Result<DecodedOutput> {
        Ok(DecodedOutput::Raw(c.to_vec()))
    }
}

/// Identity-projection aligner with delta=0 (always passes the breach
/// check, even with very strict cone radii).
pub struct BenchMockAligner;

impl AlignmentProjector for BenchMockAligner {
    fn project(&self, c: &[f32], _: &AlignmentCone) -> Result<(Vec<f32>, f32)> {
        Ok((c.to_vec(), 0.0))
    }
}

/// Detector that always converges on the first round with a fixed
/// confidence. Cheaper than scripting the full sequence.
pub struct ConstConvergeDetector {
    pub confidence: f32,
}

impl ConsensusDetector for ConstConvergeDetector {
    fn evaluate(&mut self, _round: u32, _stats: &FusionStats) -> Verdict {
        Verdict::Converged {
            confidence: self.confidence,
        }
    }
    fn reset(&mut self) {}
}

// ====================================================================
// Controller builder
// ====================================================================

/// Build a fully wired SwarmController with all bench mocks.
///
/// `converge_at_round` decides at which round the consensus detector
/// emits Converged. For the throughput / latency bench we use round 1
/// (single-round happy path) — keeping the bench focused on the
/// genesis + dissolution overhead. For larger scenarios we can use
/// `spec.max_rounds` to exercise the full multi-round loop.
pub fn build_bench_controller(
    spec: &SwarmSpec,
    converge_at_round: u32,
) -> SwarmController {
    let dim_usize = spec.dim_latent.get() as usize;
    let total_count: u32 = spec.population.total();

    SwarmController::builder()
        .genesis(Arc::new(BenchMockGenesis {
            dim: spec.dim_latent,
            count: total_count,
        }))
        .fusion(Arc::new(BenchMockFusion::new(dim_usize)))
        .dissolver(Arc::new(BenchMockDissolver::new(dim_usize)))
        .alignment(Arc::new(BenchMockAligner))
        .telemetry(Arc::new(NullTelemetryCollector))
        .arbiter(SwarmArbiter::without_energy_metering())
        .consensus_factory(move || {
            // Generate `converge_at_round - 1` Continue then a Converged.
            let mut verdicts: Vec<Verdict> = (1..converge_at_round)
                .map(|_| Verdict::Continue)
                .collect();
            verdicts.push(Verdict::Converged { confidence: 0.92 });
            Box::new(ScriptedDetectorBench { verdicts, idx: 0 })
        })
        .config(ControllerConfig::default())
        .build()
        .expect("build bench controller")
}

/// Local scripted detector (mirrors `tests::mocks::ScriptedDetector`).
struct ScriptedDetectorBench {
    verdicts: Vec<Verdict>,
    idx: usize,
}

impl ConsensusDetector for ScriptedDetectorBench {
    fn evaluate(&mut self, _round: u32, _stats: &FusionStats) -> Verdict {
        let v = self
            .verdicts
            .get(self.idx)
            .copied()
            .unwrap_or(Verdict::Continue);
        self.idx += 1;
        v
    }
    fn reset(&mut self) {
        self.idx = 0;
    }
}

/// Build a fresh `Request` for the bench. The request id is randomized
/// (UUIDv4) — the bench measures the per-request execution cost, not
/// the request setup.
pub fn build_request(spec: &SwarmSpec) -> Request {
    Request::for_tests(spec.clone())
}

// ====================================================================
// Sprint D2 — Memory pool / arena helpers
// ====================================================================
//
// These helpers are consumed by `memory_real_arena.rs` (the D2 bench).
// They live here rather than in a D2-only module because:
//
// 1. `make_runtime` is generally useful for any async bench that does
//    not want to pay the per-iter cost of building a tokio runtime.
//    Keeping it here makes it available to `swarm_e2e` and any future
//    async bench at zero extra LOC.
//
// 2. `make_pool` is parameterised so any caller can dial the prealloc
//    / max / arena_size triple. The D2 bench uses three distinct
//    profiles (warm, cold, contention); having one factory keeps the
//    bench file focused on measurement, not setup.
//
// Both helpers are `#[allow(dead_code)]` because the existing Sprint C
// benches (`swarm_e2e`, `memory_e2e`) do not consume them yet — the
// `dead_code` lint would otherwise nag on every Sprint C build.

use efe_core::memory::{DeviceBackend, HostBackend, MemoryPool, PoolConfig};

/// Build a `MemoryPool` with explicit `prealloc / max_arenas /
/// default_arena_size`. Bench-tuned defaults are intentionally
/// **not** baked in here so the caller can sweep the parameter space
/// (e.g. `make_pool(0, 1, 1 << 20)` for cold-acquire isolation,
/// `make_pool(8, 8, 1 << 20)` for fully warmed concurrent acquire).
///
/// The backend is always [`HostBackend`] — the only `DeviceBackend`
/// available without the gated `cuda` feature. CUDA bench coverage
/// is part of the Phase 2 roadmap (cf.
/// `docs/bench/d2_memory_real_arena_architecture.md` §13.1).
///
/// # Panics
///
/// Panics if [`PoolConfig::validate`] or [`MemoryPool::new`] fails —
/// invariants the caller is expected to respect (see
/// [`PoolConfig::with_arena_size`] for the alignment / minimum-size
/// rules).
#[allow(dead_code)]
#[must_use]
pub fn make_pool(prealloc: usize, max: usize, default_arena_size: u64) -> MemoryPool {
    let cfg = PoolConfig::default()
        .with_prealloc(prealloc)
        .with_max_arenas(max)
        .with_arena_size(default_arena_size);
    let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
    MemoryPool::new(cfg, backend).expect("memory pool")
}

/// Build a multi-threaded tokio runtime with 4 worker threads. Mirror
/// of the D1 `make_runtime()` helper in `efe-server/benches/`.
///
/// Why 4 workers and not `num_cpus::get()` :
///
/// * Stable bench reproducibility across hosts (CI runners ↔ M3 Pro).
/// * Matches the production Mode-1 server's typical RT.
/// * Avoids over-saturating a 4-core CI runner with bench tasks +
///   criterion's own runtime.
///
/// One runtime per bench invocation (created in `bench_*` and shared
/// across iterations) keeps the per-iter cost free of runtime build
/// overhead (~5 ms) which would otherwise contaminate every sample.
#[allow(dead_code)]
#[must_use]
pub fn make_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("tokio runtime")
}
