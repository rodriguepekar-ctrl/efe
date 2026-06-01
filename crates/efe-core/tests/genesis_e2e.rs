//! End-to-end integration tests for the genesis pipeline + M1 controller.
//!
//! These tests validate that:
//! 1. `GenesisEngine` can be plugged into `SwarmController` without
//!    any signature tweaks.
//! 2. The produced `NanoAgentBatch` + `EpistemicLatentSpace` contain
//!    the expected number of agents and populated allocations.
//! 3. Determinism: same input → same output on HostBackend.

use std::sync::Arc;

use efe_core::els::EpistemicLatentSpace;
use efe_core::genesis::{
    BatchSpawner, Cardinality, CardinalityEstimator, DefaultSpawner, GenesisEngine,
    HeuristicEstimator, HeuristicMetaAnalyzer, MetaAnalyzer, SpawnerConfig, StaticRouter,
    StubTemplateRegistry, TemplateRegistry, TemplateRouter,
};
use efe_core::memory::{DeviceBackend, HostBackend, MemoryPool, NanoAgentBatch, PoolConfig};
use efe_core::streams::{StreamPool, StreamPoolConfig};
use efe_core::swarm::GenesisStrategy;
use efe_core::types::{
    AlignmentCone, LatentDim, PopulationMix, SwarmSpec, SwarmSpecBuilder, TemplateCode,
};

fn make_infra(dim: u32) -> (Arc<MemoryPool>, Arc<StreamPool>, Arc<dyn TemplateRegistry>) {
    let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
    let pool = Arc::new(
        MemoryPool::new(
            PoolConfig::default()
                .with_max_arenas(4)
                .with_prealloc(2)
                .with_arena_size(1 << 22),
            backend,
        )
        .expect("pool"),
    );
    let stream_pool =
        Arc::new(StreamPool::new(StreamPoolConfig::default().with_size(2)).expect("streams"));
    let registry: Arc<dyn TemplateRegistry> = Arc::new(StubTemplateRegistry::mvp_defaults(dim));
    (pool, stream_pool, registry)
}

fn make_engine(dim: u32) -> GenesisEngine {
    let (pool, streams, registry) = make_infra(dim);
    GenesisEngine::builder()
        .build_mvp(pool, registry, streams)
        .expect("engine")
}

fn make_spec(dim: u32) -> SwarmSpec {
    SwarmSpecBuilder::default()
        .problem_embedding(vec![0.05f32; dim as usize])
        .dim_latent(LatentDim::new(dim).expect("dim"))
        .population(PopulationMix::new(vec![(TemplateCode::NaSem, 128)]).expect("mix"))
        .alignment_cone(AlignmentCone::new(vec![vec![0.01f32; dim as usize]], 0.15).expect("cone"))
        .build()
        .expect("spec")
}

// =====================================================================
// Happy path
// =====================================================================

#[tokio::test]
async fn engine_spawn_produces_valid_batch_and_els() {
    let engine = make_engine(32);
    let spec = make_spec(32);
    let (batch, els) = engine.spawn(&spec).await.expect("spawn");
    assert!(batch.count() >= 128, "cardinality estimator min = 128");
    assert!(batch.has_arena(), "batch must own an arena handle");
    assert_eq!(batch.templates().len(), batch.count() as usize);
    assert_eq!(els.dim().get(), 32);
    assert_eq!(els.agent_count(), batch.count());
    assert!(els.is_populated());
    assert_eq!(els.anchor_count(), 1);
}

// =====================================================================
// Determinism
// =====================================================================

#[tokio::test]
async fn spawn_is_deterministic_across_calls_with_same_swarm_id() {
    let engine = make_engine(16);
    let spec = make_spec(16);

    let (batch1, els1) = engine.spawn(&spec).await.expect("first");
    let (batch2, els2) = engine.spawn(&spec).await.expect("second");

    assert_eq!(batch1.count(), batch2.count());
    assert_eq!(batch1.templates(), batch2.templates());

    // Compare latent bytes — HostBackend lets us read host pointers.
    let p1 = els1.latent_raw().expect("latent populated").ptr;
    let p2 = els2.latent_raw().expect("latent populated").ptr;
    let len = (batch1.count() as usize) * 16 * 2; // bf16
    unsafe {
        let a = std::slice::from_raw_parts(p1.as_raw().as_ptr(), len);
        let b = std::slice::from_raw_parts(p2.as_raw().as_ptr(), len);
        assert_eq!(a, b);
    }
}

// =====================================================================
// Integration with SwarmController (uses GenesisStrategy trait)
// =====================================================================

#[tokio::test]
async fn genesis_engine_implements_genesis_strategy() {
    let engine: Arc<dyn GenesisStrategy> = Arc::new(make_engine(16));
    let spec = make_spec(16);
    let (batch, els) = engine.spawn(&spec).await.expect("trait call");
    assert!(batch.count() > 0);
    assert!(els.is_populated());
}

// =====================================================================
// Resource lifecycle
// =====================================================================

#[tokio::test]
async fn dropping_batch_releases_arena_back_to_pool() {
    let (pool, streams, registry) = make_infra(16);
    let engine = GenesisEngine::builder()
        .build_mvp(pool.clone(), registry, streams)
        .expect("engine");
    let spec = make_spec(16);

    {
        let (_batch, _els) = engine.spawn(&spec).await.expect("spawn");
        assert_eq!(pool.metrics().arenas_active, 1);
    }
    assert_eq!(pool.metrics().arenas_active, 0);
}

// =====================================================================
// Custom pipeline stages (mocking via the trait system)
// =====================================================================

struct FixedCardinality(u32);

impl CardinalityEstimator for FixedCardinality {
    fn estimate(
        &self,
        _dag: &efe_core::genesis::TaskDag,
        distribution: &efe_core::genesis::TemplateDistribution,
        _ctx: &efe_core::genesis::GenesisContext,
    ) -> efe_core::Result<Cardinality> {
        Ok(distribution.distribute(self.0))
    }
}

#[tokio::test]
async fn custom_cardinality_overrides_default() {
    let dim = 16;
    let (pool, streams, registry) = make_infra(dim);
    let spec = make_spec(dim);

    let meta: Arc<dyn MetaAnalyzer> =
        Arc::new(HeuristicMetaAnalyzer::load_default(dim).expect("meta"));
    let router: Arc<dyn TemplateRouter> = Arc::new(StaticRouter::mvp_defaults().expect("router"));
    let cardinality: Arc<dyn CardinalityEstimator> = Arc::new(FixedCardinality(200));
    let spawner: Arc<dyn BatchSpawner> = Arc::new(DefaultSpawner::new(
        pool,
        registry,
        streams,
        SpawnerConfig::default(),
    ));

    let engine = GenesisEngine::builder()
        .meta(meta)
        .router(router)
        .cardinality(cardinality)
        .spawner(spawner)
        .build()
        .expect("build");

    let (batch, _els) = engine.spawn(&spec).await.expect("spawn");
    assert_eq!(batch.count(), 200);
}

#[test]
fn cardinality_validate_catches_sum_mismatch() {
    let c = Cardinality {
        total: 100,
        per_template: vec![(TemplateCode::NaSem, 50), (TemplateCode::NaLog, 30)],
    };
    assert!(c.validate().is_err());
}

#[tokio::test]
async fn batch_and_els_agree_on_counts() {
    let engine = make_engine(16);
    let spec = make_spec(16);
    let (batch, els) = engine.spawn(&spec).await.expect("ok");
    assert_eq!(batch.count(), els.agent_count());
    assert_eq!(els.dim().get(), 16);
}

// =====================================================================
// Error paths
// =====================================================================

#[tokio::test]
async fn spawn_rejects_spec_with_mismatched_embedding_dim() {
    let engine = make_engine(16);
    // Build a spec with dim 16 but the engine's meta-analyzer also
    // wants 16 — so we need to test another kind of mismatch.
    // Test: anchor dim mismatch.
    let spec = SwarmSpecBuilder::default()
        .problem_embedding(vec![0.1f32; 16])
        .dim_latent(LatentDim::new(16).unwrap())
        .population(PopulationMix::new(vec![(TemplateCode::NaSem, 8)]).unwrap())
        .alignment_cone(AlignmentCone::new(vec![vec![0.0f32; 8]], 0.15).unwrap())
        .build()
        .unwrap();
    let err = engine.spawn(&spec).await.unwrap_err();
    assert!(matches!(err, efe_core::EfeError::InvalidConfig(_)));
}

// =====================================================================
// HeuristicEstimator boundaries exercised through the engine
// =====================================================================

#[tokio::test]
async fn engine_respects_heuristic_bounds() {
    let engine = make_engine(32);
    let spec = make_spec(32);
    let (batch, _) = engine.spawn(&spec).await.unwrap();
    assert!(batch.count() >= HeuristicEstimator::default().min_agents);
    assert!(batch.count() <= HeuristicEstimator::default().max_agents);
}

// Silence "unused imports" warnings in the test compilation unit
// even when some helpers are only used by a subset of tests.
#[allow(dead_code)]
fn _keep_types_used(_: NanoAgentBatch, _: EpistemicLatentSpace) {}
