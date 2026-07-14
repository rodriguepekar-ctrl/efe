//! YAML manifest reader + validator.
//!
//! PR #6 shipped the parser only.  PR #8 adds [`validate_manifest`]
//! which checks dimensions, kappa range, duplicate ids, and safety
//! geometry consistency.  PR #12 adds [`validate_reload_compatibility`]
//! which compares a candidate new manifest against the running one
//! and rejects changes that would invalidate in-flight participant
//! state (cf. `EFE_A1_PHASE2_PR12_ARCHITECTURE.md` AD3-AD5 / § 16).

mod schema;

pub use schema::{
    AlignmentConeManifest, ControllerManifest, H1CanaryManifest, IterationManifest, Manifest,
    MetaSpaceManifest, ParticipantManifest, PopulationManifest, SafetyBoxManifest, SafetyManifest,
};

use std::collections::HashSet;
use std::path::Path;

use crate::error::ServerError;

/// Floating-point tolerance used to detect κ drift across reload (PR #12).
///
/// Participant κ is shipped as `f32` and round-trips through YAML serde
/// plus Phase-1's `MockParticipant`.  A naïve byte-identical comparison
/// would reject `0.6` parsed twice from the same YAML.  `1.0e-6` is
/// generous : Phase-1's contraction-correctness margin is the `(1 − κ)`
/// gap, well above this tolerance for any production κ.
pub const RELOAD_KAPPA_TOLERANCE: f32 = 1.0e-6;

/// Read + parse a manifest from disk.
pub fn load(path: impl AsRef<Path>) -> Result<Manifest, ServerError> {
    let p = path.as_ref();
    let raw = std::fs::read_to_string(p).map_err(ServerError::Io)?;
    let m: Manifest = serde_yaml::from_str(&raw)
        .map_err(|e| ServerError::Manifest(format!("manifest {} : {e}", p.display())))?;
    Ok(m)
}

/// Load + validate.  Convenience wrapper used at controller startup.
pub fn load_and_validate(path: impl AsRef<Path>) -> Result<Manifest, ServerError> {
    let m = load(path)?;
    validate_manifest(&m)?;
    Ok(m)
}

/// Find the participant row matching `id`.  Returns `None` if the
/// manifest does not declare a participant with that id.
///
/// Used by the participant binary at boot to discover its own
/// `dim` / `kappa` / `seed`.
#[must_use]
pub fn find_participant<'a>(m: &'a Manifest, id: &str) -> Option<&'a ParticipantManifest> {
    m.participants.iter().find(|p| p.id == id)
}

/// Validate a parsed manifest against Phase-2 semantic rules.
///
/// Rules :
///   1. `meta.dim > 0`.
///   2. At least one participant.
///   3. Participant ids are unique.
///   4. Each participant has `dim > 0`.
///   5. Each participant's `kappa ∈ (0, 1)` (contractive).
///   6. Safety geometry is internally consistent (positive radius
///      for Box ; matching dimensions for Poly).
///
/// Mitigates threats **T5** (manifest tampering) and **T8** (dim
/// mismatch) per `docs/security/PHASE2_THREAT_MODEL.md`.
pub fn validate_manifest(m: &Manifest) -> Result<(), ServerError> {
    if m.meta.dim == 0 {
        return Err(ServerError::Manifest("meta.dim must be > 0".into()));
    }
    if m.participants.is_empty() {
        return Err(ServerError::Manifest(
            "manifest must declare at least one participant".into(),
        ));
    }

    let mut seen = HashSet::with_capacity(m.participants.len());
    for p in &m.participants {
        if !seen.insert(p.id.as_str()) {
            return Err(ServerError::Manifest(format!(
                "duplicate participant id : {:?}",
                p.id,
            )));
        }
        if p.dim == 0 {
            return Err(ServerError::Manifest(format!(
                "participant {:?} : dim must be > 0",
                p.id,
            )));
        }
        // Couche 3 M1.d — engine-dependent presence rules (fail-closed both
        // ways ; see `ParticipantManifest` docs).
        match &p.controller {
            None => {
                // Mock row : kappa/seed REQUIRED, kappa in (0, 1) exactly as
                // the historical rule.
                let Some(kappa) = p.kappa else {
                    return Err(ServerError::Manifest(format!(
                        "participant {:?} : kappa is required for a mock row \
                         (no `controller` block)",
                        p.id,
                    )));
                };
                if p.seed.is_none() {
                    return Err(ServerError::Manifest(format!(
                        "participant {:?} : seed is required for a mock row \
                         (no `controller` block)",
                        p.id,
                    )));
                }
                if !(kappa > 0.0 && kappa < 1.0) {
                    return Err(ServerError::Manifest(format!(
                        "participant {:?} : kappa = {kappa} must be in (0, 1)",
                        p.id,
                    )));
                }
            }
            Some(c) => {
                // efe-core row : kappa/seed are mock-only concepts — their
                // presence is ambiguous and rejected (deprecation, doc §5).
                if p.kappa.is_some() || p.seed.is_some() {
                    return Err(ServerError::Manifest(format!(
                        "participant {:?} : kappa/seed are mock-only and must \
                         be ABSENT on an efe-core row (the engine is identified \
                         by its `controller` block / config_hash)",
                        p.id,
                    )));
                }
                validate_controller_block(&p.id, p.dim, c)?;
            }
        }
    }

    match &m.meta.safety {
        SafetyManifest::Box { radius } => {
            // Reject 0, negative, AND NaN.  Explicit NaN check first
            // because `r <= 0` lets NaN slip through silently.
            if radius.is_nan() || *radius <= 0.0 {
                return Err(ServerError::Manifest(format!(
                    "Box safety radius = {radius} must be positive (and not NaN)",
                )));
            }
        }
        SafetyManifest::Poly { dim_a, a_flat, b } => {
            let expected_len = (*dim_a as usize).saturating_mul(m.meta.dim as usize);
            if a_flat.len() != expected_len {
                return Err(ServerError::Manifest(format!(
                    "Poly.a_flat length = {} but dim_a × meta.dim = {expected_len}",
                    a_flat.len(),
                )));
            }
            if b.len() != *dim_a as usize {
                return Err(ServerError::Manifest(format!(
                    "Poly.b length = {} but dim_a = {dim_a}",
                    b.len(),
                )));
            }
        }
        SafetyManifest::Hull { .. } => {
            // Phase-2 doesn't exercise LearnedHull ; structural OK
            // is enough.
        }
    }

    Ok(())
}

/// Couche 3 M1.d — structural validation of an efe-core `controller` block.
///
/// Runs on BOTH binaries (controller + participant) regardless of the
/// `core-integration` feature : a mock-built CONTROLLER may legitimately
/// orchestrate real participants living behind gRPC, so the block must
/// validate everywhere.  What is feature-gated is only the participant-side
/// CONSTRUCTION (`LocalParticipant::from_manifest_row` fails closed on an
/// efe-core row when the binary lacks the feature).
///
/// Template-code strings are validated at construction (they need efe-core's
/// `TemplateCode::from_str`, unavailable in the default build) — here we
/// check everything structural.
fn validate_controller_block(
    id: &str,
    dim: u32,
    c: &ControllerManifest,
) -> Result<(), ServerError> {
    if c.schema != 1 {
        return Err(ServerError::Manifest(format!(
            "participant {id:?} : controller.schema = {} is not understood \
             (this binary only knows schema 1) — refusing fail-closed",
            c.schema,
        )));
    }
    if c.population.is_empty() {
        return Err(ServerError::Manifest(format!(
            "participant {id:?} : controller.population must not be empty",
        )));
    }
    for row in &c.population {
        if row.count == 0 {
            return Err(ServerError::Manifest(format!(
                "participant {id:?} : controller.population template {:?} \
                 count must be > 0",
                row.template,
            )));
        }
    }
    if c.max_rounds == 0 {
        return Err(ServerError::Manifest(format!(
            "participant {id:?} : controller.max_rounds must be > 0",
        )));
    }
    if c.alignment_cone.anchors.is_empty() {
        return Err(ServerError::Manifest(format!(
            "participant {id:?} : controller.alignment_cone.anchors must not be empty",
        )));
    }
    for (i, a) in c.alignment_cone.anchors.iter().enumerate() {
        if a.len() != dim as usize {
            return Err(ServerError::Manifest(format!(
                "participant {id:?} : controller.alignment_cone.anchors[{i}] \
                 has length {} but participant dim = {dim}",
                a.len(),
            )));
        }
        if !a.iter().all(|v| v.is_finite()) {
            return Err(ServerError::Manifest(format!(
                "participant {id:?} : controller.alignment_cone.anchors[{i}] \
                 contains a non-finite value",
            )));
        }
    }
    let mpd = c.alignment_cone.max_projection_distance;
    if !mpd.is_finite() || mpd <= 0.0 {
        return Err(ServerError::Manifest(format!(
            "participant {id:?} : controller.alignment_cone.\
             max_projection_distance = {mpd} must be finite and > 0",
        )));
    }
    let (lo, hi) = (c.safety_box.lo, c.safety_box.hi);
    if !lo.is_finite() || !hi.is_finite() || lo >= hi {
        return Err(ServerError::Manifest(format!(
            "participant {id:?} : controller.safety_box [{lo}, {hi}] must be \
             finite with lo < hi",
        )));
    }
    if c.tenant_id.is_empty() {
        return Err(ServerError::Manifest(format!(
            "participant {id:?} : controller.tenant_id must not be empty",
        )));
    }
    match c.h1_canary.mode.as_str() {
        "boot" | "off" => {}
        other => {
            return Err(ServerError::Manifest(format!(
                "participant {id:?} : controller.h1_canary.mode = {other:?} \
                 must be \"boot\" or \"off\"",
            )));
        }
    }
    if c.h1_canary.mode == "boot" {
        if c.h1_canary.n_seeds < 2 {
            return Err(ServerError::Manifest(format!(
                "participant {id:?} : controller.h1_canary.n_seeds must be ≥ 2 \
                 (the canary needs at least one pair)",
            )));
        }
        if !(0.0..1.0).contains(&c.h1_canary.margin) {
            return Err(ServerError::Manifest(format!(
                "participant {id:?} : controller.h1_canary.margin = {} must be \
                 in [0, 1)",
                c.h1_canary.margin,
            )));
        }
    }
    Ok(())
}

/// Validate that `new` is a compatible **reload candidate** for the
/// currently-running `current` manifest.  PR #12 invariant : reload
/// MUST NOT invalidate any in-flight participant state.
///
/// Rules (cf. `EFE_A1_PHASE2_PR12_ARCHITECTURE.md` § 16) :
///   1. `version` is identical.  Schema drift across reload is a caller
///      bug ; surface it rather than silently apply.
///   2. `meta.dim` is identical.  Mutation would invalidate every
///      participant's `LinearLift` / `LinearPull` operator (AD4).
///   3. `meta.safety.kind` is identical (Box/Poly/Hull).  Mutation
///      would invalidate the safety-projection contract (AD4).  The
///      kind-specific params (`Box.radius`, `Poly.a_flat` / `Poly.b`,
///      `Hull` data) MAY drift inside a same-kind reload — in-flight
///      Iterates use their snapshot ; next Iterate picks up the new
///      params.
///   4. For every participant present in BOTH manifests, `dim`,
///      `kappa` (within [`RELOAD_KAPPA_TOLERANCE`]) and `seed` are
///      identical.  Mutation breaks Phase-1's contraction / determinism
///      contracts (AD3).
///
/// Adding / removing participants is **allowed** : the controller
/// reconnects on next Iterate (AD5).  A participant added at reload
/// time has no pre-reload state to invalidate ; one removed at reload
/// simply stops appearing in the participants set.
///
/// This function runs **outside the write lock** (cf. controller
/// `apply_reload` § 9.2) so a rejected reload produces zero observable
/// change in the running state.  Caller is expected to have already
/// run [`validate_manifest`] on `new` ; this check is the reload-only
/// delta on top.
///
/// # Errors
///
/// Returns [`ServerError::Manifest`] with a diagnostic naming the
/// drifting field on the first violation.
///
/// # Threat model
///
/// Mitigates **T19** (malicious manifest swap : structural drift +
/// per-participant drift is rejected at validation time) and helps
/// uphold **T20** (race during reload : a rejected reload doesn't
/// enter the write-lock window, so in-flight Iterates aren't even
/// briefly contended).
pub fn validate_reload_compatibility(
    current: &Manifest,
    new: &Manifest,
) -> Result<(), ServerError> {
    if current.version != new.version {
        return Err(ServerError::Manifest(format!(
            "reload : manifest version drift {:?} → {:?} (not supported in Phase 2)",
            current.version, new.version,
        )));
    }
    if current.meta.dim != new.meta.dim {
        return Err(ServerError::Manifest(format!(
            "reload : meta.dim cannot change ({} → {})",
            current.meta.dim, new.meta.dim,
        )));
    }
    if !safety_kind_matches(&current.meta.safety, &new.meta.safety) {
        return Err(ServerError::Manifest(format!(
            "reload : meta.safety.kind cannot change ({} → {})",
            safety_kind_name(&current.meta.safety),
            safety_kind_name(&new.meta.safety),
        )));
    }

    // Per-participant drift check for ids present in BOTH manifests.
    // Removed (in current, not in new) and added (in new, not in current)
    // rows are allowed by AD5.
    for old_p in &current.participants {
        let Some(new_p) = new.participants.iter().find(|p| p.id == old_p.id) else {
            // Participant removed at reload → AD5 allows.
            continue;
        };
        if old_p.dim != new_p.dim {
            return Err(ServerError::Manifest(format!(
                "reload : participant {:?} dim drift {} → {} (not supported in Phase 2)",
                old_p.id, old_p.dim, new_p.dim,
            )));
        }
        // NaN-safe κ drift detection, None-aware since Couche 3 M1.d
        // (kappa is absent on efe-core rows).  Presence drift (Some ↔ None)
        // is an engine change — rejected.  If either value is NaN, treat as
        // drift (every comparison with NaN is false ; `.abs()` propagates) —
        // a NaN κ would have been caught by `validate_manifest`, but defence
        // in depth here keeps the reload path independently safe.
        match (old_p.kappa, new_p.kappa) {
            (None, None) => {}
            (Some(old_k), Some(new_k)) => {
                let drift = (old_k - new_k).abs();
                if drift.is_nan() || drift > RELOAD_KAPPA_TOLERANCE {
                    return Err(ServerError::Manifest(format!(
                        "reload : participant {:?} kappa drift {old_k} → {new_k} \
                         (tolerance {RELOAD_KAPPA_TOLERANCE}, not supported in Phase 2)",
                        old_p.id,
                    )));
                }
            }
            (old_k, new_k) => {
                return Err(ServerError::Manifest(format!(
                    "reload : participant {:?} kappa drift {old_k:?} → {new_k:?} \
                     (engine change across reload is not supported)",
                    old_p.id,
                )));
            }
        }
        if old_p.seed != new_p.seed {
            return Err(ServerError::Manifest(format!(
                "reload : participant {:?} seed drift {:?} → {:?} (not supported in Phase 2)",
                old_p.id, old_p.seed, new_p.seed,
            )));
        }
        // Couche 3 M1.d — the efe-core engine block is baked in at boot
        // (exactly like the mock's dim/kappa/seed) ; ANY drift — including
        // presence drift (mock ↔ efe-core) — is rejected fail-closed.  The
        // comparison is verbatim structural equality (`PartialEq`).
        if old_p.controller != new_p.controller {
            return Err(ServerError::Manifest(format!(
                "reload : participant {:?} controller drift (the efe-core \
                 engine block is immutable across reload ; restart to change it)",
                old_p.id,
            )));
        }
    }

    Ok(())
}

/// True iff both safety geometries are of the same variant (Box/Poly/Hull),
/// regardless of their kind-specific parameters.
fn safety_kind_matches(a: &SafetyManifest, b: &SafetyManifest) -> bool {
    matches!(
        (a, b),
        (SafetyManifest::Box { .. }, SafetyManifest::Box { .. })
            | (SafetyManifest::Poly { .. }, SafetyManifest::Poly { .. })
            | (SafetyManifest::Hull { .. }, SafetyManifest::Hull { .. })
    )
}

fn safety_kind_name(s: &SafetyManifest) -> &'static str {
    match s {
        SafetyManifest::Box { .. } => "box",
        SafetyManifest::Poly { .. } => "poly",
        SafetyManifest::Hull { .. } => "hull",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parses_minimal_manifest() {
        let body = r"
version: v0.2.0
meta:
  dim: 256
  safety:
    kind: box
    radius: 10.0
participants:
  - id: supplier-scoring
    dim: 384
    kappa: 0.6
    seed: 11
";
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(body.as_bytes()).unwrap();
        let m = load(f.path()).unwrap();
        assert_eq!(m.meta.dim, 256);
        assert_eq!(m.participants.len(), 1);
        assert_eq!(m.participants[0].id, "supplier-scoring");
    }

    #[test]
    fn parses_polyhedron_safety() {
        let body = r"
version: v0.2.0
meta:
  dim: 4
  safety:
    kind: poly
    dim_a: 2
    a_flat: [1.0, 0.0, 0.0, 1.0]
    b: [1.0, 1.0]
";
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(body.as_bytes()).unwrap();
        let m = load(f.path()).unwrap();
        match m.meta.safety {
            SafetyManifest::Poly { dim_a, .. } => assert_eq!(dim_a, 2),
            _ => panic!("expected Poly"),
        }
    }

    #[test]
    fn rejects_invalid_yaml() {
        let body = "this is :not: valid yaml: [unbalanced";
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(body.as_bytes()).unwrap();
        let err = load(f.path()).unwrap_err();
        assert!(matches!(err, ServerError::Manifest(_)));
    }

    #[test]
    fn missing_file_is_io_error() {
        let err = load("/nonexistent/path/manifest.yaml").unwrap_err();
        assert!(matches!(err, ServerError::Io(_)));
    }
}

// ---------------------------------------------------------------------------
//  Validation tests (PR #8 — 6 tests for the new `validate_manifest`).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod validation_tests {
    use super::*;

    fn good_manifest() -> Manifest {
        Manifest {
            version: "v0.2.0".into(),
            meta: MetaSpaceManifest {
                dim: 256,
                safety: SafetyManifest::Box { radius: 10.0 },
            },
            participants: vec![ParticipantManifest {
                id: "supplier-scoring".into(),
                dim: 384,
                kappa: Some(0.6),
                seed: Some(11),
                blas_backend: "none".into(),
                controller: None,
            }],
            iteration: IterationManifest::default(),
        }
    }

    #[test]
    fn validates_good_manifest() {
        validate_manifest(&good_manifest()).expect("good manifest");
    }

    #[test]
    fn rejects_zero_meta_dim() {
        let mut m = good_manifest();
        m.meta.dim = 0;
        let err = validate_manifest(&m).unwrap_err();
        assert!(matches!(err, ServerError::Manifest(_)));
    }

    #[test]
    fn rejects_empty_participants() {
        let mut m = good_manifest();
        m.participants.clear();
        let err = validate_manifest(&m).unwrap_err();
        assert!(matches!(err, ServerError::Manifest(ref msg) if msg.contains("at least one")));
    }

    #[test]
    fn rejects_duplicate_participant_id() {
        let mut m = good_manifest();
        m.participants.push(m.participants[0].clone());
        let err = validate_manifest(&m).unwrap_err();
        assert!(matches!(err, ServerError::Manifest(ref msg) if msg.contains("duplicate")));
    }

    #[test]
    fn rejects_kappa_out_of_range() {
        let mut m = good_manifest();
        m.participants[0].kappa = Some(1.5);
        let err = validate_manifest(&m).unwrap_err();
        assert!(matches!(err, ServerError::Manifest(ref msg) if msg.contains("kappa")));

        m.participants[0].kappa = Some(0.0);
        let err = validate_manifest(&m).unwrap_err();
        assert!(matches!(err, ServerError::Manifest(_)));

        m.participants[0].kappa = Some(-0.1);
        let err = validate_manifest(&m).unwrap_err();
        assert!(matches!(err, ServerError::Manifest(_)));

        // NaN : `!(NaN > 0 && NaN < 1)` reduces to `!(false && false) = true`,
        // so the existing check rejects NaN by construction.  This
        // test pins the behaviour against future refactors.
        m.participants[0].kappa = Some(f32::NAN);
        let err = validate_manifest(&m).unwrap_err();
        assert!(matches!(err, ServerError::Manifest(ref msg) if msg.contains("kappa")));
    }

    #[test]
    fn rejects_negative_box_radius() {
        let mut m = good_manifest();
        m.meta.safety = SafetyManifest::Box { radius: -1.0 };
        let err = validate_manifest(&m).unwrap_err();
        assert!(matches!(err, ServerError::Manifest(_)));
    }

    #[test]
    fn rejects_zero_box_radius() {
        let mut m = good_manifest();
        m.meta.safety = SafetyManifest::Box { radius: 0.0 };
        let err = validate_manifest(&m).unwrap_err();
        assert!(matches!(err, ServerError::Manifest(_)));
    }

    #[test]
    fn rejects_nan_box_radius() {
        // NaN slips through `r <= 0` (NaN compares false to
        // everything) ; the explicit `is_nan()` guard catches it.
        let mut m = good_manifest();
        m.meta.safety = SafetyManifest::Box { radius: f32::NAN };
        let err = validate_manifest(&m).unwrap_err();
        assert!(matches!(err, ServerError::Manifest(ref msg) if msg.contains("NaN")));
    }

    #[test]
    fn find_participant_returns_some_when_present() {
        let m = good_manifest();
        let found = find_participant(&m, "supplier-scoring").expect("present");
        assert_eq!(found.id, "supplier-scoring");
        assert_eq!(found.dim, 384);
    }

    #[test]
    fn find_participant_returns_none_when_absent() {
        let m = good_manifest();
        assert!(find_participant(&m, "unknown-id").is_none());
    }

    #[test]
    fn find_participant_handles_unicode_id() {
        let mut m = good_manifest();
        m.participants.push(ParticipantManifest {
            id: "fournisseur-éval".into(),
            dim: 64,
            kappa: Some(0.5),
            seed: Some(1),
            blas_backend: "none".into(),
            controller: None,
        });
        let found = find_participant(&m, "fournisseur-éval").expect("present");
        assert_eq!(found.id, "fournisseur-éval");
    }

    #[test]
    fn rejects_poly_with_wrong_a_flat_length() {
        let mut m = good_manifest();
        m.meta.dim = 4;
        m.meta.safety = SafetyManifest::Poly {
            dim_a: 2,
            a_flat: vec![1.0, 2.0, 3.0], // expected 2 × 4 = 8 entries
            b: vec![1.0, 1.0],
        };
        let err = validate_manifest(&m).unwrap_err();
        assert!(matches!(err, ServerError::Manifest(ref msg) if msg.contains("a_flat")));
    }
}

// ---------------------------------------------------------------------------
//  PR #12 : validate_reload_compatibility tests.
//
//  Architecture spec § 16.4 enumerates 10 distinct accept/reject cases ;
//  we ship a 1-to-1 mapping below plus the NaN-κ defence-in-depth case
//  (PR12-R4).  These are pure-function tests : no async, no I/O.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod reload_compat_tests {
    use super::*;

    fn three_participant_manifest() -> Manifest {
        Manifest {
            version: "v0.2.0".into(),
            meta: MetaSpaceManifest {
                dim: 16,
                safety: SafetyManifest::Box { radius: 50.0 },
            },
            participants: vec![
                ParticipantManifest {
                    id: "supplier-scoring".into(),
                    dim: 8,
                    kappa: Some(0.6),
                    seed: Some(11),
                    blas_backend: "none".into(),
                    controller: None,
                },
                ParticipantManifest {
                    id: "contract-risk".into(),
                    dim: 8,
                    kappa: Some(0.7),
                    seed: Some(22),
                    blas_backend: "none".into(),
                    controller: None,
                },
                ParticipantManifest {
                    id: "spend-categorization".into(),
                    dim: 8,
                    kappa: Some(0.5),
                    seed: Some(33),
                    blas_backend: "none".into(),
                    controller: None,
                },
            ],
            iteration: IterationManifest::default(),
        }
    }

    #[test]
    fn accepts_identical_manifest() {
        // The no-op reload : current == new.  Operator may legitimately
        // SIGHUP without editing the file (e.g. on Pod restart probes).
        let m = three_participant_manifest();
        validate_reload_compatibility(&m, &m).expect("identical reload must succeed");
    }

    #[test]
    fn accepts_iteration_alpha_change() {
        // The canonical hot-reload : pure α tweak with everything else
        // frozen.  This is the workflow the runbook (§ 26) advertises.
        let current = three_participant_manifest();
        let mut new = current.clone();
        new.iteration.alpha_default = 0.7;
        new.iteration.coupling_gamma = 0.4;
        new.iteration.max_iters = 200;
        new.iteration.tolerance = 1.0e-5;
        new.iteration.max_wall_ms = 25_000;
        validate_reload_compatibility(&current, &new).expect("α/γ tweak must succeed");
    }

    #[test]
    fn accepts_added_participant() {
        // AD5 : adding rows at reload is fine.  The new participant has
        // no pre-reload state, the existing three are unchanged.
        let current = three_participant_manifest();
        let mut new = current.clone();
        new.participants.push(ParticipantManifest {
            id: "fraud-detection".into(),
            dim: 8,
            kappa: Some(0.55),
            seed: Some(44),
            blas_backend: "none".into(),
            controller: None,
        });
        validate_reload_compatibility(&current, &new).expect("add participant must succeed");
    }

    #[test]
    fn accepts_removed_participant() {
        // AD5 : removing a row at reload is fine.  Controller drops
        // the connection on next Iterate ; pre-reload Iterates see
        // their snapshot.
        let current = three_participant_manifest();
        let mut new = current.clone();
        new.participants.remove(2); // drop spend-categorization
        validate_reload_compatibility(&current, &new).expect("remove participant must succeed");
    }

    #[test]
    fn accepts_safety_radius_change_within_same_kind() {
        // OQ-PR12-4 : Box.radius MAY change inside a Box→Box reload.
        // The kind is invariant ; the param is data carried inside
        // the snapshot.
        let current = three_participant_manifest();
        let mut new = current.clone();
        new.meta.safety = SafetyManifest::Box { radius: 75.0 };
        validate_reload_compatibility(&current, &new)
            .expect("safety.radius change within Box must succeed");
    }

    #[test]
    fn rejects_version_drift() {
        // Schema versions diverging across reload is a caller bug
        // (mismatched binaries vs manifest format) ; surface it.
        let current = three_participant_manifest();
        let mut new = current.clone();
        new.version = "v0.3.0".into();
        let err = validate_reload_compatibility(&current, &new).unwrap_err();
        assert!(
            matches!(err, ServerError::Manifest(ref msg) if msg.contains("version drift")),
            "{err:?}",
        );
    }

    #[test]
    fn rejects_meta_dim_change() {
        // AD4 : meta.dim change invalidates every participant's
        // Lift/Pull operator ; refuse.
        let current = three_participant_manifest();
        let mut new = current.clone();
        new.meta.dim = 32;
        let err = validate_reload_compatibility(&current, &new).unwrap_err();
        assert!(
            matches!(err, ServerError::Manifest(ref msg) if msg.contains("meta.dim")),
            "{err:?}",
        );
    }

    #[test]
    fn rejects_existing_participant_dim_change() {
        // AD3 : mutating an existing row's dim invalidates the
        // participant's tensor state ; refuse.
        let current = three_participant_manifest();
        let mut new = current.clone();
        new.participants[0].dim = 16;
        let err = validate_reload_compatibility(&current, &new).unwrap_err();
        assert!(
            matches!(err, ServerError::Manifest(ref msg) if msg.contains("dim drift")),
            "{err:?}",
        );
    }

    #[test]
    fn rejects_existing_participant_kappa_change() {
        // AD3 : mutating κ breaks contraction guarantee ; refuse.
        let current = three_participant_manifest();
        let mut new = current.clone();
        new.participants[1].kappa = Some(0.85);
        let err = validate_reload_compatibility(&current, &new).unwrap_err();
        assert!(
            matches!(err, ServerError::Manifest(ref msg) if msg.contains("kappa drift")),
            "{err:?}",
        );
    }

    #[test]
    fn rejects_existing_participant_seed_change() {
        // AD3 : mutating seed breaks bit-identical determinism ; refuse.
        let current = three_participant_manifest();
        let mut new = current.clone();
        new.participants[2].seed = Some(9999);
        let err = validate_reload_compatibility(&current, &new).unwrap_err();
        assert!(
            matches!(err, ServerError::Manifest(ref msg) if msg.contains("seed drift")),
            "{err:?}",
        );
    }

    #[test]
    fn rejects_safety_kind_change() {
        // AD4 : Box→Poly invalidates safety projection contract ;
        // refuse.
        let current = three_participant_manifest();
        let mut new = current.clone();
        new.meta.dim = 4;
        new.meta.safety = SafetyManifest::Poly {
            dim_a: 2,
            a_flat: vec![1.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0],
            b: vec![1.0, 1.0],
        };
        // Hit `meta.dim` first ; sanity-check the order of rejection.
        let err = validate_reload_compatibility(&current, &new).unwrap_err();
        assert!(
            matches!(err, ServerError::Manifest(ref msg) if msg.contains("meta.dim")),
            "{err:?}",
        );

        // Now with matching dim — should hit the safety-kind check.
        let mut current2 = current.clone();
        current2.meta.dim = 4;
        current2.meta.safety = SafetyManifest::Box { radius: 5.0 };
        let err = validate_reload_compatibility(&current2, &new).unwrap_err();
        assert!(
            matches!(err, ServerError::Manifest(ref msg) if msg.contains("safety.kind")),
            "{err:?}",
        );
    }

    #[test]
    fn nan_kappa_in_new_manifest_is_rejected_as_drift() {
        // Risk PR12-R4 : NaN κ slipping past `(a - b).abs() > tol`.
        // `validate_manifest` would normally catch NaN, but
        // `validate_reload_compatibility` MUST be safe to call
        // independently — defence in depth.
        let current = three_participant_manifest();
        let mut new = current.clone();
        new.participants[0].kappa = Some(f32::NAN);
        let err = validate_reload_compatibility(&current, &new).unwrap_err();
        assert!(
            matches!(err, ServerError::Manifest(ref msg) if msg.contains("kappa")),
            "{err:?}",
        );
    }

    #[test]
    fn tiny_kappa_jitter_within_tolerance_is_accepted() {
        // Round-tripping a κ through YAML deserialize twice can shift
        // the bits by < 1.0e-7.  Reload must not reject these.
        let current = three_participant_manifest();
        let mut new = current.clone();
        new.participants[0].kappa = Some(0.6 + 5.0e-7);
        validate_reload_compatibility(&current, &new)
            .expect("κ jitter within tolerance must be accepted");
    }
}
