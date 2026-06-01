//! # PR #27 — G.11 closure — integration test : `tolerance_used`
//! populated in every NDJSON row emitted by the verdict aggregator.
//!
//! ## Significance
//!
//! Distinct from `g11_round_trip.rs` (the round-trip pinning test
//! propriété-forte) : this file asserts the **populated-everywhere**
//! invariant — for every metric × every verdict branch, the emitted
//! row carries a `tolerance_used` field conforming to the v3 shape.
//!
//! Each test in this file pins a specific sub-defect of G.11 :
//! - **G.11.a** — historically `tolerance_used: null` in 60% of WARN paths.
//! - **G.11.b** — historically `tolerance_used` entirely absent from
//!   INCONCLUSIVE rows.
//! - **G.11.c** — historically the runtime override was calculated
//!   but the YAML value was reported.
//! - **G.11.d** — historically the `applied_at` subfield did not exist.
//!
//! Day 3 makes these 4 historical defects structurally non-regressable :
//! their test will fail if any future PR reintroduces the bug.
//!
//! ## Test-to-defect mapping
//!
//! | Test name                                                     | Pins sub-defect |
//! |---------------------------------------------------------------|-----------------|
//! | `aggregator_populates_tolerance_used_in_every_row`            | Cross-branch    |
//! | `aggregator_handles_pass_warn_fail_inconclusive_uniformly`    | G.11.a + G.11.b |
//! | `aggregator_reflects_runtime_override_in_tolerance_used`      | G.11.c (env)    |
//! | `aggregator_emits_applied_at_subfield`                        | G.11.d          |
//! | `aggregator_emits_full_v3_schema_for_every_row`               | Schema complete |
//! | `aggregator_reflects_gpu_override_in_tolerance_used`          | G.11.c (gpu)    |
//! | `aggregator_double_implication_holds_for_emitted_rows`        | Invariant       |
//!
//! ## References
//!
//! - Architecture : `docs/architecture/EFE_A1_PHASE3_PR27_S2_DAY3_ARCHITECTURE.md`
//! - STABILITY § 24.3.6 (tolerance_used shape)
//! - Companion test file : `bench/harness/tests/g11_round_trip.rs`

#![allow(clippy::needless_return)]
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};

use efe_bench_harness::compare::{
    pretend_aggregator_emit_rows, EnvContext, MetricVerdictV3, RunnerIntrospection,
    ToleranceSource, ToleranceValue,
};

// ═══════════════════════════════════════════════════════════════════
//  Workspace + fixture helpers (AD-D3-2 : duplicated intentionally)
// ═══════════════════════════════════════════════════════════════════

fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        let cargo = p.join("Cargo.toml");
        if cargo.exists() {
            let body = std::fs::read_to_string(&cargo).expect("Cargo.toml must read");
            if body.contains("[workspace]") {
                return p;
            }
        }
        assert!(p.pop(), "workspace root not found");
    }
}

fn fixture(rel: &str) -> PathBuf {
    workspace_root().join("bench").join("fixtures").join(rel)
}

fn canonical_applied_at() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-04-15T03:14:22Z")
        .unwrap()
        .with_timezone(&Utc)
}

// ═══════════════════════════════════════════════════════════════════
//  Tests 1-4 — Original 4 pin tests
// ═══════════════════════════════════════════════════════════════════

/// Drive the synthetic scenario with observed values that produce a
/// mix of verdicts (PASS / WARN / FAIL / INCONCLUSIVE), then assert
/// EVERY emitted row carries a populated `tolerance_used`.
///
/// **Cross-branch invariant** : independent of verdict branch, every
/// row has its `tolerance_used` populated.
#[test]
fn aggregator_populates_tolerance_used_in_every_row() {
    let scenario_path = fixture("scenarios/G11_synthetic.yaml");
    let observed: HashMap<String, f64> = HashMap::from_iter([
        ("metric_no_more_than_only".to_string(), 100.0),
        ("metric_within_range_pct".to_string(), 96.0),
        ("metric_no_less_than_with_gpu_override".to_string(), 60000.0),
        ("metric_unbounded".to_string(), 42.0),
    ]);
    let runner = RunnerIntrospection::cpu_only("test-runner");
    let env = EnvContext::empty();
    let applied_at = canonical_applied_at();

    let rows = pretend_aggregator_emit_rows(&scenario_path, &observed, &runner, &env, applied_at)
        .expect("aggregator must emit rows");

    assert_eq!(rows.len(), 4, "expected 4 metrics, got {}", rows.len());

    for row in &rows {
        let eff = &row.tolerance_used;
        eff.validate_internal_consistency()
            .unwrap_or_else(|e| panic!("row '{}': {e}", row.metric_name));

        let value_null = eff.value.is_none();
        let source_allows_null = matches!(
            eff.source,
            ToleranceSource::NoToleranceDefined | ToleranceSource::PreG11Unbackfilled
        );
        assert_eq!(
            value_null, source_allows_null,
            "row '{}' violates value-null ⇔ source-allows-null invariant",
            row.metric_name
        );

        assert!(
            eff.applied_at.is_some(),
            "row '{}' missing applied_at",
            row.metric_name
        );
        assert_eq!(row.schema_version, 3);
    }
}

/// **Pin G.11.a + G.11.b** : historically WARN paths had
/// `tolerance_used: null` and INCONCLUSIVE paths omitted the field
/// entirely.  This test forces all 4 verdict branches to uniformly
/// populate `tolerance_used`.
#[test]
fn aggregator_handles_pass_warn_fail_inconclusive_uniformly() {
    let scenario_path = fixture("scenarios/G11_synthetic.yaml");
    let runner = RunnerIntrospection::cpu_only("test-runner");
    let env = EnvContext::empty();
    let applied_at = canonical_applied_at();

    for observed_value in [50.0, 195.0, 300.0] {
        let mut observed = HashMap::new();
        observed.insert("metric_no_more_than_only".to_string(), observed_value);
        let rows = pretend_aggregator_emit_rows(
            &scenario_path,
            &observed,
            &runner,
            &env,
            applied_at,
        )
        .expect("aggregator must emit rows");
        let row = rows
            .iter()
            .find(|r| r.metric_name == "metric_no_more_than_only")
            .expect("metric emitted");
        assert!(
            row.tolerance_used.value.is_some(),
            "verdict {} for observed={observed_value} : tolerance_used.value must be populated (G.11.a pin)",
            row.verdict
        );
        assert!(
            row.tolerance_used.applied_at.is_some(),
            "verdict {} for observed={observed_value} : tolerance_used.applied_at must be populated (G.11.d pin)",
            row.verdict
        );
    }

    // INCONCLUSIVE branch (G.11.b pin) : observed absent → verdict
    // INCONCLUSIVE but tolerance_used still populated.
    let observed_empty: HashMap<String, f64> = HashMap::new();
    let rows = pretend_aggregator_emit_rows(
        &scenario_path,
        &observed_empty,
        &runner,
        &env,
        applied_at,
    )
    .expect("aggregator must emit rows");
    let inconc = rows
        .iter()
        .find(|r| r.metric_name == "metric_no_more_than_only")
        .expect("metric emitted");
    assert_eq!(inconc.verdict, "INCONCLUSIVE");
    assert!(
        inconc.tolerance_used.value.is_some(),
        "INCONCLUSIVE verdict must still populate tolerance_used (G.11.b pin)"
    );
}

/// **Pin G.11.c (env-var override path)** : historically the runtime
/// override was calculated correctly but the YAML value was reported.
#[test]
fn aggregator_reflects_runtime_override_in_tolerance_used() {
    let scenario_path = fixture("scenarios/G11_synthetic.yaml");
    let mut observed = HashMap::new();
    observed.insert("metric_no_more_than_only".to_string(), 350.0);

    let env = EnvContext::with_override("G11_synthetic", "metric_no_more_than_only", 500.0);
    let runner = RunnerIntrospection::cpu_only("test-runner");
    let applied_at = canonical_applied_at();

    let rows = pretend_aggregator_emit_rows(&scenario_path, &observed, &runner, &env, applied_at)
        .expect("aggregator must emit rows");
    let row = rows
        .iter()
        .find(|r| r.metric_name == "metric_no_more_than_only")
        .expect("metric emitted");

    assert_eq!(
        row.tolerance_used.source,
        ToleranceSource::RuntimeOverride,
        "source must be runtime_override (G.11.c pin)"
    );
    assert_eq!(
        row.tolerance_used
            .value
            .as_ref()
            .and_then(ToleranceValue::as_scalar),
        Some(500.0),
        "reported value must be the override (500), NOT the YAML (200) — G.11.c pin"
    );
    assert_eq!(row.verdict, "PASS");
}

/// **Pin G.11.d** : the inner `applied_at` subfield is always present
/// for non-legacy rows.
#[test]
fn aggregator_emits_applied_at_subfield() {
    let scenario_path = fixture("scenarios/G11_synthetic.yaml");
    let observed: HashMap<String, f64> =
        HashMap::from_iter([("metric_no_more_than_only".to_string(), 100.0)]);
    let runner = RunnerIntrospection::cpu_only("test-runner");
    let env = EnvContext::empty();
    let applied_at = canonical_applied_at();

    let rows = pretend_aggregator_emit_rows(&scenario_path, &observed, &runner, &env, applied_at)
        .expect("aggregator must emit rows");
    let row = rows
        .iter()
        .find(|r| r.metric_name == "metric_no_more_than_only")
        .expect("metric emitted");

    let inner = row.tolerance_used.applied_at;
    assert!(inner.is_some(), "tolerance_used.applied_at must be present");
    assert_eq!(
        inner,
        Some(applied_at),
        "tolerance_used.applied_at must match the aggregator's clock"
    );

    let json = serde_json::to_string(row).expect("serialise");
    assert!(
        json.contains(r#""applied_at":"2026-04-15T03:14:22Z""#),
        "serialised row missing applied_at literal : {json}"
    );
}

// ═══════════════════════════════════════════════════════════════════
//  Day 3 additions — schema completeness + GPU symmetric coverage
//  + explicit double-implication pin
// ═══════════════════════════════════════════════════════════════════

/// **Day 3 — full v3 schema completeness pin**.
///
/// For every row emitted by the mini-aggregator, assert that ALL
/// mandatory fields of the v3 schema are present and well-typed.
///
/// Pins the wire-level shape contract of `MetricVerdictV3` :
/// - `schema_version == 3`
/// - `scenario_id` non-empty
/// - `metric_name` non-empty
/// - `verdict` ∈ {"PASS", "WARN", "FAIL", "INCONCLUSIVE"}
/// - `tolerance_used` non-null and structurally valid
/// - `applied_at` non-null
/// - `runner` non-null and a JSON object
#[test]
fn aggregator_emits_full_v3_schema_for_every_row() {
    let scenario_path = fixture("scenarios/G11_synthetic.yaml");
    let observed: HashMap<String, f64> = HashMap::from_iter([
        ("metric_no_more_than_only".to_string(), 100.0),
        ("metric_within_range_pct".to_string(), 97.0),
        ("metric_no_less_than_with_gpu_override".to_string(), 60000.0),
        ("metric_unbounded".to_string(), 42.0),
    ]);
    let runner = RunnerIntrospection::cpu_only("test-runner-schema");
    let env = EnvContext::empty();
    let applied_at = canonical_applied_at();

    let rows = pretend_aggregator_emit_rows(&scenario_path, &observed, &runner, &env, applied_at)
        .expect("aggregator must emit rows");

    for row in &rows {
        assert_eq!(row.schema_version, 3, "schema_version must be 3 (v3)");
        assert!(!row.scenario_id.is_empty(), "scenario_id must be non-empty");
        assert!(!row.metric_name.is_empty(), "metric_name must be non-empty");
        assert!(
            matches!(
                row.verdict.as_str(),
                "PASS" | "WARN" | "FAIL" | "INCONCLUSIVE"
            ),
            "verdict '{}' not in closed taxonomy",
            row.verdict
        );
        assert!(
            row.tolerance_used.validate_internal_consistency().is_ok(),
            "tolerance_used invariant violated on {}",
            row.metric_name
        );
        assert!(
            row.applied_at <= chrono::Utc::now()
                || row.applied_at == canonical_applied_at(),
            "applied_at must be canonical or past"
        );
        assert!(
            row.runner.is_object(),
            "runner must be a JSON object on {}",
            row.metric_name
        );

        // Round-trip the entire row to confirm full-schema serde
        // compatibility.
        let json = serde_json::to_string(row).expect("serialise");
        let back: MetricVerdictV3 =
            serde_json::from_str(&json).expect("deserialise");
        assert_eq!(back.schema_version, 3);
        assert_eq!(back.tolerance_used, row.tolerance_used);
    }
}

/// **Pin G.11.c (GPU-runner override path)** — symmetric coverage with
/// `aggregator_reflects_runtime_override_in_tolerance_used`.
///
/// The G.11.c sub-defect concerned BOTH the env-var override and the
/// GPU-runner override paths : historically, both produced a row with
/// `tolerance_used = YAML value` instead of the effective override
/// value.  The env-var case is covered by the original test ; this
/// test covers the GPU-runner case symmetrically.
#[test]
fn aggregator_reflects_gpu_override_in_tolerance_used() {
    let scenario_path = fixture("scenarios/G11_synthetic.yaml");
    let mut observed = HashMap::new();
    // 120000 events/sec is above both YAML (50000) and GPU override
    // (100000) for this metric.
    observed.insert(
        "metric_no_less_than_with_gpu_override".to_string(),
        120_000.0,
    );

    let runner = RunnerIntrospection::with_gpu("gpu-runner-A100-001", "nvidia-A100-80GB");
    let env = EnvContext::empty();
    let applied_at = canonical_applied_at();

    let rows = pretend_aggregator_emit_rows(&scenario_path, &observed, &runner, &env, applied_at)
        .expect("aggregator must emit rows");
    let row = rows
        .iter()
        .find(|r| r.metric_name == "metric_no_less_than_with_gpu_override")
        .expect("metric emitted");

    assert_eq!(
        row.tolerance_used.source,
        ToleranceSource::GpuRunnerOverride,
        "source must be gpu_runner_override (G.11.c GPU pin)"
    );
    assert_eq!(
        row.tolerance_used
            .value
            .as_ref()
            .and_then(ToleranceValue::as_scalar),
        Some(100_000.0),
        "reported value must be the GPU override (100000), NOT the YAML (50000) — G.11.c GPU pin"
    );
    assert!(
        row.tolerance_used.override_reason.is_some(),
        "GPU override row must carry an override_reason"
    );
    assert_eq!(row.verdict, "PASS"); // 120k >= 100k × 1.10 = 110k, so above soft band → PASS
}

/// **Day 3 — explicit double-implication pin at the integration level**.
///
/// Iterates over every emitted row of the synthetic scenario and
/// asserts the double-implication frozen in STABILITY § 24.3.6 :
///
///   `tolerance_used.value.is_none()  ⇔
///     tolerance_used.source ∈ {NoToleranceDefined, PreG11Unbackfilled}`
///
/// This is the same invariant tested by
/// `compare::effective_tolerance::tests::validate_*` at the unit
/// level (Day 1), but here applied at the full-row integration level.
/// If a future refactor causes the aggregator to emit rows that
/// violate the invariant at run time (e.g. by producing
/// `value = null` with `source = yaml`), this test catches it.
#[test]
fn aggregator_double_implication_holds_for_emitted_rows() {
    let scenario_path = fixture("scenarios/G11_synthetic.yaml");
    let observed: HashMap<String, f64> = HashMap::from_iter([
        ("metric_no_more_than_only".to_string(), 100.0),
        ("metric_within_range_pct".to_string(), 97.0),
        ("metric_no_less_than_with_gpu_override".to_string(), 60000.0),
        ("metric_unbounded".to_string(), 42.0), // → NoToleranceDefined
    ]);
    let runner = RunnerIntrospection::cpu_only("test-runner-di");
    let env = EnvContext::empty();
    let applied_at = canonical_applied_at();

    let rows = pretend_aggregator_emit_rows(&scenario_path, &observed, &runner, &env, applied_at)
        .expect("aggregator must emit rows");

    let mut sources_seen = std::collections::HashSet::new();
    for row in &rows {
        let eff = &row.tolerance_used;
        let value_null = eff.value.is_none();
        let source_allows_null = matches!(
            eff.source,
            ToleranceSource::NoToleranceDefined | ToleranceSource::PreG11Unbackfilled
        );
        assert_eq!(
            value_null, source_allows_null,
            "row '{}' violates double-implication : value_null={} but source_allows_null={}",
            row.metric_name, value_null, source_allows_null
        );
        sources_seen.insert(eff.source);
    }

    // Confirm we exercised at least Yaml and NoToleranceDefined sources.
    assert!(
        sources_seen.contains(&ToleranceSource::Yaml),
        "expected to see Yaml source in fixture"
    );
    assert!(
        sources_seen.contains(&ToleranceSource::NoToleranceDefined),
        "expected to see NoToleranceDefined source in fixture (metric_unbounded)"
    );
}
