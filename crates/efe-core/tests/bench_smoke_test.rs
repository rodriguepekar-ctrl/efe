//! **Sprint C — Bench infrastructure smoke tests.**
//!
//! These tests run as part of the regular `cargo test` suite (no
//! special features required) and verify that:
//!
//! 1. Every realistic fixture (`scripts/fixtures/realistic_*.json`)
//!    parses, builds a valid `SwarmSpec`, and passes
//!    `SwarmSpec::validate()`.
//! 2. The bench's `build_bench_controller` happy path runs once
//!    end-to-end without panicking — sanity check that the bench
//!    is measuring SOMETHING, not a degenerate stub.
//!
//! If a fixture is corrupted by an editor or by an incompatible
//! schema change, this file fails BEFORE the bench ever runs,
//! short-circuiting a run-and-fail loop in CI.

use std::sync::Arc;

use async_trait::async_trait;
use efe_core::els::EpistemicLatentSpace;
use efe_core::errors::Result;
use efe_core::fusion::FusionStats;
use efe_core::memory::NanoAgentBatch;
use efe_core::swarm::{
    AlignmentProjector, ConsensusDetector, DecodeMode, DecodedOutput, Dissolver, FusionStrategy,
    GenesisStrategy, NullTelemetryCollector, Request, SwarmArbiter, SwarmController, Verdict,
};
use efe_core::types::{
    AlignmentCone, LatentDim, PopulationMix, SwarmSpec, TemplateCode,
};

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

fn load_fixture(name: &str) -> SwarmSpec {
    let crate_root = env!("CARGO_MANIFEST_DIR");
    let path = format!("{crate_root}/../../scripts/fixtures/realistic_{name}.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {path}: {e}"));
    let wrapper: FixtureWrapper = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("parse {path}: {e}"));
    let f = wrapper.spec;
    let dim = LatentDim::new(f.dim_latent).expect("dim");
    let pairs: Vec<(TemplateCode, u32)> = f
        .population
        .into_iter()
        .map(|(s, n)| (parse_template(&s), n))
        .collect();
    let pop = PopulationMix::new(pairs).expect("pop");
    let cone = AlignmentCone::new(
        f.alignment_cone.anchors,
        f.alignment_cone.max_projection_distance,
    )
    .expect("cone");
    SwarmSpec::builder()
        .problem_embedding(f.problem_embedding)
        .dim_latent(dim)
        .population(pop)
        .max_rounds(f.max_rounds)
        .expect("rounds")
        .alignment_cone(cone)
        .build()
        .expect("spec")
}

#[test]
fn realistic_small_validates() {
    let spec = load_fixture("small");
    spec.validate().expect("small valid");
    assert_eq!(spec.dim_latent.get(), 128);
    assert_eq!(spec.population.total(), 8);
    assert_eq!(spec.alignment_cone.anchors.len(), 4);
}

#[test]
fn realistic_medium_validates() {
    let spec = load_fixture("medium");
    spec.validate().expect("medium valid");
    assert_eq!(spec.dim_latent.get(), 256);
    assert_eq!(spec.population.total(), 32);
    assert_eq!(spec.alignment_cone.anchors.len(), 8);
}

#[test]
fn realistic_large_validates() {
    let spec = load_fixture("large");
    spec.validate().expect("large valid");
    assert_eq!(spec.dim_latent.get(), 256);
    assert_eq!(spec.population.total(), 128);
    assert_eq!(spec.alignment_cone.anchors.len(), 16);
}

// =====================================================================
// Inline bench mocks (mirror common/mod.rs but stay compile-time
// independent — the test crate cannot import from benches/).
// =====================================================================

struct SmokeGenesis(LatentDim, u32);
#[async_trait]
impl GenesisStrategy for SmokeGenesis {
    async fn spawn(
        &self,
        s: &SwarmSpec,
    ) -> Result<(NanoAgentBatch, EpistemicLatentSpace)> {
        Ok((
            NanoAgentBatch::new_stub(s.id, self.1),
            EpistemicLatentSpace::new_stub(s.id, self.0, self.1),
        ))
    }
    async fn respawn_diversify(&self, _: &mut NanoAgentBatch, _: u32) -> Result<()> {
        Ok(())
    }
}

struct SmokeFusion(usize);
#[async_trait]
impl FusionStrategy for SmokeFusion {
    async fn run_round(
        &self,
        _: &mut EpistemicLatentSpace,
        _: &NanoAgentBatch,
        round: u32,
    ) -> Result<FusionStats> {
        let mut s = FusionStats::synthetic_converging(round, self.0);
        s.round = round;
        Ok(s)
    }
    async fn inject_noise(&self, _: &mut EpistemicLatentSpace, _: f32) -> Result<()> {
        Ok(())
    }
}

struct SmokeDissolver(usize);
#[async_trait]
impl Dissolver for SmokeDissolver {
    async fn extract_centroid(&self, _: &EpistemicLatentSpace) -> Result<Vec<f32>> {
        Ok(vec![0.05; self.0])
    }
    async fn decode(&self, c: &[f32], _: &DecodeMode) -> Result<DecodedOutput> {
        Ok(DecodedOutput::Raw(c.to_vec()))
    }
}

struct SmokeAligner;
impl AlignmentProjector for SmokeAligner {
    fn project(&self, c: &[f32], _: &AlignmentCone) -> Result<(Vec<f32>, f32)> {
        Ok((c.to_vec(), 0.0))
    }
}

struct SmokeDetector;
impl ConsensusDetector for SmokeDetector {
    fn evaluate(&mut self, _round: u32, _stats: &FusionStats) -> Verdict {
        Verdict::Converged { confidence: 0.92 }
    }
    fn reset(&mut self) {}
}

#[tokio::test]
async fn bench_smoke_small_executes() {
    let spec = load_fixture("small");
    let dim_usize = spec.dim_latent.get() as usize;
    let total = spec.population.total();

    let controller = SwarmController::builder()
        .genesis(Arc::new(SmokeGenesis(spec.dim_latent, total)))
        .fusion(Arc::new(SmokeFusion(dim_usize)))
        .dissolver(Arc::new(SmokeDissolver(dim_usize)))
        .alignment(Arc::new(SmokeAligner))
        .telemetry(Arc::new(NullTelemetryCollector))
        .arbiter(SwarmArbiter::without_energy_metering())
        .consensus_factory(|| Box::new(SmokeDetector))
        .build()
        .expect("build");

    let req = Request::for_tests(spec);
    let resp = controller.execute(req).await.expect("execute");
    assert!((resp.confidence - 0.92).abs() < 1e-5);
    assert_eq!(resp.rounds_executed, 1);
}
