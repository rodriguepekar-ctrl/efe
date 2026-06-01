//! End-to-end integration tests for M4 ELS.
//!
//! These tests wire M3 (genesis) + M4 (ELS) + M2 (memory) and verify
//! that the pipeline `spawn → compute_stats → project_onto_anchors →
//! snapshot` is sound and deterministic.

use std::sync::Arc;

use efe_core::els::{
    compute_stats, cosine_similarity, l2_distance, project_onto_anchors,
    project_vector_onto_anchors, ElsSnapshot, EpistemicLatentSpace,
};
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

async fn spawn_swarm(dim: u32) -> (NanoAgentBatch, EpistemicLatentSpace) {
    let (pool, streams, registry) = make_infra(dim);
    let engine = GenesisEngine::builder()
        .build_mvp(pool, registry, streams)
        .unwrap();
    let spec = SwarmSpecBuilder::default()
        .problem_embedding(vec![0.05f32; dim as usize])
        .dim_latent(LatentDim::new(dim).unwrap())
        .population(PopulationMix::new(vec![(TemplateCode::NaSem, 128)]).unwrap())
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

// =====================================================================
// Views
// =====================================================================

#[tokio::test]
async fn latent_view_reflects_spawned_population() {
    let (batch, els) = spawn_swarm(16).await;
    let arena = batch.arena_handle().unwrap().arena();

    let latent = els.latent_view(arena).unwrap().expect("populated");
    assert_eq!(latent.n_agents(), batch.count());
    assert_eq!(latent.dim(), 16);

    let row0 = latent.row(0).unwrap();
    assert_eq!(row0.len(), 16);
}

#[tokio::test]
async fn mask_view_marks_all_agents_active_after_spawn() {
    let (batch, els) = spawn_swarm(8).await;
    let arena = batch.arena_handle().unwrap().arena();
    let mask = els.mask_view(arena).unwrap().expect("populated");
    assert_eq!(mask.active_count(), batch.count());
}

#[tokio::test]
async fn anchors_view_returns_uploaded_cone() {
    let (batch, els) = spawn_swarm(8).await;
    let arena = batch.arena_handle().unwrap().arena();
    let anchors = els.anchors_view(arena).unwrap().expect("populated");
    assert_eq!(anchors.n_anchors(), 2);
    assert_eq!(anchors.dim(), 8);
    let first = anchors.get(0).unwrap();
    assert!((first[0] - 0.1).abs() < 1e-5);
}

#[tokio::test]
async fn problem_embedding_view_matches_spec() {
    let (batch, els) = spawn_swarm(12).await;
    let arena = batch.arena_handle().unwrap().arena();
    let emb = els
        .problem_embedding_view(arena)
        .unwrap()
        .expect("populated");
    assert_eq!(emb.dim(), 12);
    assert!((emb.as_slice()[0] - 0.05).abs() < 1e-5);
}

// =====================================================================
// Split-mut
// =====================================================================

#[tokio::test]
async fn split_mut_permits_concurrent_read_write() {
    let (batch, mut els) = spawn_swarm(8).await;
    let arena = batch.arena_handle().unwrap().arena();

    let mut views = els.split_mut(arena).unwrap();
    // Simultaneous: read anchors while mutating latent.
    let anchors = views.anchors.get(0).unwrap();
    assert_eq!(anchors.len(), 8);
    let row = views.latent.row_mut(0).unwrap();
    row[0] = half::bf16::from_f32(42.0);
    // Mask read still works.
    assert!(views.mask.is_active(0));
}

// =====================================================================
// Stats
// =====================================================================

#[tokio::test]
async fn compute_stats_returns_plausible_values() {
    let (batch, els) = spawn_swarm(32).await;
    let arena = batch.arena_handle().unwrap().arena();
    let stats = compute_stats(&els, arena).unwrap();
    assert_eq!(stats.n_active, batch.count());
    assert!(stats.centroid.len() == 32);
    assert!(stats.variance.is_finite());
    assert!(!stats.has_anomaly());
}

#[tokio::test]
async fn into_fusion_stats_produces_consumable_struct() {
    let (batch, els) = spawn_swarm(16).await;
    let arena = batch.arena_handle().unwrap().arena();
    let stats = compute_stats(&els, arena).unwrap();
    let fs = stats.clone().into_fusion_stats(7);
    assert_eq!(fs.round, 7);
    assert!((fs.variance - stats.variance).abs() < 1e-6);
    assert_eq!(fs.centroid.len(), stats.centroid.len());
    assert!(!fs.has_numerical_issues());
}

// =====================================================================
// Projection
// =====================================================================

#[tokio::test]
async fn project_onto_els_anchors_returns_valid_output() {
    let (batch, els) = spawn_swarm(8).await;
    let arena = batch.arena_handle().unwrap().arena();
    let stats = compute_stats(&els, arena).unwrap();
    let (projected, delta) = project_onto_anchors(&els, arena, &stats.centroid, 0.3).unwrap();
    assert_eq!(projected.len(), 8);
    assert!(delta.is_finite());
    assert!(delta >= 0.0);
}

#[tokio::test]
async fn project_is_consistent_with_pure_variant() {
    let (batch, els) = spawn_swarm(8).await;
    let arena = batch.arena_handle().unwrap().arena();

    let v = vec![0.15f32; 8];
    let anchors_host = vec![vec![0.1f32; 8], vec![0.2f32; 8]];
    let strength = 0.4;

    let (a, da) = project_onto_anchors(&els, arena, &v, strength).unwrap();
    let (b, db) = project_vector_onto_anchors(&v, &anchors_host, strength).unwrap();

    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(b.iter()) {
        assert!((x - y).abs() < 1e-4, "projection mismatch: x={x}, y={y}");
    }
    assert!((da - db).abs() < 1e-5);
}

// =====================================================================
// Snapshot
// =====================================================================

#[tokio::test]
async fn snapshot_hash_is_deterministic_for_same_swarm() {
    let (batch, els) = spawn_swarm(16).await;
    let arena = batch.arena_handle().unwrap().arena();
    let snap1 = els.snapshot(arena).unwrap();
    let snap2 = els.snapshot(arena).unwrap();
    assert_eq!(snap1.centroid_hash, snap2.centroid_hash);
    assert_eq!(snap1, snap2);
}

#[tokio::test]
async fn snapshot_serializes_to_json() {
    let (batch, els) = spawn_swarm(8).await;
    let arena = batch.arena_handle().unwrap().arena();
    let snap = els.snapshot(arena).unwrap();
    let json = serde_json::to_string(&snap).unwrap();
    let back: ElsSnapshot = serde_json::from_str(&json).unwrap();
    assert_eq!(snap, back);
}

// =====================================================================
// Determinism check across two separate swarms with same seed
// =====================================================================

#[tokio::test]
async fn same_spec_produces_same_snapshot_hash() {
    let dim = 16;
    let (pool, streams, registry) = make_infra(dim);
    let engine = GenesisEngine::builder()
        .build_mvp(pool, registry, streams)
        .unwrap();
    let spec = SwarmSpecBuilder::default()
        .problem_embedding(vec![0.12f32; dim as usize])
        .dim_latent(LatentDim::new(dim).unwrap())
        .population(PopulationMix::new(vec![(TemplateCode::NaSem, 64)]).unwrap())
        .alignment_cone(AlignmentCone::new(vec![vec![0.2f32; dim as usize]], 0.15).unwrap())
        .build()
        .unwrap();
    let (b1, e1) = engine.spawn(&spec).await.unwrap();
    let (b2, e2) = engine.spawn(&spec).await.unwrap();
    let s1 = e1.snapshot(b1.arena_handle().unwrap().arena()).unwrap();
    let s2 = e2.snapshot(b2.arena_handle().unwrap().arena()).unwrap();
    assert_eq!(s1.centroid_hash, s2.centroid_hash);
}

// =====================================================================
// Metrics integration
// =====================================================================

#[tokio::test]
async fn cosine_between_centroid_and_problem_embedding_is_finite() {
    let (batch, els) = spawn_swarm(16).await;
    let arena = batch.arena_handle().unwrap().arena();
    let stats = compute_stats(&els, arena).unwrap();
    let emb = els.problem_embedding_view(arena).unwrap().unwrap();
    let cos = cosine_similarity(&stats.centroid, emb.as_slice());
    assert!(cos.is_finite());
    assert!(cos.abs() <= 1.0 + 1e-5);
}

#[tokio::test]
async fn l2_distance_between_rows_well_defined() {
    let (batch, els) = spawn_swarm(8).await;
    let arena = batch.arena_handle().unwrap().arena();
    let latent = els.latent_view(arena).unwrap().unwrap();
    let row0: Vec<f32> = latent.row(0).unwrap().iter().map(|v| v.to_f32()).collect();
    let row1: Vec<f32> = latent.row(1).unwrap().iter().map(|v| v.to_f32()).collect();
    let d = l2_distance(&row0, &row1);
    assert!(d.is_finite());
    assert!(d >= 0.0);
}

// =====================================================================
// Cross-module consistency
// =====================================================================

#[tokio::test]
async fn snapshot_and_compute_stats_agree_on_scalars() {
    let (batch, els) = spawn_swarm(16).await;
    let arena = batch.arena_handle().unwrap().arena();
    let stats = compute_stats(&els, arena).unwrap();
    let snap = els.snapshot(arena).unwrap();
    assert_eq!(snap.n_active, stats.n_active);
    assert!((snap.variance - stats.variance).abs() < f32::EPSILON);
    assert!((snap.gradient_norm - stats.gradient_norm).abs() < f32::EPSILON);
    assert_eq!(snap.has_anomaly, stats.has_anomaly());
}

#[tokio::test]
async fn snapshot_from_precomputed_stats_matches_ondemand() {
    let (batch, els) = spawn_swarm(16).await;
    let arena = batch.arena_handle().unwrap().arena();
    let stats = compute_stats(&els, arena).unwrap();
    let snap_direct = els.snapshot(arena).unwrap();
    let snap_from_stats = ElsSnapshot::from_stats(&els, &stats);
    assert_eq!(snap_direct, snap_from_stats);
}

#[tokio::test]
async fn stats_all_finite_on_real_spawn() {
    let (batch, els) = spawn_swarm(32).await;
    let arena = batch.arena_handle().unwrap().arena();
    let stats = compute_stats(&els, arena).unwrap();
    assert!(stats.all_finite());
    assert!(!stats.has_anomaly());
}

#[tokio::test]
async fn anchors_view_to_host_matches_spec_cone() {
    let (batch, els) = spawn_swarm(8).await;
    let arena = batch.arena_handle().unwrap().arena();
    let anchors = els.anchors_view(arena).unwrap().unwrap();
    let host = anchors.to_host();
    assert_eq!(host.len(), 2);
    assert!((host[0][0] - 0.1).abs() < 1e-5);
    assert!((host[1][0] - 0.2).abs() < 1e-5);
}

#[tokio::test]
async fn projection_pure_variant_matches_anchors_view_variant() {
    let (batch, els) = spawn_swarm(8).await;
    let arena = batch.arena_handle().unwrap().arena();
    let anchors = els.anchors_view(arena).unwrap().unwrap();
    let host = anchors.to_host();
    let v = vec![0.12f32; 8];
    let (a, _) = project_onto_anchors(&els, arena, &v, 0.3).unwrap();
    let (b, _) = project_vector_onto_anchors(&v, &host, 0.3).unwrap();
    for (x, y) in a.iter().zip(b.iter()) {
        assert!((x - y).abs() < 1e-5);
    }
}
