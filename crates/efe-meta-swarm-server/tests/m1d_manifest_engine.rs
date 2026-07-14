// SPDX-License-Identifier: Apache-2.0
//! **Couche 3 · M1.d — the efe-core manifest engine block, default build.**
//!
//! Proves the fail-closed contract WITHOUT the `core-integration` feature :
//!
//! * new-schema manifests parse (and OLD mock manifests keep parsing) ;
//! * `validate_manifest` enforces the engine presence rules both ways ;
//! * reload compatibility rejects `controller` block drift ;
//! * a default-built binary REFUSES to construct an efe-core row (never a
//!   silent mock) ;
//! * the wire descriptor stays mock-honest (`config_hash` absent).

use efe_meta_swarm_server::manifest::{
    validate_manifest, validate_reload_compatibility, AlignmentConeManifest, ControllerManifest,
    H1CanaryManifest, IterationManifest, Manifest, MetaSpaceManifest, ParticipantManifest,
    PopulationManifest, SafetyBoxManifest, SafetyManifest,
};

const DIM: u32 = 16;

fn engine_block() -> ControllerManifest {
    ControllerManifest {
        schema: 1,
        population: vec![PopulationManifest { template: "na-sem".into(), count: 4 }],
        max_rounds: 3,
        alignment_cone: AlignmentConeManifest {
            anchors: vec![vec![0.1; DIM as usize]],
            max_projection_distance: 0.5,
        },
        safety_box: SafetyBoxManifest { lo: 0.05, hi: 0.30 },
        tenant_id: "tenant-m1d".into(),
        h1_canary: H1CanaryManifest::default(),
    }
}

fn engine_row(id: &str) -> ParticipantManifest {
    ParticipantManifest {
        id: id.into(),
        dim: DIM,
        kappa: None,
        seed: None,
        blas_backend: "none".into(),
        controller: Some(engine_block()),
    }
}

fn mock_row(id: &str) -> ParticipantManifest {
    ParticipantManifest {
        id: id.into(),
        dim: 8,
        kappa: Some(0.6),
        seed: Some(11),
        blas_backend: "none".into(),
        controller: None,
    }
}

fn manifest_with(rows: Vec<ParticipantManifest>) -> Manifest {
    Manifest {
        version: "v0.2.0".into(),
        meta: MetaSpaceManifest {
            dim: 16,
            safety: SafetyManifest::Box { radius: 50.0 },
        },
        participants: rows,
        iteration: IterationManifest::default(),
    }
}

// ─────────────────────────────────────────────────────────────────────
// Parsing — backward AND forward compatible.
// ─────────────────────────────────────────────────────────────────────

/// An OLD v0.2.0 mock manifest (kappa/seed present, no controller block)
/// still parses and validates — byte-identical backward compat.
#[test]
fn old_mock_yaml_still_parses_and_validates() {
    let yaml = r#"
version: v0.2.0
meta:
  dim: 16
  safety:
    kind: box
    radius: 10.0
participants:
  - id: supplier-scoring
    dim: 8
    kappa: 0.6
    seed: 11
"#;
    let m: Manifest = serde_yaml::from_str(yaml).expect("old YAML parses");
    validate_manifest(&m).expect("old YAML validates");
    assert_eq!(m.participants[0].kappa, Some(0.6));
    assert!(m.participants[0].controller.is_none());
}

/// A NEW efe-core row parses from YAML (kappa/seed omitted, controller block
/// present) and validates.
#[test]
fn engine_yaml_parses_and_validates() {
    let yaml = r#"
version: v0.2.0
meta:
  dim: 16
  safety:
    kind: box
    radius: 10.0
participants:
  - id: core-participant
    dim: 16
    controller:
      schema: 1
      population:
        - template: na-sem
          count: 4
      max_rounds: 3
      alignment_cone:
        anchors:
          - [0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1]
        max_projection_distance: 0.5
      safety_box:
        lo: 0.05
        hi: 0.30
      tenant_id: tenant-m1d
"#;
    let m: Manifest = serde_yaml::from_str(yaml).expect("engine YAML parses");
    validate_manifest(&m).expect("engine YAML validates");
    let p = &m.participants[0];
    assert!(p.kappa.is_none() && p.seed.is_none(), "engine rows omit kappa/seed");
    let c = p.controller.as_ref().expect("controller block present");
    assert_eq!(c.schema, 1);
    assert_eq!(c.h1_canary.mode, "boot", "canary defaults to boot (fail-closed honest)");
}

/// The fail-closed-against-old-binaries property : an efe-core row OMITS
/// kappa/seed, and a mock row REQUIRES them — so the two shapes can never be
/// silently confused (validate enforces the presence rules both ways).
#[test]
fn validate_enforces_engine_presence_rules_both_ways() {
    // Mock row missing kappa → rejected.
    let mut row = mock_row("p");
    row.kappa = None;
    let err = validate_manifest(&manifest_with(vec![row])).unwrap_err();
    assert!(format!("{err}").contains("kappa is required"), "{err}");

    // Mock row missing seed → rejected.
    let mut row = mock_row("p");
    row.seed = None;
    let err = validate_manifest(&manifest_with(vec![row])).unwrap_err();
    assert!(format!("{err}").contains("seed is required"), "{err}");

    // Engine row WITH kappa → rejected (mock-only concept, deprecation).
    let mut row = engine_row("p");
    row.kappa = Some(0.5);
    let err = validate_manifest(&manifest_with(vec![row])).unwrap_err();
    assert!(format!("{err}").contains("mock-only"), "{err}");

    // Engine row WITH seed → rejected.
    let mut row = engine_row("p");
    row.seed = Some(7);
    let err = validate_manifest(&manifest_with(vec![row])).unwrap_err();
    assert!(format!("{err}").contains("mock-only"), "{err}");

    // A clean engine row → accepted.
    validate_manifest(&manifest_with(vec![engine_row("p")])).expect("clean engine row validates");
}

/// Structural fail-closed checks on the controller block itself.
#[test]
fn validate_rejects_malformed_engine_blocks() {
    // Unknown schema version.
    let mut row = engine_row("p");
    row.controller.as_mut().unwrap().schema = 2;
    let err = validate_manifest(&manifest_with(vec![row])).unwrap_err();
    assert!(format!("{err}").contains("schema"), "{err}");

    // Empty population.
    let mut row = engine_row("p");
    row.controller.as_mut().unwrap().population.clear();
    let err = validate_manifest(&manifest_with(vec![row])).unwrap_err();
    assert!(format!("{err}").contains("population"), "{err}");

    // Anchor length ≠ dim.
    let mut row = engine_row("p");
    row.controller.as_mut().unwrap().alignment_cone.anchors = vec![vec![0.1; 4]];
    let err = validate_manifest(&manifest_with(vec![row])).unwrap_err();
    assert!(format!("{err}").contains("anchors"), "{err}");

    // Degenerate safety box (lo ≥ hi).
    let mut row = engine_row("p");
    row.controller.as_mut().unwrap().safety_box = SafetyBoxManifest { lo: 0.5, hi: 0.5 };
    let err = validate_manifest(&manifest_with(vec![row])).unwrap_err();
    assert!(format!("{err}").contains("safety_box"), "{err}");

    // Unknown canary mode.
    let mut row = engine_row("p");
    row.controller.as_mut().unwrap().h1_canary.mode = "sometimes".into();
    let err = validate_manifest(&manifest_with(vec![row])).unwrap_err();
    assert!(format!("{err}").contains("h1_canary.mode"), "{err}");

    // Canary probe too small to score a single pair.
    let mut row = engine_row("p");
    row.controller.as_mut().unwrap().h1_canary.n_seeds = 1;
    let err = validate_manifest(&manifest_with(vec![row])).unwrap_err();
    assert!(format!("{err}").contains("n_seeds"), "{err}");
}

// ─────────────────────────────────────────────────────────────────────
// Reload — the engine block is immutable across SIGHUP.
// ─────────────────────────────────────────────────────────────────────

/// Editing the controller block across reload is rejected ("controller
/// drift") — the running engine and its boot-time H1 certificate were built
/// from the old block.
#[test]
fn reload_rejects_controller_block_drift() {
    let current = manifest_with(vec![engine_row("p")]);
    let mut new = current.clone();
    new.participants[0].controller.as_mut().unwrap().max_rounds = 5;
    let err = validate_reload_compatibility(&current, &new).unwrap_err();
    assert!(format!("{err}").contains("controller drift"), "{err}");
}

/// Engine PRESENCE drift (mock → efe-core or back) is likewise rejected.
#[test]
fn reload_rejects_engine_presence_drift() {
    // mock → efe-core : the mock row carries kappa/seed, the engine row does
    // not — the kappa presence drift triggers first (engine change).
    let current = manifest_with(vec![mock_row("p")]);
    let mut new = current.clone();
    new.participants[0] = ParticipantManifest { dim: 8, ..engine_row("p") };
    let err = validate_reload_compatibility(&current, &new).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("engine change") || msg.contains("controller drift"),
        "{msg}",
    );

    // Identical engine manifests reload fine (the no-op SIGHUP).
    let m = manifest_with(vec![engine_row("p")]);
    validate_reload_compatibility(&m, &m).expect("identical engine manifest reloads");
}

// ─────────────────────────────────────────────────────────────────────
// Construction — fail-closed without the feature ; honest descriptor.
// ─────────────────────────────────────────────────────────────────────

/// THE fail-closed guarantee of the default build : an efe-core row on a
/// binary built WITHOUT `core-integration` refuses construction — it must
/// never silently serve the κ·x mock in place of a real engine.
#[cfg(not(feature = "core-integration"))]
#[test]
fn default_build_refuses_engine_rows_fail_closed() {
    use efe_meta_swarm_server::participant::LocalParticipant;
    let err = LocalParticipant::from_manifest_row(&engine_row("p")).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("core-integration"),
        "the error must name the missing feature, got : {msg}",
    );
}

/// A mock participant's wire descriptor carries NO config_hash and NO engine
/// token — the honest-wire change is invisible to legacy deployments.
#[test]
fn mock_descriptor_has_no_engine_metadata() {
    use efe_meta_swarm_server::config::participant::ParticipantConfig;
    use efe_meta_swarm_server::participant::ParticipantState;
    let config = ParticipantConfig {
        id: "mock-p".into(),
        grpc_bind: "127.0.0.1:0".parse().unwrap(),
        metrics_bind: "127.0.0.1:0".parse().unwrap(),
        worker_threads: 1,
        tls_required: false,
        tls_cert_path: None,
        tls_key_path: None,
        tls_ca_path: None,
        tls_mutual: false,
        tls_min_protocol: efe_meta_swarm_server::tls::ProtocolVersion::Tls13,
        tls_allowed_peer_sans: Vec::new(),
        tls_peer_san: None,
    };
    let state = ParticipantState::new_for_tests(config);
    let d = state.descriptor();
    assert!(d.config_hash.is_none(), "mock rows must not advertise a config_hash");
    assert!(
        !d.features.iter().any(|f| f.starts_with("engine:")),
        "mock rows must not advertise an engine token, got {:?}",
        d.features,
    );
    assert!((d.kappa - 0.6).abs() < 1e-6, "mock kappa still on the wire");
    assert_eq!(d.seed, 11, "mock seed still on the wire");
}
