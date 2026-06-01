//! Sprint A1 P1 — PR #4 — Bench smoke test.
//!
//! Verifies *without* running the full criterion harness that :
//!
//!   1. The Kovant scenario builds and converges (≤ 60 iters, residual
//!      ≤ 1e-3).
//!   2. Determinism : 3 reruns produce bit-identical (iters, residual).
//!   3. The baseline YAML at `bench_baselines/v0.10.0/meta_swarm_slos.yaml`
//!      parses and contains the 7 expected SLO ids + a numeric
//!      `last_p99_observed_ms` field.
//!
//! We deliberately do **not** invoke `cargo bench` in a sub-process —
//! that would cost ~60 s on dev box and would re-discover the disk
//! pressure issue we hit on the freeze host.  The smoke test instead
//! reconstructs the exact same `KovantScenario` (small duplication of
//! constants ; pinned by Test 1 of this file).
//!
//! Cf. `crates/efe-meta-swarm/benches/three_swarms_kovant_latency.rs`
//! for the canonical scenario, and `bench_baselines/v0.10.0/meta_swarm_slos.yaml`
//! for the 7 SLOs.

use std::path::PathBuf;

use efe_meta_swarm::prelude::*;

/// Minimal local mirror of `KovantScenario` from the bench file.
/// Held tiny on purpose — Test 1 is the pin between the two.
fn build_kovant_meta() -> MetaSwarm {
    MetaSwarm::builder()
        .id("smoke-three-swarms-kovant")
        .meta_space_dim(256)
        .add_participant(
            MockParticipant::with_contraction("supplier-scoring", 384, 0.6, 11)
                .unwrap().boxed(),
        )
        .add_participant(
            MockParticipant::with_contraction("contract-risk", 512, 0.7, 22)
                .unwrap().boxed(),
        )
        .add_participant(
            MockParticipant::with_contraction("spend-categorization", 384, 0.5, 33)
                .unwrap().boxed(),
        )
        .lift_for("supplier-scoring",     LinearLift::identity(384, 256).unwrap().boxed())
        .lift_for("contract-risk",        LinearLift::identity(512, 256).unwrap().boxed())
        .lift_for("spend-categorization", LinearLift::identity(384, 256).unwrap().boxed())
        .pull_for("supplier-scoring",     LinearPull::identity(256, 384).unwrap().boxed())
        .pull_for("contract-risk",        LinearPull::identity(256, 512).unwrap().boxed())
        .pull_for("spend-categorization", LinearPull::identity(256, 384).unwrap().boxed())
        .meta_operator(UniformMetaOp::new(256).boxed())
        .meta_safety(BoxSafety::symmetric(256, 10.0).boxed())
        .alpha_default(0.5)
        .coupling_gamma(0.2)
        .tolerance(1e-4)
        .max_iters(100)
        .trace(false)
        .build().unwrap()
}

/// Test 1 — Kovant scenario converges within the architectural budget.
///
/// Pins :  iterations ≤ 60         (SLO-A1-META-CONVERGENCE-ITERS).
///         final_residual ≤ 1e-3   (SLO-A1-META-RESIDUAL-FINAL).
///         StopReason::Converged.
#[test]
fn kovant_converges_within_budget() {
    let meta = build_kovant_meta();
    let result = meta.iterate().expect("iterate must succeed");

    assert_eq!(
        result.stop_reason, StopReason::Converged,
        "Kovant scenario must converge (stop_reason was {:?})", result.stop_reason,
    );
    assert!(
        result.iterations <= 60,
        "SLO-A1-META-CONVERGENCE-ITERS : {} > 60", result.iterations,
    );
    assert!(
        result.final_residual <= 1e-3,
        "SLO-A1-META-RESIDUAL-FINAL : {} > 1e-3", result.final_residual,
    );
}

/// Test 2 — Determinism : 3 reruns produce bit-identical
/// `(iterations, final_residual)` tuples.
///
/// This is exactly the check the bench performs in
/// `bench_determinism_check()` ; we duplicate it here so a regression
/// in determinism is caught by `cargo test` even when the bench is
/// not run.  Backs SLO-A1-META-DETERMINISM.
#[test]
fn kovant_determinism_3_reruns() {
    let mut signatures = Vec::with_capacity(3);
    for _ in 0..3 {
        let meta = build_kovant_meta();
        let r = meta.iterate().expect("iterate must succeed");
        signatures.push((r.iterations, r.final_residual.to_bits()));
    }
    let all_equal = signatures.windows(2).all(|w| w[0] == w[1]);
    assert!(
        all_equal,
        "SLO-A1-META-DETERMINISM : non-deterministic reruns {signatures:?}"
    );
}

/// Test 3 — Final state stays inside its safety set (smoke).
///
/// The bench scenario uses `BoxSafety::symmetric(256, 10.0)` as meta-
/// safety ; we verify the final meta-consensus lies inside it.  This
/// is the cheapest sanity check that PR #3's `safety_every_step_meta`
/// invariants still hold on the Kovant geometry.
#[test]
fn kovant_meta_consensus_inside_safety() {
    let meta = build_kovant_meta();
    let result = meta.iterate().expect("iterate must succeed");
    let safety = BoxSafety::symmetric(256, 10.0);
    assert!(
        safety.contains(&result.meta_consensus, 1e-3),
        "final meta-consensus outside Box(±10) on Kovant scenario",
    );
}

/// Test 4 — Baseline YAML : presence + version + 7 SLO ids.
///
/// Pins the contract between the bench (which reads
/// `last_p99_observed_ms`) and the master-gate (which consumes the 7
/// SLO definitions).
#[test]
fn baseline_yaml_parses_with_7_slos() {
    let path = baseline_path();
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("baseline missing : {} : {e}", path.display()));
    let doc: serde_yaml::Value = serde_yaml::from_str(&raw)
        .expect("baseline is valid YAML");

    // Top-level fields.
    assert_eq!(
        doc.get("version").and_then(|v| v.as_str()),
        Some("v0.10.0"),
        "baseline version must be v0.10.0",
    );
    assert_eq!(
        doc.get("domain").and_then(|v| v.as_str()),
        Some("a1_meta_swarm"),
        "baseline domain must be a1_meta_swarm",
    );

    // last_p99_observed_ms must be a finite positive number.
    let p99 = doc.get("last_p99_observed_ms")
        .and_then(serde_yaml::Value::as_f64)
        .expect("last_p99_observed_ms must be a number");
    assert!(p99.is_finite() && p99 > 0.0, "p99 baseline must be > 0 finite, got {p99}");

    // The 7 expected SLO ids.
    let expected: &[&str] = &[
        "SLO-A1-META-LATENCY-P99",
        "SLO-A1-META-LATENCY-P50",
        "SLO-A1-META-CONVERGENCE-ITERS",
        "SLO-A1-META-RESIDUAL-FINAL",
        "SLO-A1-META-DETERMINISM",
        "SLO-A1-META-NON-REGRESSION",
        "SLO-A1-META-MEMORY-PEAK-MB",
    ];
    let slos = doc.get("slos").and_then(|v| v.as_sequence())
        .expect("baseline must have a `slos` sequence");
    assert_eq!(slos.len(), expected.len(), "expected 7 SLO entries");
    let actual: Vec<&str> = slos.iter()
        .filter_map(|s| s.get("id").and_then(|v| v.as_str()))
        .collect();
    for exp in expected {
        assert!(actual.contains(exp), "missing SLO id : {exp}");
    }
}

/// Test 5 — Each SLO carries BOTH encodings of the comparator+threshold :
///
///   (a) `target` + `operator` for `consolidate_all_slos.py`.
///   (b) `comparator` + `threshold_*` for `check_regression.py` + the
///       per-sprint baseline readers.
///
/// The dual encoding is intentional ; pinning both parsers ensures we
/// catch any partial migration in either direction.
#[test]
fn baseline_yaml_comparators_valid() {
    let raw = std::fs::read_to_string(baseline_path()).unwrap();
    let doc: serde_yaml::Value = serde_yaml::from_str(&raw).unwrap();
    let slos = doc["slos"].as_sequence().unwrap();
    for s in slos {
        let id = s["id"].as_str().unwrap();

        // (a) consolidate_all_slos.py contract.
        let target_present = s.get("target").is_some();
        let op = s["operator"].as_str()
            .unwrap_or_else(|| panic!("{id} : missing operator"));
        assert!(target_present, "{id} : missing target (consolidator contract)");
        assert!(
            matches!(op, ">=" | "<=" | "=="),
            "{id} : invalid operator '{op}' (expected '>=' | '<=' | '==')",
        );

        // (b) check_regression.py + per-sprint contract.
        let cmp = s["comparator"].as_str()
            .unwrap_or_else(|| panic!("{id} : missing comparator"));
        assert!(
            matches!(cmp, "lte" | "gte" | "eq"),
            "{id} : invalid comparator '{cmp}' (expected 'lte' | 'gte' | 'eq')",
        );

        // Cross-check : `lte` ↔ `<=`, `gte` ↔ `>=`, `eq` ↔ `==`.
        let consistent = matches!((cmp, op),
            ("lte", "<=") | ("gte", ">=") | ("eq", "=="));
        assert!(
            consistent,
            "{id} : comparator '{cmp}' and operator '{op}' disagree",
        );
    }
}

/// Test 7 — Every SLO has a `deferred_ticket` until the soft-launch
/// observation window (3 main runs) elapses.
///
/// This is the safety net against the master-gate breaking the moment
/// the bench has a transient hiccup.  When the bench stabilises, a
/// follow-up PR removes the `deferred_ticket` rows (cf.
/// `docs/EFE_A1_BENCH_TRACKING.md` § "Removing the soft-launch ticket").
#[test]
fn baseline_yaml_soft_launch_dt_present() {
    let raw = std::fs::read_to_string(baseline_path()).unwrap();
    let doc: serde_yaml::Value = serde_yaml::from_str(&raw).unwrap();
    let slos = doc["slos"].as_sequence().unwrap();
    for s in slos {
        let id = s["id"].as_str().unwrap();
        let dt = s["deferred_ticket"].as_str()
            .unwrap_or_else(|| panic!("{id} : missing deferred_ticket (soft-launch)"));
        assert!(
            dt.starts_with("DT-A1-"),
            "{id} : deferred_ticket '{dt}' must start with 'DT-A1-'",
        );
    }
}

/// Test 6 — `host_fingerprint_baseline` is set (not a placeholder).
///
/// Phase 1 freeze must capture the real host.  A placeholder slipping
/// to main signals the freeze script was not run.
#[test]
fn baseline_yaml_host_fingerprint_real() {
    let raw = std::fs::read_to_string(baseline_path()).unwrap();
    let doc: serde_yaml::Value = serde_yaml::from_str(&raw).unwrap();
    let fp = doc["host_fingerprint_baseline"].as_str()
        .expect("host_fingerprint_baseline must be present");
    assert!(!fp.is_empty(), "fingerprint must not be empty");
    assert!(
        !fp.starts_with("PLACEHOLDER"),
        "fingerprint must not be a PLACEHOLDER value, got {fp:?}",
    );
}

/// Test 8 — Master rollup `SLO-OPS-A1-META-SWARM` exists in
/// `bench_baselines/v0.9.0/ops_master_slos.yaml` and inherits EXACTLY
/// the 7 surface A1 SLO ids.
///
/// This is the **load-bearing pin** for the master-gate hook : without
/// the rollup, `master_gate_check.py` ignores A1 SLO regressions
/// silently (cf. PR #4 P7 audit fix).  A future refactor that drops
/// the rollup must remove this test in the same commit.
#[test]
fn master_rollup_inherits_all_seven_a1_slos() {
    let raw = std::fs::read_to_string(master_baseline_path())
        .unwrap_or_else(|e| panic!("master baseline missing : {e}"));
    let doc: serde_yaml::Value = serde_yaml::from_str(&raw)
        .expect("master baseline is valid YAML");

    let master_slos = doc["spec"]["slos"].as_sequence()
        .expect("master spec.slos must be a sequence");

    let rollup = master_slos.iter()
        .find(|s| s.get("id").and_then(|v| v.as_str())
                  == Some("SLO-OPS-A1-META-SWARM"))
        .expect(
            "Master rollup `SLO-OPS-A1-META-SWARM` is missing from \
             `bench_baselines/v0.9.0/ops_master_slos.yaml`. \
             Without it, master_gate_check.py will not block on \
             A1 regressions (cf. PR #4 P7 audit fix).",
        );

    // Status must be ACTIVE — DRAFT/INACTIVE would silently disable
    // the gate.
    let status = rollup["status"].as_str()
        .expect("rollup status must be a string");
    assert_eq!(status, "ACTIVE",
        "rollup status must be ACTIVE (was {status:?}) — otherwise the \
         master gate skips it silently");

    // Inherits must contain EXACTLY the 7 surface SLO ids.
    let inherits: Vec<&str> = rollup["inherits"].as_sequence()
        .expect("rollup must declare `inherits` list")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    let expected: &[&str] = &[
        "SLO-A1-META-LATENCY-P99",
        "SLO-A1-META-LATENCY-P50",
        "SLO-A1-META-CONVERGENCE-ITERS",
        "SLO-A1-META-RESIDUAL-FINAL",
        "SLO-A1-META-DETERMINISM",
        "SLO-A1-META-NON-REGRESSION",
        "SLO-A1-META-MEMORY-PEAK-MB",
    ];
    assert_eq!(
        inherits.len(), expected.len(),
        "rollup must inherit exactly 7 surface ids (got {} : {:?})",
        inherits.len(), inherits,
    );
    for exp in expected {
        assert!(
            inherits.contains(exp),
            "rollup inherits is missing surface id : {exp}",
        );
    }
}

// ---------------------------------------------------------------------------
//  Helpers
// ---------------------------------------------------------------------------

/// Workspace-root-relative path to the v0.10.0 baseline YAML.
fn baseline_path() -> PathBuf {
    // CARGO_MANIFEST_DIR points to `crates/efe-meta-swarm/`.
    // The baseline lives at `bench_baselines/v0.10.0/meta_swarm_slos.yaml`
    // relative to the workspace root, i.e. two `..` levels up.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent().unwrap()      // crates/
        .parent().unwrap()      // workspace root
        .join("bench_baselines/v0.10.0/meta_swarm_slos.yaml")
}

/// Workspace-root-relative path to the v0.9.0 master SLO YAML.
fn master_baseline_path() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent().unwrap()      // crates/
        .parent().unwrap()      // workspace root
        .join("bench_baselines/v0.9.0/ops_master_slos.yaml")
}
