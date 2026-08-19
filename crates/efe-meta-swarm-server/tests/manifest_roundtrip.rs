//! PR #13 — round-trip the Helm-rendered manifest YAML through
//! Rust's [`efe_meta_swarm_server::manifest::load_and_validate`].
//!
//! Mechanism :
//!   1. The path to a `manifest.yaml` file is passed via the
//!      `EFE_PR13_MANIFEST_PATH` env var (CI extracts it from the
//!      `helm template` output using `yq`).
//!   2. The test calls `load_and_validate` and asserts `Ok(_)`.
//!   3. Sanity checks on the parsed structure (version, dim,
//!      participants count > 0).
//!
//! When the env var is unset, the test is **skipped** (returns
//! early with a tracing diagnostic) so local `cargo test` runs
//! don't fail without the Helm-rendered file at hand.
//!
//! See `docs/EFE_A1_PHASE2_PR13_ARCHITECTURE.md` § 27.1 and § 35.4.

use std::path::PathBuf;

use efe_meta_swarm_server::manifest::{self, SafetyManifest};

const ENV_VAR: &str = "EFE_PR13_MANIFEST_PATH";

#[test]
fn helm_rendered_manifest_round_trips_through_load_and_validate() {
    let Some(path_raw) = std::env::var_os(ENV_VAR) else {
        eprintln!(
            "PR13 round-trip test : {ENV_VAR} unset — skipping. CI sets this \
             via `yq` extraction from `helm template` output.",
        );
        return;
    };
    let path = PathBuf::from(&path_raw);
    assert!(
        path.exists(),
        "{ENV_VAR} = {} points at a file that does not exist",
        path.display(),
    );

    // The load-bearing assertion.  If the helm template produces YAML
    // that the Rust validator rejects, the chart's manifest schema
    // has drifted from the Rust schema — fail the build.
    let parsed = match manifest::load_and_validate(&path) {
        Ok(m) => m,
        Err(e) => panic!(
            "PR13 round-trip FAILED : {} did not validate ; \
             diagnostic = {e:?}",
            path.display(),
        ),
    };

    // Sanity : the chart's defaults map to a coherent manifest.
    assert!(!parsed.version.is_empty(), "version must be non-empty");
    assert!(parsed.meta.dim >= 1, "meta.dim must be >= 1");
    assert!(
        !parsed.participants.is_empty(),
        "manifest must have at least one participant",
    );

    // Cross-check safety variant matches the rendered manifest.
    // The helm chart uses `kind` as the discriminator ; the Rust
    // validator parses it as a tagged enum.  This assertion
    // pins the kind→variant mapping for both ends.
    let safety_kind = match &parsed.meta.safety {
        SafetyManifest::Box { radius } => {
            assert!(*radius > 0.0, "Box radius must be positive ; got {radius}");
            "box"
        }
        SafetyManifest::Poly { dim_a, a_flat, b } => {
            assert!(*dim_a >= 1, "Poly.dim_a must be >= 1");
            assert!(!a_flat.is_empty(), "Poly.a_flat must be non-empty");
            assert!(!b.is_empty(), "Poly.b must be non-empty");
            assert_eq!(b.len(), *dim_a as usize, "Poly.b.len must equal dim_a",);
            "poly"
        }
        SafetyManifest::Hull {
            version,
            serialized_model_path,
        } => {
            assert!(!version.is_empty(), "Hull.version must be non-empty");
            assert!(
                !serialized_model_path.is_empty(),
                "Hull.serialized_model_path must be non-empty",
            );
            "hull"
        }
    };

    eprintln!(
        "PR13 round-trip OK : version={} meta.dim={} safety.kind={safety_kind} participants={}",
        parsed.version,
        parsed.meta.dim,
        parsed.participants.len(),
    );

    // Every participant has kappa in (0, 1) — defence in depth ;
    // `validate_manifest` already checked this, but pinning it here
    // protects against future Rust validator regressions.
    for p in &parsed.participants {
        // Couche 3 M1.d : kappa is a mock-only concept — required (and range-
        // checked) only on rows WITHOUT an efe-core `controller` block.
        if p.controller.is_none() {
            let kappa = p.kappa.expect("mock row must declare kappa");
            assert!(
                kappa > 0.0 && kappa < 1.0,
                "participant {} has kappa = {kappa} (must be in (0, 1))",
                p.id,
            );
        }
        assert!(p.dim >= 1, "participant {} has dim = 0", p.id);
    }
}
