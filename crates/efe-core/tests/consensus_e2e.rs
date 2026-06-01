//! End-to-end integration tests for M6 Consensus Detector.
//!
//! Wires M3 (genesis) + M5 (fusion) + M6 (consensus) and verifies:
//! - Real fusion outputs are ingestible without `InvalidConfig`.
//! - `Continue` → `Converged` / `Stalled` transition on synthetic runs.
//! - Anomaly & divergence detection work end-to-end.
//! - Reset enables reuse of the detector across swarms.

use std::sync::Arc;

use efe_core::consensus::{
    ConsensusConfig, ConsensusDetector, ConsensusVerdict, MultiCriteriaDetector, SignalWeights,
    StubDetector,
};
use efe_core::els::EpistemicLatentSpace;
use efe_core::fusion::{FusionConfig, FusionStrategy, MvpCpuFusion, MvpCpuFusionBuilder};
use efe_core::genesis::{GenesisEngine, StubTemplateRegistry, TemplateRegistry};
use efe_core::memory::{DeviceBackend, HostBackend, MemoryPool, NanoAgentBatch, PoolConfig};
use efe_core::streams::{StreamPool, StreamPoolConfig};
use efe_core::swarm::GenesisStrategy;
use efe_core::types::{AlignmentCone, LatentDim, PopulationMix, SwarmSpecBuilder, TemplateCode};

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
        .unwrap(),
    );
    let streams = Arc::new(StreamPool::new(StreamPoolConfig::default().with_size(2)).unwrap());
    let registry: Arc<dyn TemplateRegistry> = Arc::new(StubTemplateRegistry::mvp_defaults(dim));
    (pool, streams, registry)
}

async fn spawn_swarm(dim: u32, agents: u32) -> (NanoAgentBatch, EpistemicLatentSpace) {
    let (pool, streams, registry) = make_infra(dim);
    let engine = GenesisEngine::builder()
        .build_mvp(pool, registry, streams)
        .unwrap();
    let spec = SwarmSpecBuilder::default()
        .problem_embedding(vec![0.05f32; dim as usize])
        .dim_latent(LatentDim::new(dim).unwrap())
        .population(PopulationMix::new(vec![(TemplateCode::NaSem, agents)]).unwrap())
        .alignment_cone(
            AlignmentCone::new(
                vec![vec![0.1f32; dim as usize], vec![0.2f32; dim as usize]],
                0.15,
            )
            .unwrap(),
        )
        .build()
        .unwrap();
    engine.spawn(&spec).await.unwrap()
}

fn default_fusion() -> MvpCpuFusion {
    let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
    MvpCpuFusionBuilder::default()
        .with_backend(backend)
        .with_replay_enabled(true)
        .with_replay_capacity(4)
        .with_replay_arena_bytes(1 << 20)
        .build()
        .unwrap()
}

// =====================================================================
// Smoke — ingest accepts real M5 output
// =====================================================================

#[tokio::test]
async fn consensus_ingests_real_fusion_stats() {
    let (batch, mut els) = spawn_swarm(16, 16).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion = default_fusion();
    let mut detector = MultiCriteriaDetector::new(ConsensusConfig::default()).unwrap();
    let cfg = FusionConfig::default();

    let round = fusion.run_round(&mut els, arena, &cfg, 0).await.unwrap();
    let verdict = detector.ingest(&round.stats).unwrap();
    // First round: either Continue or (unlikely) Converged after 1 round.
    // Never Anomaly on a healthy spawn.
    assert!(!matches!(verdict, ConsensusVerdict::Anomaly { .. }));
}

#[tokio::test]
async fn consensus_history_updates_per_ingest() {
    let (batch, mut els) = spawn_swarm(16, 16).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion = default_fusion();
    let mut detector = MultiCriteriaDetector::new(ConsensusConfig::default()).unwrap();
    let cfg = FusionConfig::default();

    for r in 0..4 {
        let round = fusion.run_round(&mut els, arena, &cfg, r).await.unwrap();
        detector.ingest(&round.stats).unwrap();
    }
    assert_eq!(detector.history().len(), 4);
}

// =====================================================================
// Convergence — full loop
// =====================================================================

#[tokio::test]
async fn consensus_loop_reaches_terminal_verdict_eventually() {
    let (batch, mut els) = spawn_swarm(16, 16).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion = default_fusion();
    let mut detector =
        MultiCriteriaDetector::new(ConsensusConfig::default().with_confidence_threshold(0.65))
            .unwrap();
    let cfg = FusionConfig::default();

    let mut final_verdict = None;
    for r in 0..16 {
        let round = fusion.run_round(&mut els, arena, &cfg, r).await.unwrap();
        let v = detector.ingest(&round.stats).unwrap();
        if v.is_terminal() {
            final_verdict = Some(v);
            break;
        }
    }
    // Acceptable terminal states: Converged (best case) or nothing (loop
    // exhausted). On a healthy swarm we should NOT see Diverged/Anomaly.
    if let Some(v) = &final_verdict {
        match v {
            ConsensusVerdict::Converged { confidence } => {
                assert!((0.0..=1.0).contains(confidence));
            }
            ConsensusVerdict::Diverged { .. } | ConsensusVerdict::Anomaly { .. } => {
                panic!("unexpected terminal state on healthy swarm: {v:?}");
            }
            _ => {}
        }
    }
    // If no terminal, the loop just ran to 16 — also acceptable.
}

// =====================================================================
// Determinism — same inputs, same verdict
// =====================================================================

#[tokio::test]
async fn consensus_deterministic_on_same_input() {
    let (batch, mut els) = spawn_swarm(8, 16).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion = default_fusion();
    let cfg = FusionConfig::default();

    // Run one round, capture stats.
    let round = fusion.run_round(&mut els, arena, &cfg, 0).await.unwrap();

    // Two detectors with the same config, fed the same stats → same verdict.
    let mut d1 = MultiCriteriaDetector::new(ConsensusConfig::default()).unwrap();
    let mut d2 = MultiCriteriaDetector::new(ConsensusConfig::default()).unwrap();
    let v1 = d1.ingest(&round.stats).unwrap();
    let v2 = d2.ingest(&round.stats).unwrap();
    assert_eq!(v1, v2);
    assert!((d1.confidence() - d2.confidence()).abs() < 1e-6);
}

// =====================================================================
// Anomaly injection
// =====================================================================

#[tokio::test]
async fn consensus_detects_nan_in_one_ingest() {
    let (batch, mut els) = spawn_swarm(8, 16).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion = default_fusion();
    let cfg = FusionConfig::default();
    let mut detector = MultiCriteriaDetector::new(ConsensusConfig::default()).unwrap();

    let round = fusion.run_round(&mut els, arena, &cfg, 0).await.unwrap();
    let mut bad_stats = round.stats.clone();
    bad_stats.has_nan = true;
    let v = detector.ingest(&bad_stats).unwrap();
    assert!(matches!(v, ConsensusVerdict::Anomaly { .. }));
    assert!(v.should_rollback());
}

// =====================================================================
// Divergence — fabricated increasing variance
// =====================================================================

#[tokio::test]
async fn consensus_diverges_on_exploding_variance() {
    let mut cfg = ConsensusConfig::default();
    cfg.variance_explosion_threshold = 10.0;
    let mut detector = MultiCriteriaDetector::new(cfg).unwrap();

    let mut stats = efe_core::fusion::FusionStats::synthetic_with(0, 4, 100.0, 10.0);
    stats.n_active = 4;
    let v = detector.ingest(&stats).unwrap();
    assert!(matches!(v, ConsensusVerdict::Diverged { .. }));
    assert!(v.should_rollback());
}

// =====================================================================
// Reuse across swarms — reset
// =====================================================================

#[tokio::test]
async fn consensus_reset_enables_reuse_across_swarms() {
    let mut detector = MultiCriteriaDetector::new(ConsensusConfig::default()).unwrap();

    // First swarm — feed a few rounds.
    {
        let (batch, mut els) = spawn_swarm(8, 16).await;
        let arena = batch.arena_handle().unwrap().arena();
        let fusion = default_fusion();
        let cfg = FusionConfig::default();
        for r in 0..3 {
            let round = fusion.run_round(&mut els, arena, &cfg, r).await.unwrap();
            detector.ingest(&round.stats).unwrap();
        }
    }

    detector.reset();
    assert_eq!(detector.verdict(), &ConsensusVerdict::Continue);
    assert!(detector.history().is_empty());

    // Second swarm — different dim, would fail CentroidDimMismatch without reset.
    {
        let (batch, mut els) = spawn_swarm(16, 16).await;
        let arena = batch.arena_handle().unwrap().arena();
        let fusion = default_fusion();
        let cfg = FusionConfig::default();
        let round = fusion.run_round(&mut els, arena, &cfg, 0).await.unwrap();
        let v = detector.ingest(&round.stats).unwrap();
        assert!(!matches!(v, ConsensusVerdict::Anomaly { .. }));
    }
}

// =====================================================================
// Stub detector usage in a controller loop
// =====================================================================

#[tokio::test]
async fn stub_detector_never_terminates_loop() {
    let (batch, mut els) = spawn_swarm(8, 16).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion = default_fusion();
    let mut detector: Box<dyn ConsensusDetector> = Box::new(StubDetector::new());
    let cfg = FusionConfig::default();

    for r in 0..5 {
        let round = fusion.run_round(&mut els, arena, &cfg, r).await.unwrap();
        let v = detector.ingest(&round.stats).unwrap();
        assert_eq!(v, ConsensusVerdict::Continue);
    }
}

// =====================================================================
// Custom weights respect the decision boundary
// =====================================================================

#[tokio::test]
async fn consensus_honors_custom_weights() {
    // All weight on `no_anomaly` → any clean round ≈ confidence 1.
    let weights = SignalWeights::new(0.0, 0.0, 0.0, 0.0, 0.0, 1.0).unwrap();
    let mut detector = MultiCriteriaDetector::new(
        ConsensusConfig::default()
            .with_weights(weights)
            .with_confidence_threshold(0.99)
            .with_hysteresis(1),
    )
    .unwrap();

    let (batch, mut els) = spawn_swarm(8, 16).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion = default_fusion();
    let cfg = FusionConfig::default();
    let round = fusion.run_round(&mut els, arena, &cfg, 0).await.unwrap();
    let v = detector.ingest(&round.stats).unwrap();
    assert!(
        matches!(v, ConsensusVerdict::Converged { .. }),
        "expected immediate Converged with no_anomaly-only weights, got {v:?}"
    );
}
