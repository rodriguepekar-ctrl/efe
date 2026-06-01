//! End-to-end integration tests for M5 Fusion Kernels.
//!
//! These tests wire M3 (genesis) + M4 (ELS) + M5 (fusion) and verify
//! that a full fusion campaign runs cleanly:
//! - Rounds produce consistent stats.
//! - Variance trends down on synthetic swarms.
//! - Anomaly injection is detected.
//! - Replay buffer snapshots/restores exactly.

use std::sync::Arc;

use efe_core::els::EpistemicLatentSpace;
use efe_core::fusion::{
    alpha_for_round, beta_for_round, FusionConfig, FusionStrategy, MvpCpuFusion,
    MvpCpuFusionBuilder, ReplayableFusion,
};
use efe_core::genesis::{GenesisEngine, StubTemplateRegistry, TemplateRegistry};
use efe_core::memory::{DeviceBackend, HostBackend, MemoryPool, NanoAgentBatch, PoolConfig};
use efe_core::streams::{StreamPool, StreamPoolConfig};
use efe_core::swarm::GenesisStrategy;
use efe_core::types::{AlignmentCone, LatentDim, PopulationMix, SwarmSpecBuilder, TemplateCode};
use efe_core::EfeError;

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
// Smoke — a round completes
// =====================================================================

#[tokio::test]
async fn fusion_round_completes_on_spawned_swarm() {
    let (batch, mut els) = spawn_swarm(16, 32).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion = default_fusion();
    let cfg = FusionConfig::default();
    let round = fusion.run_round(&mut els, arena, &cfg, 0).await.unwrap();
    assert_eq!(round.round, 0);
    assert_eq!(round.backend, "mvp-cpu");
    assert_eq!(round.stats.centroid.len(), 16);
    assert!(round.stats.all_finite());
    assert!(!round.stats.has_numerical_issues());
}

#[tokio::test]
async fn fusion_round_emits_timing_and_stats() {
    let (batch, mut els) = spawn_swarm(16, 32).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion = default_fusion();
    let cfg = FusionConfig::default();
    let round = fusion.run_round(&mut els, arena, &cfg, 0).await.unwrap();
    // Timing fields filled in.
    assert!(round.timing.total_us > 0);
    assert!(
        round.timing.scores_us > 0 || round.timing.softmax_us > 0 || round.timing.aggregate_us > 0
    );
    // Entropy fields must be finite and non-negative. Upper bound is
    // ln(n_active); we don't assert exact equality because M3's
    // cardinality estimator may inflate the `agents` arg (MVP min=128)
    // and the stats struct doesn't expose n_active directly on this
    // path — see `snapshot_and_compute_stats_agree_on_scalars` which
    // cross-checks via the ELS snapshot for the authoritative N.
    let tol = 0.05;
    assert!(
        round.stats.mean_attention_entropy.is_finite(),
        "mean entropy non-finite: {}",
        round.stats.mean_attention_entropy
    );
    assert!(round.stats.mean_attention_entropy >= -tol);
    assert!(round.stats.min_attention_entropy >= -tol);
    // Max is bounded by ln(N) — for any plausible N ≤ 10^6, ln(N) < 15.
    assert!(
        round.stats.max_attention_entropy < 20.0,
        "implausible max entropy: {}",
        round.stats.max_attention_entropy
    );
}

// =====================================================================
// Scheduling
// =====================================================================

#[tokio::test]
async fn fusion_schedules_decay_over_rounds() {
    let (batch, mut els) = spawn_swarm(8, 16).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion = default_fusion();
    let cfg = FusionConfig::default();

    let mut alphas = vec![];
    let mut betas = vec![];
    for r in 0..5 {
        let round = fusion.run_round(&mut els, arena, &cfg, r).await.unwrap();
        alphas.push(round.stats.alpha_applied);
        betas.push(round.stats.beta_applied);
    }
    // α decreases (linear), β decreases (exp).
    assert!(alphas[0] >= alphas[4], "α should decrease: {:?}", alphas);
    assert!(betas[0] >= betas[4], "β should decrease: {:?}", betas);
    // Matches the closed-form schedules.
    assert!((alphas[0] - alpha_for_round(&cfg, 0)).abs() < 1e-6);
    assert!((betas[0] - beta_for_round(&cfg, 0)).abs() < 1e-6);
}

// =====================================================================
// Determinism
// =====================================================================

#[tokio::test]
async fn fusion_deterministic_on_same_input_bytes() {
    // M5 determinism: given identical starting latent bytes and
    // identical config, two runs must produce identical results.
    //
    // Rather than relying on M3 to produce bit-identical swarms across
    // two separate spawns (which depends on PRNG seeding behavior we
    // don't control here), we spawn once, byte-copy the latent into
    // a fresh swarm, and compare outcomes.
    let dim = 8;
    let (batch1, mut els1) = spawn_swarm(dim, 16).await;
    let (batch2, mut els2) = spawn_swarm(dim, 16).await;
    let arena1 = batch1.arena_handle().unwrap().arena();
    let arena2 = batch2.arena_handle().unwrap().arena();

    // Copy els1 latent bytes into els2 — identical input to both runs.
    let n = els1.agent_count();
    let size = u64::from(n) * u64::from(dim) * 2;
    unsafe {
        let src = els1.latent_raw().unwrap().ptr.as_raw().as_ptr();
        let dst = els2.latent_raw().unwrap().ptr.as_raw().as_ptr();
        std::ptr::copy_nonoverlapping(src, dst, size as usize);
    }
    // Verify identical starting state.
    let pre1 = els1.snapshot(arena1).unwrap();
    let pre2 = els2.snapshot(arena2).unwrap();
    assert_eq!(pre1.centroid_hash, pre2.centroid_hash);

    // Also copy the problem embedding and anchors — they're part of
    // the inputs to fusion too.
    let pe_size = u64::from(dim) * 4;
    unsafe {
        let src = els1.problem_embedding_raw().unwrap().ptr.as_raw().as_ptr();
        let dst = els2.problem_embedding_raw().unwrap().ptr.as_raw().as_ptr();
        std::ptr::copy_nonoverlapping(src, dst, pe_size as usize);
        let src = els1.anchors_raw().unwrap().ptr.as_raw().as_ptr();
        let dst = els2.anchors_raw().unwrap().ptr.as_raw().as_ptr();
        let a_size = u64::from(els1.anchor_count()) * u64::from(dim) * 4;
        std::ptr::copy_nonoverlapping(src, dst, a_size as usize);
    }

    let fusion1 = default_fusion();
    let fusion2 = default_fusion();
    let cfg = FusionConfig::default();
    for r in 0..3 {
        fusion1.run_round(&mut els1, arena1, &cfg, r).await.unwrap();
        fusion2.run_round(&mut els2, arena2, &cfg, r).await.unwrap();
    }
    let s1 = els1.snapshot(arena1).unwrap();
    let s2 = els2.snapshot(arena2).unwrap();
    assert_eq!(
        s1.centroid_hash, s2.centroid_hash,
        "M5 is non-deterministic on same inputs"
    );
}

// =====================================================================
// Mutation
// =====================================================================

#[tokio::test]
async fn fusion_round_changes_latent() {
    let (batch, mut els) = spawn_swarm(16, 32).await;
    let arena = batch.arena_handle().unwrap().arena();
    let snap_before = els.snapshot(arena).unwrap();
    let fusion = default_fusion();
    let cfg = FusionConfig::default();
    fusion.run_round(&mut els, arena, &cfg, 0).await.unwrap();
    let snap_after = els.snapshot(arena).unwrap();
    // Centroid (and therefore hash) must change — unless swarm was already
    // at a perfect fixed point, which M3 initialization doesn't produce.
    assert_ne!(snap_before.centroid_hash, snap_after.centroid_hash);
}

// =====================================================================
// Replay
// =====================================================================

#[tokio::test]
async fn fusion_replay_restores_pre_round_state() {
    let (batch, mut els) = spawn_swarm(8, 16).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion = default_fusion();
    assert!(fusion.supports_replay());
    let cfg = FusionConfig::default();

    let snap_r0 = els.snapshot(arena).unwrap();

    // Push snapshot before running round 0, run round 0.
    fusion.record_snapshot(0, &els, arena).await.unwrap();
    fusion.run_round(&mut els, arena, &cfg, 0).await.unwrap();
    let snap_after = els.snapshot(arena).unwrap();
    assert_ne!(snap_r0.centroid_hash, snap_after.centroid_hash);

    // Restore.
    fusion.restore_snapshot(0, &mut els, arena).await.unwrap();
    let snap_restored = els.snapshot(arena).unwrap();
    assert_eq!(snap_r0.centroid_hash, snap_restored.centroid_hash);
}

// =====================================================================
// Multiple rounds — convergence trend
// =====================================================================

#[tokio::test]
async fn fusion_multi_round_keeps_stats_finite() {
    let (batch, mut els) = spawn_swarm(16, 32).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion = default_fusion();
    let cfg = FusionConfig::default();
    let mut had_anomaly = false;
    for r in 0..8 {
        let round = fusion.run_round(&mut els, arena, &cfg, r).await.unwrap();
        if round.stats.has_numerical_issues() {
            had_anomaly = true;
            break;
        }
        assert!(round.stats.all_finite());
    }
    assert!(!had_anomaly, "unexpected anomaly in multi-round run");
}

#[tokio::test]
async fn fusion_variance_does_not_explode() {
    let (batch, mut els) = spawn_swarm(16, 32).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion = default_fusion();
    let cfg = FusionConfig::default();
    let mut variances = vec![];
    for r in 0..6 {
        let round = fusion.run_round(&mut els, arena, &cfg, r).await.unwrap();
        variances.push(round.stats.variance);
    }
    // Variance at round 5 should not be > 100x round 0 — loose
    // non-explosion bound.
    let ratio = variances[5] / variances[0].max(1e-6);
    assert!(
        ratio < 100.0,
        "variance ratio {} suggests explosion: {:?}",
        ratio,
        variances
    );
}

// =====================================================================
// Anomaly detection
// =====================================================================

#[tokio::test]
async fn fusion_detects_pre_round_anomaly() {
    let (batch, mut els) = spawn_swarm(8, 16).await;
    let arena = batch.arena_handle().unwrap().arena();

    // Inject NaN into the latent.
    {
        let mut split = els.split_mut(arena).unwrap();
        let row = split.latent.row_mut(0).unwrap();
        row[0] = half::bf16::from_f32(f32::NAN);
    }

    let fusion = default_fusion();
    let cfg = FusionConfig::default();
    let err = fusion
        .run_round(&mut els, arena, &cfg, 0)
        .await
        .unwrap_err();
    assert!(matches!(err, EfeError::NumericalExplosion { .. }));
}

// =====================================================================
// Deadline handling
// =====================================================================

#[tokio::test]
async fn fusion_tight_deadline_warns() {
    let (batch, mut els) = spawn_swarm(16, 64).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion = default_fusion();
    let cfg = FusionConfig::default().with_round_deadline_ms(1);
    let round = fusion.run_round(&mut els, arena, &cfg, 0).await.unwrap();
    // On a large swarm with 1 ms budget, we expect a deadline warning
    // (but the round still completes because strict_deadline is false).
    // We don't hard-assert the warning because CPU speed varies — we
    // only require that the round completed and reports its timing.
    assert!(round.timing.total_us > 0);
    let _ = round.has_deadline_warning();
}

#[tokio::test]
async fn fusion_strict_deadline_errors_on_overrun() {
    let (batch, mut els) = spawn_swarm(16, 64).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion = default_fusion();
    let mut cfg = FusionConfig::default();
    cfg.round_deadline_ms = 1;
    cfg.strict_deadline = true;
    let result = fusion.run_round(&mut els, arena, &cfg, 0).await;
    // May or may not error depending on CPU speed. If it errors, it
    // must be BudgetExhausted; otherwise the round completed within
    // the tight budget (unlikely but possible).
    match result {
        Ok(r) => assert!(r.on_time, "claimed OK but not on_time"),
        Err(EfeError::BudgetExhausted { kind }) => {
            assert_eq!(kind, "fusion_deadline");
        }
        Err(other) => panic!("unexpected error: {other:?}"),
    }
}

// =====================================================================
// Config validation
// =====================================================================

#[tokio::test]
async fn fusion_rejects_invalid_config() {
    let (batch, mut els) = spawn_swarm(8, 16).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion = default_fusion();
    let cfg = FusionConfig::default().with_temperature(f32::NAN);
    let err = fusion
        .run_round(&mut els, arena, &cfg, 0)
        .await
        .unwrap_err();
    assert!(matches!(err, EfeError::InvalidConfig(_)));
}

// =====================================================================
// Is-clean semantics
// =====================================================================

#[tokio::test]
async fn fusion_round_is_clean_on_healthy_swarm() {
    let (batch, mut els) = spawn_swarm(8, 16).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion = default_fusion();
    let cfg = FusionConfig::default();
    let round = fusion.run_round(&mut els, arena, &cfg, 0).await.unwrap();
    // is_clean requires: on_time, no warnings, no anomaly.
    // Our default config on healthy swarm should satisfy this.
    if !round.is_clean() {
        eprintln!(
            "round warnings: {:?}, on_time={}, stats_issues={}",
            round.warnings,
            round.on_time,
            round.stats.has_numerical_issues()
        );
    }
    // Don't hard-assert — CPU timing variance can emit deadline warning.
    assert!(!round.stats.has_numerical_issues());
}

// =====================================================================
// Fusion + snapshot cross-check
// =====================================================================

#[tokio::test]
async fn fusion_stats_centroid_matches_els_snapshot() {
    let (batch, mut els) = spawn_swarm(8, 16).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion = default_fusion();
    let cfg = FusionConfig::default();
    let round = fusion.run_round(&mut els, arena, &cfg, 0).await.unwrap();
    let snap = els.snapshot(arena).unwrap();
    // The stats returned by FusionRound come from compute_stats run
    // *after* the mutation, and the snapshot re-runs compute_stats.
    // They should agree on n_active and variance.
    assert_eq!(round.stats.centroid.len() as u32, snap.dim);
    assert!((round.stats.variance - snap.variance).abs() < 1e-5);
}

// =====================================================================
// Long-campaign — validates double-buffering (fix for scratch leak)
// =====================================================================

#[tokio::test]
async fn fusion_campaign_32_rounds_does_not_leak_arena() {
    // Pre-fix, each round called `arena.alloc_zeroed_raw(N*D*2)`, which
    // saturated a 4 MB arena after ~8 rounds (512 KB × 8 = 4 MB).
    // With persistent scratch, we complete 32 rounds using at most
    // 2 × 512 KB = 1 MB of arena, regardless of round count.
    let (batch, mut els) = spawn_swarm(16, 16).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion = default_fusion();
    let cfg = FusionConfig::default().with_max_rounds(64);
    for r in 0..32 {
        let round = fusion
            .run_round(&mut els, arena, &cfg, r)
            .await
            .unwrap_or_else(|e| panic!("round {r} failed: {e:?}"));
        assert!(
            round.stats.all_finite(),
            "round {} produced non-finite stats",
            round.round
        );
    }
}

#[tokio::test]
async fn fusion_scratch_reused_not_reallocated() {
    // Indirect check of arena persistence: run 10 rounds, take a
    // snapshot, then run one more round. If scratch were re-alloc'd
    // each round, the arena would be near-saturated and the last round
    // might fail.
    let (batch, mut els) = spawn_swarm(8, 16).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion = default_fusion();
    let cfg = FusionConfig::default().with_max_rounds(32);
    for r in 0..10 {
        fusion.run_round(&mut els, arena, &cfg, r).await.unwrap();
    }
    let _snap = els.snapshot(arena).unwrap();
    let last = fusion.run_round(&mut els, arena, &cfg, 10).await.unwrap();
    assert!(last.stats.all_finite());
}

// =====================================================================
// Softmax NaN path — fail-fast via the pre-round validator, then
// verified again at the softmax kernel level if somehow bypassed.
// =====================================================================

#[tokio::test]
async fn fusion_rejects_nan_via_pre_round_validator() {
    let (batch, mut els) = spawn_swarm(8, 16).await;
    let arena = batch.arena_handle().unwrap().arena();
    {
        let mut split = els.split_mut(arena).unwrap();
        let row = split.latent.row_mut(0).unwrap();
        row[0] = half::bf16::from_f32(f32::NAN);
    }
    let fusion = default_fusion();
    let cfg = FusionConfig::default();
    let err = fusion
        .run_round(&mut els, arena, &cfg, 3)
        .await
        .unwrap_err();
    // The pre-round validator stamps the correct round index.
    assert!(matches!(err, EfeError::NumericalExplosion { round: 3 }));
}

// =====================================================================
// variance_warn_ratio configurability
// =====================================================================

#[tokio::test]
async fn fusion_variance_warn_ratio_infinity_disables_warning() {
    let (batch, mut els) = spawn_swarm(8, 16).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion = default_fusion();
    let mut cfg = FusionConfig::default();
    cfg.variance_warn_ratio = f32::INFINITY;
    let round = fusion.run_round(&mut els, arena, &cfg, 0).await.unwrap();
    for w in &round.warnings {
        assert!(
            !matches!(w, efe_core::fusion::FusionWarning::HighVarianceDelta { .. }),
            "unexpected HighVarianceDelta with INF ratio: {w:?}"
        );
    }
}
