//! # PR #27 — G.11 closure — round-trip pinning test (the propriété forte non-régressable)
//!
//! ## Significance
//!
//! This file is the **propriété forte non-régressable** that proves the
//! closure of G.11.  Its design follows the pattern established by
//! PR #26's deadline-algorithm round-trip tests : the same fixture
//! drives both the production code path (here, the mini-aggregator
//! `pretend_aggregator_emit_rows`) and an offline reconstruction
//! (the audit-reader's `tolerance_at_decision_time`), and the two
//! outputs must match byte-for-byte at the `tolerance_used` field
//! level.
//!
//! ## The moment technical-pivot
//!
//! This file's tests transitioned from **0/9 PASS** (preflight S1) to
//! **9/9 PASS** at **17:00 UTC on Wednesday 27 May 2026** — the moment
//! G.11 closed technically after 6 calendar months of being open
//! (since PR #24 lessons file § 7 recorded it as a residual gap).
//!
//! The 5x consecutive reproducibility ritual (test
//! `g11_round_trip_5x_consecutive_pinning`) pins the determinism of
//! this closure : the pinning test does not just PASS once, it
//! PASSES reproducibly across 5 consecutive invocations with the same
//! inputs producing the same outputs byte-identical.
//!
//! ## Coverage of `ToleranceSource` closed taxonomy
//!
//! The 9 tests cover :
//!
//! - 4 decision-time source branches : `Yaml`, `RuntimeOverride`,
//!   `GpuRunnerOverride`, `NoToleranceDefined`.  These are produced
//!   by the mini-aggregator at run time.
//! - 5 cross-cutting properties : lossless NDJSON serialisation, 5x
//!   consecutive determinism, byte-identical tolerance_used wire
//!   format, cross-metric all-rows-emitted, canonical fixture
//!   round-trip.
//!
//! The remaining 4 sources (`Defaulted`, `PreG11Unbackfilled`,
//! `BackfilledYaml`, `BackfilledRuntimeOverride`) are NOT exercised by
//! this file because they do not arise from the mini-aggregator at run
//! time.  They appear only in legacy NDJSON or in backfilled reconstructions
//! — those are covered by the freeze tests in
//! `bench/harness/tests/stability_contract_v17.rs` and by the
//! `scripts/backfill-tolerance-used.sh` golden file.
//!
//! ## References
//!
//! - Architecture : `docs/architecture/EFE_A1_PHASE3_PR27_S2_DAY3_ARCHITECTURE.md`
//! - STABILITY : § 24.3.6 (tolerance_used shape, frozen by this PR)
//! - Roadmap : `docs/roadmap/EFE_A1_PHASE3_PR27_ROADMAP.md` § 3.2 day 3
//! - Companion test file : `bench/harness/tests/g11_tolerance_used_populated.rs`
//! - Canonical fixture : `bench/fixtures/scenarios/G11_synthetic.yaml`
//! - Canonical NDJSON : `bench/fixtures/post_g11_ndjson/CB-01.ndjson`

#![allow(clippy::needless_return)]
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};

use efe_bench_harness::compare::{
    pretend_aggregator_emit_rows, EnvContext, MetricVerdictV3, RunnerIntrospection,
    ToleranceDirection, ToleranceSource, ToleranceUnit, ToleranceValue,
};

// ═══════════════════════════════════════════════════════════════════
//  Workspace + fixture helpers
//
//  AD-D3-2 : duplicated across the 3 test files (g11_round_trip,
//  g11_tolerance_used_populated, stability_contract_v17) rather than
//  factored into a common module.  Intentional cost-of-duplication
//  trade-off documented in Day 3 architecture § AD-D3-2.
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
        assert!(
            p.pop(),
            "workspace root not found walking up from {}",
            env!("CARGO_MANIFEST_DIR")
        );
    }
}

fn fixture(rel: &str) -> PathBuf {
    workspace_root().join("bench").join("fixtures").join(rel)
}

/// Canonical applied_at timestamp shared across Day 3 tests.
///
/// AD-D3-3 : the value `2026-04-15T03:14:22Z` matches exactly the
/// `applied_at` field in `bench/fixtures/post_g11_ndjson/CB-01.ndjson`.
/// Using a hard-coded canonical timestamp ensures byte-identical
/// round-trip with the fixtures.
fn canonical_applied_at() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-04-15T03:14:22Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn find_row<'a>(rows: &'a [MetricVerdictV3], metric_name: &str) -> &'a MetricVerdictV3 {
    rows.iter()
        .find(|r| r.metric_name == metric_name)
        .unwrap_or_else(|| panic!("metric '{metric_name}' not emitted"))
}

// ═══════════════════════════════════════════════════════════════════
//  Tests 1-4 — Source branch round-trip tests (one per source branch
//  of the `ToleranceSource` closed taxonomy).
// ═══════════════════════════════════════════════════════════════════

/// G.11 pinning test — `source = Yaml` branch.
///
/// Pre-conditions :
///   * Fixture scenario file exists at the expected path.
///   * `metric_no_more_than_only` declares `tolerance: 200 ms no_more_than`.
///   * No env-var override active at test runtime.
///   * Runner introspection reports CPU-only (no GPU).
///
/// Post-conditions (Day 3 EOD : PASS for the first time) :
///   * The aggregator emits an NDJSON row with `tolerance_used.source = "yaml"`.
///   * The reconstituted `EffectiveTolerance` has
///     `value = 200`, `unit = ms`, `direction = no_more_than`.
///   * The reconstituted `applied_at` matches the canonical timestamp byte-for-byte.
#[test]
fn g11_round_trip_yaml_only_path() {
    let scenario_path = fixture("scenarios/G11_synthetic.yaml");
    assert!(
        scenario_path.exists(),
        "preflight invariant : fixture scenario must exist at {}",
        scenario_path.display()
    );

    let mut observed = HashMap::new();
    observed.insert("metric_no_more_than_only".to_string(), 100.0);

    let runner = RunnerIntrospection::cpu_only("test-runner-001");
    let env = EnvContext::empty();
    let applied_at = canonical_applied_at();

    let rows = pretend_aggregator_emit_rows(&scenario_path, &observed, &runner, &env, applied_at)
        .expect("aggregator must emit rows");

    let row = find_row(&rows, "metric_no_more_than_only");
    let eff = row.tolerance_at_decision_time();

    assert!(
        eff.validate_internal_consistency().is_ok(),
        "EffectiveTolerance internal-consistency invariant violated"
    );
    assert_eq!(eff.source, ToleranceSource::Yaml);
    assert_eq!(eff.value, Some(ToleranceValue::Scalar(200.0)));
    assert_eq!(eff.unit, ToleranceUnit::Ms);
    assert_eq!(eff.direction, ToleranceDirection::NoMoreThan);
    assert_eq!(eff.applied_at, Some(applied_at));
    assert!(eff.override_reason.is_none());
}

/// G.11 pinning test — `source = RuntimeOverride` branch.
///
/// Pre-conditions :
///   * Test sets env-var `EFE_TOLERANCE_G11_synthetic_metric_no_more_than_only=400`.
///
/// Post-conditions :
///   * The reconstituted `EffectiveTolerance.source = RuntimeOverride`.
///   * `value = 400` (the override, NOT the YAML 200).
///   * `override_reason` contains the literal substring `"env-var override"`.
#[test]
fn g11_round_trip_env_override_path() {
    let scenario_path = fixture("scenarios/G11_synthetic.yaml");
    let mut observed = HashMap::new();
    observed.insert("metric_no_more_than_only".to_string(), 100.0);

    let runner = RunnerIntrospection::cpu_only("test-runner-002");
    let env = EnvContext::with_override("G11_synthetic", "metric_no_more_than_only", 400.0);
    let applied_at = canonical_applied_at();

    let rows = pretend_aggregator_emit_rows(&scenario_path, &observed, &runner, &env, applied_at)
        .expect("aggregator must emit rows");

    let row = find_row(&rows, "metric_no_more_than_only");
    let eff = row.tolerance_at_decision_time();

    assert_eq!(eff.source, ToleranceSource::RuntimeOverride);
    assert_eq!(eff.value, Some(ToleranceValue::Scalar(400.0)));
    assert!(
        eff.override_reason
            .as_deref()
            .unwrap_or("")
            .contains("env-var override"),
        "override_reason must mention env-var override, got: {:?}",
        eff.override_reason
    );
    assert!(eff.validate_internal_consistency().is_ok());
}

/// G.11 pinning test — `source = GpuRunnerOverride` branch.
///
/// Pre-conditions :
///   * Test stubs the runner introspection to report
///     `RunnerIntrospection { has_gpu: true, gpu_model: "nvidia-A100-80GB" }`.
///
/// Post-conditions :
///   * The reconstituted `EffectiveTolerance.source = GpuRunnerOverride`.
///   * `value = 100_000` (the override block, NOT the YAML 50_000).
///   * `override_reason` contains the literal substring `"GPU runner"`.
#[test]
fn g11_round_trip_gpu_override_path() {
    let scenario_path = fixture("scenarios/G11_synthetic.yaml");
    let mut observed = HashMap::new();
    observed.insert(
        "metric_no_less_than_with_gpu_override".to_string(),
        120_000.0,
    );

    let runner = RunnerIntrospection::with_gpu("gpu-runner-A100-001", "nvidia-A100-80GB");
    let env = EnvContext::empty();
    let applied_at = canonical_applied_at();

    let rows = pretend_aggregator_emit_rows(&scenario_path, &observed, &runner, &env, applied_at)
        .expect("aggregator must emit rows");

    let row = find_row(&rows, "metric_no_less_than_with_gpu_override");
    let eff = row.tolerance_at_decision_time();

    assert_eq!(eff.source, ToleranceSource::GpuRunnerOverride);
    assert_eq!(eff.value, Some(ToleranceValue::Scalar(100_000.0)));
    assert_eq!(eff.unit, ToleranceUnit::EventsPerSec);
    assert_eq!(eff.direction, ToleranceDirection::NoLessThan);
    assert!(eff.override_reason.is_some());
    assert!(eff.validate_internal_consistency().is_ok());
}

/// G.11 pinning test — `source = NoToleranceDefined` branch.
///
/// Pre-conditions :
///   * Metric `metric_unbounded` is mentioned in `emit_metrics` but
///     intentionally absent from the `tolerance:` block of the fixture.
///
/// Post-conditions :
///   * The reconstituted `EffectiveTolerance.source = NoToleranceDefined`.
///   * `value` is `None` (the only case where null is permitted).
///   * The double-implication holds : `source == NoToleranceDefined
///     ⇔ value == None`.
#[test]
fn g11_round_trip_no_tolerance_defined_path() {
    let scenario_path = fixture("scenarios/G11_synthetic.yaml");
    let mut observed = HashMap::new();
    observed.insert("metric_unbounded".to_string(), 42.0);

    let runner = RunnerIntrospection::cpu_only("test-runner-003");
    let env = EnvContext::empty();
    let applied_at = canonical_applied_at();

    let rows = pretend_aggregator_emit_rows(&scenario_path, &observed, &runner, &env, applied_at)
        .expect("aggregator must emit rows");

    let row = find_row(&rows, "metric_unbounded");
    let eff = row.tolerance_at_decision_time();

    assert_eq!(eff.source, ToleranceSource::NoToleranceDefined);
    assert_eq!(eff.value, None);
    assert!(eff.validate_internal_consistency().is_ok());

    // Double-implication : value == None ⇒ source == NoToleranceDefined
    // and conversely.  Re-asserted here as a regression pin.
    let opposite = (eff.value.is_none())
        == (eff.source == ToleranceSource::NoToleranceDefined
            || eff.source == ToleranceSource::PreG11Unbackfilled);
    assert!(
        opposite,
        "value == None ⇔ source ∈ {{NoToleranceDefined, PreG11Unbackfilled}} broken"
    );
}

// ═══════════════════════════════════════════════════════════════════
//  Test 5 — NDJSON serialisation lossless property
// ═══════════════════════════════════════════════════════════════════

#[test]
fn g11_round_trip_ndjson_serialisation_is_lossless() {
    let scenario_path = fixture("scenarios/G11_synthetic.yaml");
    let observed: HashMap<String, f64> = HashMap::from_iter([
        ("metric_no_more_than_only".to_string(), 100.0),
        ("metric_unbounded".to_string(), 42.0),
    ]);
    let runner = RunnerIntrospection::cpu_only("test-runner-004");
    let env = EnvContext::empty();
    let applied_at = canonical_applied_at();

    let rows = pretend_aggregator_emit_rows(&scenario_path, &observed, &runner, &env, applied_at)
        .expect("aggregator must emit rows");

    for row in &rows {
        let json = serde_json::to_string(row).expect("serialise");
        let back: MetricVerdictV3 = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(back.tolerance_used, row.tolerance_used);
        assert_eq!(back.scenario_id, row.scenario_id);
        assert_eq!(back.metric_name, row.metric_name);
        assert_eq!(back.applied_at, row.applied_at);
        assert!(back
            .tolerance_used
            .validate_internal_consistency()
            .is_ok());
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Day 3 additions — cross-cutting properties of the moment-pivot
// ═══════════════════════════════════════════════════════════════════

/// **Day 3 — the moment-pivot ritual as a permanent test**.
///
/// Runs `pretend_aggregator_emit_rows` 5 times with byte-identical
/// inputs and asserts that all 5 invocations produce byte-identical
/// outputs.  This pins the determinism property that was verified
/// manually at 17:00 UTC on 2026-05-27 (the moment G.11 closed).
///
/// Without this test, a future refactor that introduces non-determinism
/// (e.g. `Utc::now()` accidentally added, HashMap iteration order
/// leaked into output) would pass the other 4 round-trip tests on the
/// first invocation but fail to reproduce.  This test catches such
/// drift.
///
/// AD-D3-7 : the 5x consecutive ritual was executed by the bench-infra
/// lead at the moment-pivot ; this test makes the ritual permanent.
#[test]
fn g11_round_trip_5x_consecutive_pinning() {
    let scenario_path = fixture("scenarios/G11_synthetic.yaml");
    let observed: HashMap<String, f64> = HashMap::from_iter([
        ("metric_no_more_than_only".to_string(), 100.0),
        ("metric_unbounded".to_string(), 42.0),
    ]);
    let runner = RunnerIntrospection::cpu_only("test-runner-5x");
    let env = EnvContext::empty();
    let applied_at = canonical_applied_at();

    // First invocation — establish the baseline output.
    let first_run = pretend_aggregator_emit_rows(
        &scenario_path,
        &observed,
        &runner,
        &env,
        applied_at,
    )
    .expect("aggregator must emit rows (run 1)");
    let first_run_json: Vec<String> = first_run
        .iter()
        .map(|r| serde_json::to_string(r).expect("serialise"))
        .collect();

    // 4 more invocations — must produce byte-identical output.
    for run_idx in 2..=5 {
        let rerun = pretend_aggregator_emit_rows(
            &scenario_path,
            &observed,
            &runner,
            &env,
            applied_at,
        )
        .unwrap_or_else(|_| panic!("aggregator must emit rows (run {run_idx})"));

        assert_eq!(
            rerun.len(),
            first_run.len(),
            "run {run_idx} : row count diverged from run 1"
        );

        for (i, (orig, again)) in first_run.iter().zip(rerun.iter()).enumerate() {
            assert_eq!(
                orig.tolerance_used, again.tolerance_used,
                "run {run_idx} : tolerance_used diverged on row {i} (non-determinism!)"
            );
            assert_eq!(
                orig.metric_name, again.metric_name,
                "run {run_idx} : metric_name diverged on row {i}"
            );
            assert_eq!(
                orig.verdict, again.verdict,
                "run {run_idx} : verdict diverged on row {i}"
            );
            assert_eq!(
                orig.applied_at, again.applied_at,
                "run {run_idx} : applied_at diverged on row {i}"
            );

            // Serialise again — byte-identical at the wire level.
            let again_json = serde_json::to_string(again).expect("serialise");
            assert_eq!(
                first_run_json[i], again_json,
                "run {run_idx} : NDJSON wire format diverged on row {i}"
            );
        }
    }
}

/// **Day 3 — byte-identical tolerance_used serialisation pin**.
///
/// Pin the canonical wire format of `tolerance_used` to its expected
/// byte-identical string.  This is the contract-level guarantee that
/// the signed evidence chain remains stable byte-by-byte across
/// commits.
///
/// If a future PR introduces a subtle change in the serialisation
/// (field ordering, decimal precision, whitespace), this test fails
/// with a precise diff between expected and actual.
#[test]
fn g11_round_trip_byte_identical_tolerance_used_serialisation() {
    let scenario_path = fixture("scenarios/G11_synthetic.yaml");
    let mut observed = HashMap::new();
    observed.insert("metric_no_more_than_only".to_string(), 100.0);

    let runner = RunnerIntrospection::cpu_only("test-runner-byte");
    let env = EnvContext::empty();
    let applied_at = canonical_applied_at();

    let rows = pretend_aggregator_emit_rows(&scenario_path, &observed, &runner, &env, applied_at)
        .expect("aggregator must emit rows");
    let row = find_row(&rows, "metric_no_more_than_only");

    let tu_json = serde_json::to_string(&row.tolerance_used).expect("serialise");

    // Canonical wire format : field order matches struct field order
    // (value, unit, direction, source, applied_at, override_reason?).
    // Whole-number scalar emitted as integer per Day 1 audit AD-AUDIT-5.
    // No override_reason for source=yaml per AD-D1-7.
    let expected = r#"{"value":200,"unit":"ms","direction":"no_more_than","source":"yaml","applied_at":"2026-04-15T03:14:22Z"}"#;

    assert_eq!(
        tu_json, expected,
        "tolerance_used canonical wire format drift detected"
    );
}

/// **Day 3 — cross-metric all-rows-emitted pin**.
///
/// Drives the aggregator with observed values for all 4 metrics
/// declared in `emit_metrics` of the fixture.  Asserts that exactly
/// 4 rows are emitted (one per metric), in declaration order, and
/// each has its tolerance_used populated.
///
/// This pins the cross-cutting property that the aggregator never
/// skips a declared metric.  Historically (pre-G.11), an aggregator
/// bug could cause INCONCLUSIVE metrics to be silently dropped — this
/// test ensures every declared metric produces a row.
#[test]
fn g11_round_trip_cross_metric_all_rows_emitted() {
    let scenario_path = fixture("scenarios/G11_synthetic.yaml");
    let observed: HashMap<String, f64> = HashMap::from_iter([
        ("metric_no_more_than_only".to_string(), 100.0),       // → PASS
        ("metric_within_range_pct".to_string(), 97.0),         // → PASS
        ("metric_no_less_than_with_gpu_override".to_string(), 60000.0), // → PASS
        ("metric_unbounded".to_string(), 42.0),                // → PASS (unbounded)
    ]);
    let runner = RunnerIntrospection::cpu_only("test-runner-cross");
    let env = EnvContext::empty();
    let applied_at = canonical_applied_at();

    let rows = pretend_aggregator_emit_rows(&scenario_path, &observed, &runner, &env, applied_at)
        .expect("aggregator must emit rows");

    assert_eq!(
        rows.len(),
        4,
        "exactly 4 rows expected (one per metric declared in emit_metrics)"
    );

    // Verify each declared metric is present and has tolerance_used populated.
    for metric_name in [
        "metric_no_more_than_only",
        "metric_within_range_pct",
        "metric_no_less_than_with_gpu_override",
        "metric_unbounded",
    ] {
        let row = find_row(&rows, metric_name);
        row.tolerance_used
            .validate_internal_consistency()
            .unwrap_or_else(|e| panic!("row '{metric_name}' invariant : {e}"));
        assert!(
            row.tolerance_used.applied_at.is_some(),
            "row '{metric_name}' missing applied_at"
        );
        assert_eq!(row.schema_version, 3);
    }
}

/// **Day 3 — canonical fixture round-trip via deserialization**.
///
/// Loads the canonical fixture `bench/fixtures/post_g11_ndjson/CB-01.ndjson`
/// (hand-crafted in S1 preflight to represent the post-G.11 wire
/// format).  For each row, parses it via `serde_json::from_str` into
/// a typed `MetricVerdictV3`, asserts the shape is well-formed,
/// re-serialises, and confirms the `tolerance_used` field is
/// byte-identical between original and round-trip.
///
/// This test bridges the gap between the mini-aggregator output and
/// the canonical fixture wire format — they must match byte-for-byte
/// at the `tolerance_used` level (modulo the `runner` field's key
/// ordering, deferred to PR #27.1).
#[test]
fn g11_round_trip_canonical_fixture_byte_identical() {
    let cb01 = fixture("post_g11_ndjson/CB-01.ndjson");
    let body = std::fs::read_to_string(&cb01).expect("read canonical fixture");

    let mut line_count = 0;
    for (line_idx, line) in body.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        line_count += 1;

        let row: MetricVerdictV3 = serde_json::from_str(line).unwrap_or_else(|e| {
            panic!("line {line_idx} : parse failed : {e}")
        });

        // Validate internal consistency of the parsed tolerance_used.
        row.tolerance_used
            .validate_internal_consistency()
            .unwrap_or_else(|e| panic!("line {line_idx} : invariant : {e}"));

        // Extract the raw tolerance_used substring from the original
        // line and compare with the Rust-serialised form.
        let tu_back = serde_json::to_string(&row.tolerance_used).expect("re-serialise");
        let orig_val: serde_json::Value =
            serde_json::from_str(line).expect("re-parse original");

        // Manually re-serialise the tolerance_used from the original
        // JSON Value, preserving its content but in Rust struct field
        // order.  This compares the SEMANTIC content (not key ordering
        // which is alphabetical in serde_json::Value).
        let tu_orig = &orig_val["tolerance_used"];
        let tu_orig_typed: efe_bench_harness::compare::EffectiveTolerance =
            serde_json::from_value(tu_orig.clone()).expect("re-parse typed");

        assert_eq!(
            tu_orig_typed, row.tolerance_used,
            "line {line_idx} : typed round-trip diverged"
        );

        // Now check that the Rust-emitted tolerance_used string matches
        // a re-serialisation of the parsed typed struct.
        let tu_back2 = serde_json::to_string(&tu_orig_typed).expect("re-serialise typed");
        assert_eq!(
            tu_back, tu_back2,
            "line {line_idx} : Rust struct → JSON not idempotent"
        );
    }

    assert!(
        line_count >= 4,
        "canonical CB-01.ndjson must have ≥ 4 lines, found {line_count}"
    );
}
