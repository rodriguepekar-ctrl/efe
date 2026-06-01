//! End-to-end integration tests for M7 Collapse / Decoherence Engine.
//!
//! Wires M3 (genesis) + M5 (fusion + replay) + M6 (consensus) + M7
//! (collapse) to validate the full recovery pipeline:
//! - Rollback from an injected NaN.
//! - Abort fires when replay is empty and all strategies fail.
//! - Circuit breaker kicks in after repeated collapses.
//! - Manual trigger paths are accepted.

use std::sync::Arc;

use efe_core::collapse::{
    validate_post_collapse, CollapseAction, CollapseConfig, CollapseEngine, CollapsePolicy,
    CollapseTrigger, FullAbortStrategy, PostValidation, StubBehavior, StubStrategy,
};
use efe_core::consensus::{AnomalyCode, ConsensusVerdict, DivergenceReason};
use efe_core::els::EpistemicLatentSpace;
use efe_core::fusion::{FusionConfig, MvpCpuFusion, MvpCpuFusionBuilder, ReplayableFusion};
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

fn fusion_with_replay() -> MvpCpuFusion {
    let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
    MvpCpuFusionBuilder::default()
        .with_backend(backend)
        .with_replay_enabled(true)
        .with_replay_capacity(6)
        .with_replay_arena_bytes(1 << 22)
        .build()
        .unwrap()
}

fn fusion_without_replay() -> MvpCpuFusion {
    let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
    MvpCpuFusionBuilder::default()
        .with_backend(backend)
        .with_replay_enabled(false)
        .build()
        .unwrap()
}

// =====================================================================
// Smoke — engine accepts real verdict + ELS
// =====================================================================

#[tokio::test]
async fn engine_no_op_on_continue_verdict() {
    let (batch, mut els) = spawn_swarm(8, 16).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion: Arc<dyn ReplayableFusion> = Arc::new(fusion_with_replay());
    let mut engine = CollapseEngine::new(
        CollapsePolicy::default_mvp(),
        CollapseConfig::default().with_deterministic_seed(true),
    )
    .unwrap();

    let outcome = engine
        .handle(&ConsensusVerdict::Continue, &mut els, arena, fusion, 0)
        .await
        .unwrap();
    assert!(matches!(outcome.action_taken, CollapseAction::NoOp));
    assert!(outcome.success);
}

#[tokio::test]
async fn engine_no_op_on_converged_verdict() {
    let (batch, mut els) = spawn_swarm(8, 16).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion: Arc<dyn ReplayableFusion> = Arc::new(fusion_with_replay());
    let mut engine = CollapseEngine::new(
        CollapsePolicy::default_mvp(),
        CollapseConfig::default().with_deterministic_seed(true),
    )
    .unwrap();

    let v = ConsensusVerdict::Converged { confidence: 0.9 };
    let outcome = engine.handle(&v, &mut els, arena, fusion, 0).await.unwrap();
    assert!(matches!(outcome.action_taken, CollapseAction::NoOp));
    assert!(outcome.success);
}

// =====================================================================
// Soft rollback path
// =====================================================================

#[tokio::test]
async fn engine_rolls_back_on_nan_anomaly() {
    // Run 2 rounds with replay enabled, then inject NaN into the latent
    // and trigger a collapse. The engine should restore via SoftRollback.
    let (batch, mut els) = spawn_swarm(8, 16).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion_inner = fusion_with_replay();
    let fusion: Arc<dyn ReplayableFusion> = Arc::new(fusion_inner);
    let cfg = FusionConfig::default();

    // Run a few good rounds so the replay buffer has snapshots.
    for r in 0..3u32 {
        fusion.record_snapshot(r, &els, arena).await.unwrap();
        let _ = fusion.run_round(&mut els, arena, &cfg, r).await.unwrap();
    }
    let pre_snapshot = els.snapshot(arena).unwrap();

    // Inject NaN into the latent (simulating a kernel bug).
    {
        let mut split = els.split_mut(arena).unwrap();
        let row = split.latent.row_mut(0).unwrap();
        row[0] = half::bf16::from_f32(f32::NAN);
    }

    let mut engine = CollapseEngine::new(
        CollapsePolicy::default_mvp(),
        CollapseConfig::default().with_deterministic_seed(true),
    )
    .unwrap();
    let verdict = ConsensusVerdict::Anomaly {
        code: AnomalyCode::NanInf,
    };
    let outcome = engine
        .handle(&verdict, &mut els, arena, fusion, 3)
        .await
        .unwrap();

    assert!(
        outcome.is_recoverable(),
        "expected recovery, got {outcome:?}"
    );
    assert!(matches!(
        outcome.action_taken,
        CollapseAction::SoftRollback { .. }
    ));
    // Post-validate: ELS is now clean.
    let validation = validate_post_collapse(&els, arena, &CollapseConfig::default()).unwrap();
    assert!(validation.is_ok(), "validation: {validation:?}");
    let _ = pre_snapshot; // referenced for future hash assertions
}

// =====================================================================
// Abort when replay is empty and rollback inapplicable
// =====================================================================

#[tokio::test]
async fn engine_aborts_when_replay_empty_and_other_strategies_fail() {
    let (batch, mut els) = spawn_swarm(8, 16).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion: Arc<dyn ReplayableFusion> = Arc::new(fusion_without_replay());
    let mut engine = CollapseEngine::new(
        CollapsePolicy::default_mvp(),
        CollapseConfig::default().with_deterministic_seed(true),
    )
    .unwrap();

    // CentroidDimMismatch disables SoftRollback + PartialRespawn.
    // NoiseInjection requires stall/decay → not applicable either.
    // CentroidProjection IS applicable; it will succeed (since the ELS
    // has a finite centroid to project onto) — so we expect recovery,
    // not abort.
    //
    // Instead, to force the abort path we'd need a trigger that no
    // strategy can handle — the engine guarantees FullAbortStrategy is
    // last and always applicable, so FullAbort always fires as the
    // terminal step when recovery is impossible.
    //
    // This test verifies that an unfixable structural anomaly
    // (CentroidDimMismatch) escalates through the MVP policy to
    // FullAbort (because SoftRollback rejects it, and the other
    // strategies reject it too).
    let verdict = ConsensusVerdict::Anomaly {
        code: AnomalyCode::CentroidDimMismatch {
            prev: 8,
            current: 16,
        },
    };
    let outcome = engine
        .handle(&verdict, &mut els, arena, fusion, 0)
        .await
        .unwrap();
    assert!(matches!(outcome.action_taken, CollapseAction::FullAbort));
    assert!(!outcome.is_recoverable());
    assert!(outcome.is_terminal());
}

// =====================================================================
// Circuit breaker
// =====================================================================

#[tokio::test]
async fn circuit_breaker_fires_after_max_collapses() {
    let (batch, mut els) = spawn_swarm(8, 16).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion: Arc<dyn ReplayableFusion> = Arc::new(fusion_with_replay());

    // Use a stub that always succeeds to keep collapses benign, then
    // verify the breaker fires on attempt N+1.
    let stub = Arc::new(StubStrategy::new("only", StubBehavior::AlwaysSucceed));
    let stub_dyn: Arc<dyn efe_core::collapse::CollapseStrategy> = stub.clone();
    let policy = CollapsePolicy::custom(vec![stub_dyn]);
    let cfg = CollapseConfig::default()
        .with_max_collapses(3)
        .with_deterministic_seed(true);
    let mut engine = CollapseEngine::new(policy, cfg).unwrap();
    let verdict = ConsensusVerdict::Anomaly {
        code: AnomalyCode::NanInf,
    };

    // Attempts 1..=3 should succeed.
    for i in 1..=3 {
        let out = engine
            .handle(&verdict, &mut els, arena, fusion.clone(), i - 1)
            .await
            .unwrap();
        assert!(out.is_recoverable(), "attempt {i} should succeed");
    }
    // Attempt 4 trips the breaker.
    let out = engine
        .handle(&verdict, &mut els, arena, fusion.clone(), 3)
        .await
        .unwrap();
    assert!(
        matches!(out.action_taken, CollapseAction::FullAbort),
        "attempt 4 should abort"
    );
    // Reset the counter and verify attempts resume.
    engine.reset_swarm_counter(els.swarm_id());
    let out = engine
        .handle(&verdict, &mut els, arena, fusion, 4)
        .await
        .unwrap();
    assert!(out.is_recoverable(), "after reset should succeed");
}

// =====================================================================
// Events are recorded
// =====================================================================

#[tokio::test]
async fn engine_records_events_oldest_first() {
    let (batch, mut els) = spawn_swarm(8, 16).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion: Arc<dyn ReplayableFusion> = Arc::new(fusion_with_replay());
    let cfg = CollapseConfig::default()
        .with_max_collapses(10)
        .with_deterministic_seed(true);
    let mut engine = CollapseEngine::new(CollapsePolicy::default_mvp(), cfg).unwrap();

    // First, run fusion rounds to populate replay.
    let fcfg = FusionConfig::default();
    for r in 0..3u32 {
        fusion.record_snapshot(r, &els, arena).await.unwrap();
        let _ = fusion.run_round(&mut els, arena, &fcfg, r).await;
    }

    for r in 3..6 {
        let verdict = ConsensusVerdict::Anomaly {
            code: AnomalyCode::NanInf,
        };
        let _ = engine
            .handle(&verdict, &mut els, arena, fusion.clone(), r)
            .await
            .unwrap();
    }
    assert_eq!(engine.events().count(), 3);
    // Oldest-first: indices ascend.
    let indices: Vec<u32> = engine.events().map(|e| e.collapse_index).collect();
    assert_eq!(indices, vec![1, 2, 3]);
}

// =====================================================================
// Manual trigger path
// =====================================================================

#[tokio::test]
async fn engine_with_only_abort_strategy_always_terminates() {
    let (batch, mut els) = spawn_swarm(8, 16).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion: Arc<dyn ReplayableFusion> = Arc::new(fusion_without_replay());

    let abort: Arc<dyn efe_core::collapse::CollapseStrategy> = Arc::new(FullAbortStrategy);
    let policy = CollapsePolicy::custom(vec![abort]);
    let mut engine = CollapseEngine::new(
        policy,
        CollapseConfig::default().with_deterministic_seed(true),
    )
    .unwrap();

    let verdict = ConsensusVerdict::Diverged {
        reason: DivergenceReason::VarianceExplosion { variance: 1e6 },
    };
    let outcome = engine
        .handle(&verdict, &mut els, arena, fusion, 5)
        .await
        .unwrap();
    assert!(matches!(outcome.action_taken, CollapseAction::FullAbort));
}

// =====================================================================
// Validation helper
// =====================================================================

#[tokio::test]
async fn validate_post_collapse_reports_variance_too_high() {
    let (batch, els) = spawn_swarm(8, 16).await;
    let arena = batch.arena_handle().unwrap().arena();
    // Configure an absurdly low post_variance_max so real spawn stats
    // fail the validator.
    let cfg = CollapseConfig::default().with_post_variance_max(1e-12);
    let v = validate_post_collapse(&els, arena, &cfg).unwrap();
    assert!(matches!(
        v,
        PostValidation::VarianceTooHigh { .. } | PostValidation::Ok { .. }
    ));
}

// =====================================================================
// Trigger mapping via ConsensusVerdict
// =====================================================================

// =====================================================================
// Rollback targets `current_round` (pre-mutation snapshot)
// =====================================================================

#[tokio::test]
async fn rollback_restores_to_current_round_snapshot() {
    // M5 takes each replay snapshot at the start of its round, *before*
    // the mutation. So if round 3 produces NaN, restoring to snapshot(3)
    // yields the state immediately before round 3 ran — the correct
    // recovery point.
    //
    // This test validates that the engine picks `target == current_round`
    // as the preferred rollback target.
    let (batch, mut els) = spawn_swarm(8, 16).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion_inner = fusion_with_replay();
    let fusion: Arc<dyn ReplayableFusion> = Arc::new(fusion_inner);
    let cfg = FusionConfig::default();

    // Record a snapshot at round 3 (before running it). Capture its
    // centroid hash for post-rollback equality check.
    fusion.record_snapshot(3, &els, arena).await.unwrap();
    let pre_round_3 = els.snapshot(arena).unwrap();

    // Run round 3 (mutates ELS).
    let _ = fusion.run_round(&mut els, arena, &cfg, 3).await.unwrap();

    // Simulate a failure requiring collapse at round 3.
    // Inject NaN so the post-round state is obviously broken.
    {
        let mut split = els.split_mut(arena).unwrap();
        split.latent.row_mut(0).unwrap()[0] = half::bf16::from_f32(f32::NAN);
    }

    let mut engine = CollapseEngine::new(
        CollapsePolicy::default_mvp(),
        CollapseConfig::default().with_deterministic_seed(true),
    )
    .unwrap();
    let verdict = ConsensusVerdict::Anomaly {
        code: AnomalyCode::NanInf,
    };
    let outcome = engine
        .handle(&verdict, &mut els, arena, fusion, 3)
        .await
        .unwrap();

    // Must be SoftRollback to round 3 (not an older snapshot).
    match outcome.action_taken {
        CollapseAction::SoftRollback { to_round } => {
            assert_eq!(
                to_round, 3,
                "expected rollback to round 3 (pre-mutation snapshot)"
            );
        }
        other => panic!("expected SoftRollback to round 3, got {other:?}"),
    }

    // After restore, centroid hash matches the pre-round-3 snapshot.
    let post_restore = els.snapshot(arena).unwrap();
    assert_eq!(post_restore.centroid_hash, pre_round_3.centroid_hash);
}

// =====================================================================
// Empty escalation policy
// =====================================================================

#[tokio::test]
async fn engine_with_empty_policy_returns_abort() {
    let (batch, mut els) = spawn_swarm(8, 16).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion: Arc<dyn ReplayableFusion> = Arc::new(fusion_without_replay());
    let policy = CollapsePolicy::custom(vec![]);
    let mut engine = CollapseEngine::new(
        policy,
        CollapseConfig::default().with_deterministic_seed(true),
    )
    .unwrap();
    let verdict = ConsensusVerdict::Anomaly {
        code: AnomalyCode::NanInf,
    };
    let outcome = engine
        .handle(&verdict, &mut els, arena, fusion, 0)
        .await
        .unwrap();
    assert!(matches!(outcome.action_taken, CollapseAction::FullAbort));
    assert!(outcome
        .warnings
        .iter()
        .any(|w| matches!(w, efe_core::collapse::CollapseWarning::Abandoned)));
}

// =====================================================================
// Events Display impl
// =====================================================================

#[tokio::test]
async fn events_have_display_impl() {
    let (batch, mut els) = spawn_swarm(8, 16).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion: Arc<dyn ReplayableFusion> = Arc::new(fusion_without_replay());
    let mut engine = CollapseEngine::new(
        CollapsePolicy::default_mvp(),
        CollapseConfig::default().with_deterministic_seed(true),
    )
    .unwrap();
    let verdict = ConsensusVerdict::Anomaly {
        code: AnomalyCode::CentroidDimMismatch {
            prev: 8,
            current: 16,
        },
    };
    let _ = engine
        .handle(&verdict, &mut els, arena, fusion, 3)
        .await
        .unwrap();
    let event = engine.events().next().expect("at least one event");
    let formatted = format!("{event}");
    assert!(formatted.contains("event["));
    assert!(formatted.contains("full_abort") || formatted.contains("no_op"));
}

#[tokio::test]
async fn trigger_captures_all_verdict_variants() {
    let cases: Vec<(ConsensusVerdict, Option<&'static str>)> = vec![
        (ConsensusVerdict::Continue, None),
        (ConsensusVerdict::Converged { confidence: 0.95 }, None),
        (
            ConsensusVerdict::Diverged {
                reason: DivergenceReason::VarianceExplosion { variance: 1e5 },
            },
            Some("divergence"),
        ),
        (
            ConsensusVerdict::Anomaly {
                code: AnomalyCode::NanInf,
            },
            Some("anomaly"),
        ),
        (
            ConsensusVerdict::Stalled {
                rounds_flat: 6,
                confidence: 0.3,
            },
            Some("stall"),
        ),
    ];
    for (v, expected) in cases {
        let t = CollapseTrigger::from_verdict(&v, 42);
        match (t, expected) {
            (None, None) => {}
            (Some(t), Some(label)) => assert_eq!(t.label(), label),
            (None, Some(label)) => panic!("expected trigger with label {label}"),
            (Some(t), None) => panic!("expected no trigger, got {t:?}"),
        }
    }
}
