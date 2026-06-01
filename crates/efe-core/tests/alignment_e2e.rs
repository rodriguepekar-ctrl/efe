#![allow(clippy::field_reassign_with_default)]
//! End-to-end integration tests for M9 — Alignment Registry.
//!
//! Scenarios exercised :
//!
//! 1. `register_get_roundtrip` — bit-exact round trip via the trait.
//! 2. `evolve_chain` — three sequential evolutions produce v1..v4, all
//!    linked via `parent` metadata.
//! 3. `cross_tenant_isolation` — same task name in two tenants coexist.
//! 4. `breach_flow` — register + 10 breach reports + ordered history.
//! 5. `policy_violation_propagates` — register with too many anchors.
//! 6. `builder_from_registry_round_trip` — spec identical to inline.
//! 7. `evolve_union_with_missing_other` — explicit error.
//! 8. `delete_last_without_force_rejected`.
//! 9. `delete_with_force_removes_task`.
//! 10. `event_channel_receives_registered_evolved`.
//!
//! These tests exercise the public surface and the interaction between
//! multiple sub-modules, serving as regression guardrails for any future
//! refactor.

use std::sync::Arc;

use efe_core::alignment::{
    blend_cones, intersect_cones, union_cones, AlignmentRegistry, BreachContext, BreachFilter,
    BreachReport, CanonicalCone, ConeEvolutionOp, ConeKey, ConeMetadata, InMemoryRegistry,
    RegistryConfig, RegistryEvent,
};
use efe_core::types::{AlignmentCone, LatentDim, PopulationMix, SwarmId, SwarmSpec, TemplateCode};

fn cone(k: usize, d: usize, r: f32) -> AlignmentCone {
    let anchors: Vec<Vec<f32>> = (0..k)
        .map(|i| {
            let mut v = vec![0.0; d];
            v[i % d] = 1.0;
            v
        })
        .collect();
    AlignmentCone::new(anchors, r).unwrap()
}

fn canonical(k: usize, d: usize) -> CanonicalCone {
    CanonicalCone::new(cone(k, d, 0.5), "tester", "integration").unwrap()
}

fn registry() -> Arc<InMemoryRegistry> {
    Arc::new(InMemoryRegistry::with_default_config().unwrap())
}

// -------------------------------------------------------------------
// 1. register + get
// -------------------------------------------------------------------
#[tokio::test]
async fn register_get_roundtrip() {
    let reg = registry();
    let key = ConeKey::new("acme", "qa", 1).unwrap();
    let canon = canonical(3, 8);
    let h = reg.register(key.clone(), canon.clone()).await.unwrap();
    assert_eq!(h.key, key);

    let got = reg.get(&key).await.unwrap();
    assert_eq!(got.key, key);
    assert_eq!(got.cone.num_anchors(), canon.num_anchors());
    assert_eq!(got.cone.dim(), canon.dim());
    assert!((got.cone.radius() - canon.radius()).abs() < f32::EPSILON);
}

// -------------------------------------------------------------------
// 2. evolve_chain
// -------------------------------------------------------------------
#[tokio::test]
async fn evolve_chain() {
    let reg = registry();
    let k1 = ConeKey::new("acme", "qa", 1).unwrap();
    reg.register(k1.clone(), canonical(3, 8)).await.unwrap();

    // v2: append.
    let h2 = reg
        .evolve(
            k1.clone(),
            ConeEvolutionOp::AppendAnchor {
                anchor: {
                    let mut v = vec![0.0; 8];
                    v[3] = 1.0;
                    v
                },
            },
            None,
        )
        .await
        .unwrap();
    assert_eq!(h2.key.version, 2);
    assert_eq!(h2.cone.num_anchors(), 4);
    assert_eq!(h2.cone.metadata.parent.as_ref().unwrap().version, 1);

    // v3: replace idx 0.
    let h3 = reg
        .evolve(
            h2.key.clone(),
            ConeEvolutionOp::ReplaceAnchor {
                idx: 0,
                anchor: {
                    let mut v = vec![0.0; 8];
                    v[4] = 1.0;
                    v
                },
            },
            None,
        )
        .await
        .unwrap();
    assert_eq!(h3.key.version, 3);
    assert_eq!(h3.cone.metadata.parent.as_ref().unwrap().version, 2);

    // v4: update radius.
    let h4 = reg
        .evolve(
            h3.key.clone(),
            ConeEvolutionOp::UpdateRadius { new_radius: 1.25 },
            None,
        )
        .await
        .unwrap();
    assert_eq!(h4.key.version, 4);
    assert!((h4.cone.radius() - 1.25).abs() < f32::EPSILON);

    let versions = reg.list_versions("acme", "qa").await.unwrap();
    let nums: Vec<u32> = versions.iter().map(|k| k.version).collect();
    assert_eq!(nums, vec![1, 2, 3, 4]);
}

// -------------------------------------------------------------------
// 3. cross_tenant_isolation
// -------------------------------------------------------------------
#[tokio::test]
async fn cross_tenant_isolation() {
    let reg = registry();
    let a = ConeKey::new("acme", "task", 1).unwrap();
    let b = ConeKey::new("other", "task", 1).unwrap();
    reg.register(a.clone(), canonical(2, 4)).await.unwrap();
    reg.register(b.clone(), canonical(2, 4)).await.unwrap();
    assert!(reg.get(&a).await.is_ok());
    assert!(reg.get(&b).await.is_ok());
    let va = reg.list_versions("acme", "task").await.unwrap();
    let vb = reg.list_versions("other", "task").await.unwrap();
    assert_eq!(va.len(), 1);
    assert_eq!(vb.len(), 1);
    assert_ne!(va[0].tenant, vb[0].tenant);
}

// -------------------------------------------------------------------
// 4. breach_flow
// -------------------------------------------------------------------
#[tokio::test]
async fn breach_flow() {
    let reg = registry();
    let key = ConeKey::new("acme", "task", 1).unwrap();
    reg.register(key.clone(), canonical(2, 4)).await.unwrap();

    for i in 0..10u64 {
        let r = BreachReport::new(
            key.clone(),
            SwarmId::new(),
            0.6 + (i as f32) * 0.01,
            0.5,
            i + 1,
            BreachContext::Dissolution,
        );
        reg.report_breach(r).await.unwrap();
    }

    let hist = reg.breach_history(10, None).await.unwrap();
    assert_eq!(hist.len(), 10);
    // Reverse chronological.
    assert_eq!(hist[0].timestamp_nanos, 10);
    assert_eq!(hist[9].timestamp_nanos, 1);
}

#[tokio::test]
async fn breach_history_with_filter_by_magnitude() {
    let reg = registry();
    let key = ConeKey::new("t", "a", 1).unwrap();
    reg.register(key.clone(), canonical(2, 4)).await.unwrap();
    for i in 0..5u64 {
        let r = BreachReport::new(
            key.clone(),
            SwarmId::new(),
            0.5 + 0.1 * (i as f32 + 1.0),
            0.5,
            i + 1,
            BreachContext::Dissolution,
        );
        reg.report_breach(r).await.unwrap();
    }
    // Keep only magnitude >= 0.3 : observed_delta in {0.8, 0.9, 1.0} → 3 matches.
    let filter = BreachFilter::none().with_min_magnitude(0.3);
    let hist = reg.breach_history(10, Some(filter)).await.unwrap();
    assert_eq!(hist.len(), 3);
}

// -------------------------------------------------------------------
// 5. policy_violation_propagates
// -------------------------------------------------------------------
#[tokio::test]
async fn policy_violation_propagates() {
    let mut cfg = RegistryConfig::default();
    cfg.policy.max_anchors = 2;
    let reg = InMemoryRegistry::new(cfg).unwrap();
    let key = ConeKey::new("acme", "task", 1).unwrap();
    let canon = canonical(3, 4); // K=3 > max_anchors=2.
    let err = reg.register(key, canon).await.unwrap_err();
    assert!(
        err.to_string().contains("I1"),
        "expected I1 policy rejection, got: {err}"
    );
}

// -------------------------------------------------------------------
// 6. builder_from_registry_round_trip
// -------------------------------------------------------------------
#[tokio::test]
async fn builder_from_registry_round_trip() {
    let reg = registry();
    let key = ConeKey::new("acme", "task", 1).unwrap();
    reg.register(key.clone(), canonical(2, 4)).await.unwrap();

    let spec = SwarmSpec::builder()
        .problem_embedding(vec![0.0; 4])
        .dim_latent(LatentDim::new(4).unwrap())
        .population(PopulationMix::new(vec![(TemplateCode::NaSem, 2)]).unwrap())
        .max_rounds(4)
        .unwrap()
        .from_registry(&*reg, &key)
        .await
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(spec.alignment_cone.anchors.len(), 2);
    assert_eq!(spec.dim_latent.get(), 4);
    spec.validate().unwrap();
}

// -------------------------------------------------------------------
// 7. evolve_union_with_missing_other
// -------------------------------------------------------------------
#[tokio::test]
async fn evolve_union_with_missing_other() {
    let reg = registry();
    let k1 = ConeKey::new("t", "a", 1).unwrap();
    let k2 = ConeKey::new("t", "b", 1).unwrap();
    reg.register(k1.clone(), canonical(2, 4)).await.unwrap();
    let res = reg
        .evolve(k1, ConeEvolutionOp::UnionWith { other: k2 }, None)
        .await;
    assert!(res.is_err());
}

// -------------------------------------------------------------------
// 8. delete_last_without_force_rejected
// -------------------------------------------------------------------
#[tokio::test]
async fn delete_last_without_force_rejected() {
    let reg = registry();
    let k = ConeKey::new("t", "a", 1).unwrap();
    reg.register(k.clone(), canonical(2, 4)).await.unwrap();
    assert!(reg.delete(&k, false).await.is_err());
    // Still present.
    assert!(reg.get(&k).await.is_ok());
}

// -------------------------------------------------------------------
// 9. delete_with_force_removes_task
// -------------------------------------------------------------------
#[tokio::test]
async fn delete_with_force_removes_task() {
    let reg = registry();
    let k = ConeKey::new("t", "a", 1).unwrap();
    reg.register(k.clone(), canonical(2, 4)).await.unwrap();
    reg.delete(&k, true).await.unwrap();
    assert!(reg.get(&k).await.is_err());
    assert!(reg.list_versions("t", "a").await.unwrap().is_empty());
}

// -------------------------------------------------------------------
// 10. event_channel_receives_registered_evolved
// -------------------------------------------------------------------
#[tokio::test]
async fn event_channel_receives_registered_evolved() {
    let reg = registry();
    let mut rx = reg.take_events_receiver().await.unwrap();
    let k = ConeKey::new("t", "a", 1).unwrap();
    reg.register(k.clone(), canonical(2, 4)).await.unwrap();
    reg.evolve(k, ConeEvolutionOp::UpdateRadius { new_radius: 0.7 }, None)
        .await
        .unwrap();

    // Registered (v1), Registered (v2 from evolve), Evolved.
    let e1 = rx.recv().await.unwrap();
    let e2 = rx.recv().await.unwrap();
    let e3 = rx.recv().await.unwrap();
    assert!(matches!(e1, RegistryEvent::Registered { .. }));
    assert!(matches!(e2, RegistryEvent::Registered { .. }));
    match e3 {
        RegistryEvent::Evolved { op_kind, .. } => {
            assert_eq!(op_kind, "update_radius");
        }
        other => panic!("expected Evolved, got {other:?}"),
    }
}

// -------------------------------------------------------------------
// Composition cross-module
// -------------------------------------------------------------------
#[tokio::test]
async fn composition_operators_round_trip_registry() {
    let reg = registry();
    let k1 = ConeKey::new("t", "a", 1).unwrap();
    let k2 = ConeKey::new("t", "b", 1).unwrap();
    reg.register(k1.clone(), canonical(2, 4)).await.unwrap();
    reg.register(k2.clone(), canonical(2, 4)).await.unwrap();

    let h1 = reg.get(&k1).await.unwrap();
    let h2 = reg.get(&k2).await.unwrap();

    let u = union_cones(h1.as_cone(), h2.as_cone()).unwrap();
    assert_eq!(u.anchors.len(), 4);

    let b = blend_cones(h1.as_cone(), h2.as_cone(), 0.5).unwrap();
    assert_eq!(b.anchors.len(), 2);

    let i = intersect_cones(h1.as_cone(), h2.as_cone(), 0.5).unwrap();
    assert!(!i.anchors.is_empty());
}

// -------------------------------------------------------------------
// Metadata override on evolve
// -------------------------------------------------------------------
#[tokio::test]
async fn evolve_respects_metadata_override() {
    let reg = registry();
    let k = ConeKey::new("t", "a", 1).unwrap();
    reg.register(k.clone(), canonical(2, 4)).await.unwrap();
    let md = ConeMetadata::new("alice", "custom evolution").unwrap();
    let h = reg
        .evolve(
            k,
            ConeEvolutionOp::UpdateRadius { new_radius: 0.9 },
            Some(md),
        )
        .await
        .unwrap();
    assert_eq!(h.cone.metadata.created_by, "alice");
    assert_eq!(h.cone.metadata.description, "custom evolution");
    // Parent still set by the registry despite override.
    assert!(h.cone.metadata.parent.is_some());
}

// -------------------------------------------------------------------
// Regression : existing SwarmSpec.alignment_cone inline still works
// (M9 non-breaking).
// -------------------------------------------------------------------
#[tokio::test]
async fn inline_alignment_cone_still_works_without_registry() {
    let c = cone(2, 4, 0.5);
    let spec = SwarmSpec::builder()
        .problem_embedding(vec![0.0; 4])
        .dim_latent(LatentDim::new(4).unwrap())
        .population(PopulationMix::new(vec![(TemplateCode::NaSem, 2)]).unwrap())
        .max_rounds(4)
        .unwrap()
        .alignment_cone(c)
        .build()
        .unwrap();
    spec.validate().unwrap();
}
