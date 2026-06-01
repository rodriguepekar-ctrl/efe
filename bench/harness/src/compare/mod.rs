//! PR #21 § 12 + AC23 — comparison + verdict engine.
//!
//! Closed taxonomy of 6 metrics ; per-metric tolerance check ;
//! 4-level verdict (`pass | warn | fail | no-baseline`).
//!
//! ## PR #27 — G.11 closure (additive, STABILITY § 24.3.6 + § 27.2.5/6/7)
//!
//! [`effective_tolerance`] adds the rich [`EffectiveTolerance`] struct
//! that carries `(value, unit, direction, source, applied_at,
//! override_reason?)`.  The existing scalar `tolerance_used: Option<f64>`
//! on [`MetricResult`] and [`ComparisonRow`] is preserved for backward
//! compatibility ; a follow-up PR will migrate the full pipeline to
//! emit [`MetricVerdictV3`] rows.

pub mod effective_tolerance;
pub mod tolerance;
pub mod verdict;

pub use effective_tolerance::{
    compute_verdict, pretend_aggregator_emit_rows, resolve_effective_tolerance, EffectiveTolerance,
    EnvContext, GpuOverrideEntry, MetricVerdictV3, RunnerIntrospection, ScenarioToleranceEntry,
    ToleranceDirection, ToleranceSource, ToleranceUnit, ToleranceValue,
};
pub use tolerance::{check_metric, MetricResult, MetricStatus};
pub use verdict::{ComparisonReport, ComparisonRow, Verdict};

use std::collections::BTreeMap;

use crate::baseline::Baseline;
use crate::result::ScenarioResult;
use crate::scenario::{Scenario, ToleranceMetric, ToleranceSpec};

/// Compare a fresh scenario result against the committed baseline.
///
/// Returns a full report : per-metric verdict + overall verdict.
/// `no-baseline` is the verdict when the baseline file doesn't exist
/// for this `(chart_version, scenario_id)` pair (AD12).
#[must_use]
pub fn compare(
    scenario: &Scenario,
    result: &ScenarioResult,
    baseline: Option<&Baseline>,
) -> ComparisonReport {
    let Some(b) = baseline else {
        return ComparisonReport::no_baseline(scenario.id.clone(), result.metrics.clone());
    };
    let mut rows: BTreeMap<ToleranceMetric, ComparisonRow> = BTreeMap::new();

    for m in ToleranceMetric::all() {
        if let (Some(actual), Some(baseline_v)) = (result.metrics.get(m), b.metrics.get(m)) {
            let res = check_metric(m, baseline_v, actual, &scenario.tolerance);
            rows.insert(m, ComparisonRow {
                metric: m,
                baseline: Some(baseline_v),
                actual: Some(actual),
                delta_pct: res.delta_pct,
                delta_abs: res.delta_abs,
                status: res.status,
                tolerance_used: res.tolerance_used,
            });
        } else if let Some(actual) = result.metrics.get(m) {
            // Actual present but baseline missing → no compare, treat as pass.
            rows.insert(m, ComparisonRow {
                metric: m,
                baseline: None,
                actual: Some(actual),
                delta_pct: None,
                delta_abs: None,
                status: MetricStatus::NoBaseline,
                tolerance_used: None,
            });
        }
    }

    let overall = rows
        .values()
        .map(|r| Verdict::from(r.status))
        .fold(Verdict::Pass, Verdict::combine);

    ComparisonReport {
        scenario_id: scenario.id.clone(),
        baseline_chart_version: Some(b.chart_version.clone()),
        baseline_git_commit: Some(b.git_commit.clone()),
        rows,
        overall,
    }
}

/// Convenience : resolve a tolerance override from the scenario or
/// fall back to the default in [`crate::tolerance_defaults`].
#[must_use]
pub fn resolve_tolerance(
    spec: &ToleranceSpec,
    metric: ToleranceMetric,
    severity: MetricStatus,
) -> f64 {
    use crate::tolerance_defaults::*;
    match (metric, severity) {
        (ToleranceMetric::P50Ms, MetricStatus::Warn) => P50_WARN_PCT,
        (ToleranceMetric::P50Ms, MetricStatus::Fail) => {
            spec.p50_ms_max_delta_pct.unwrap_or(P50_FAIL_PCT)
        }
        (ToleranceMetric::P95Ms, MetricStatus::Warn) => P95_WARN_PCT,
        (ToleranceMetric::P95Ms, MetricStatus::Fail) => {
            spec.p95_ms_max_delta_pct.unwrap_or(P95_FAIL_PCT)
        }
        (ToleranceMetric::P99Ms, MetricStatus::Warn) => P99_WARN_PCT,
        (ToleranceMetric::P99Ms, MetricStatus::Fail) => {
            spec.p99_ms_max_delta_pct.unwrap_or(P99_FAIL_PCT)
        }
        (ToleranceMetric::ErrorRate, MetricStatus::Warn) => ERROR_RATE_WARN_PP,
        (ToleranceMetric::ErrorRate, MetricStatus::Fail) => {
            spec.error_rate_max_delta_pp.unwrap_or(ERROR_RATE_FAIL_PP)
        }
        (ToleranceMetric::ThroughputQps, MetricStatus::Warn) => THROUGHPUT_WARN_PCT,
        (ToleranceMetric::ThroughputQps, MetricStatus::Fail) => {
            spec.throughput_qps_min_delta_pct.unwrap_or(THROUGHPUT_FAIL_PCT)
        }
        (ToleranceMetric::RecoveryTimeS, MetricStatus::Warn) => RECOVERY_TIME_WARN_S,
        (ToleranceMetric::RecoveryTimeS, MetricStatus::Fail) => {
            spec.recovery_time_s_max_delta_s.unwrap_or(RECOVERY_TIME_FAIL_S)
        }
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::result::ResultMetrics;

    fn make_scenario() -> Scenario {
        Scenario {
            schema_version: 1,
            id: "S01_x".into(),
            description: "test".into(),
            duration: "5m".into(),
            cluster: Default::default(),
            loadgen: crate::scenario::ScenarioLoadgen {
                version: 1,
                name: "t".into(),
                target: serde_yaml::Value::Null,
                pattern: serde_yaml::Value::Null,
                workload: None,
                rate: None,
                termination: None,
                output: None,
                observability: None,
                run: None,
                extra: Default::default(),
            },
            chaos: vec![],
            results: Default::default(),
            tolerance: Default::default(),
        }
    }

    #[test]
    fn no_baseline_yields_no_baseline_verdict() {
        let scen = make_scenario();
        let result = ScenarioResult::with_metrics(
            "S01_x".into(),
            ResultMetrics { p50_ms: Some(8.0), p95_ms: Some(100.0), p99_ms: Some(500.0), error_rate: Some(0.001), throughput_qps: Some(100.0), recovery_time_s: None },
        );
        let report = compare(&scen, &result, None);
        assert!(matches!(report.overall, Verdict::NoBaseline));
    }

    #[test]
    fn pass_when_within_tolerance() {
        let scen = make_scenario();
        let baseline = Baseline {
            schema_version: 1,
            scenario_id: "S01_x".into(),
            harness_version: "0.1.0".into(),
            loadgen_version: "0.1.0".into(),
            chart_version: "0.6.0".into(),
            image_tag: "t".into(),
            git_commit: "abc1234".into(),
            run_id: "r".into(),
            started_at: "2026-05-16T00:00:00Z".into(),
            duration_s: 300.0,
            total_requests: 30000,
            metrics: ResultMetrics {
                p50_ms: Some(8.0),
                p95_ms: Some(100.0),
                p99_ms: Some(500.0),
                error_rate: Some(0.001),
                throughput_qps: Some(100.0),
                recovery_time_s: None,
            },
            chaos_events: vec![],
            verdict: Some(Verdict::Pass.as_str().into()),
        };
        let result = ScenarioResult::with_metrics(
            "S01_x".into(),
            ResultMetrics {
                p50_ms: Some(8.2),  // +2.5%, within 8% warn
                p95_ms: Some(101.0), // +1%, within 12% warn
                p99_ms: Some(515.0), // +3%, within 15% warn
                error_rate: Some(0.0011), // +0.01pp, within 0.2pp warn
                throughput_qps: Some(99.5),
                recovery_time_s: None,
            },
        );
        let report = compare(&scen, &result, Some(&baseline));
        assert!(matches!(report.overall, Verdict::Pass), "got {:?}", report.overall);
    }

    #[test]
    fn fail_on_p95_regression_above_fail_pct() {
        let scen = make_scenario();
        let baseline_metrics = ResultMetrics {
            p50_ms: Some(8.0),
            p95_ms: Some(100.0),
            p99_ms: Some(500.0),
            error_rate: Some(0.001),
            throughput_qps: Some(100.0),
            recovery_time_s: None,
        };
        let baseline = Baseline {
            schema_version: 1,
            scenario_id: "S01_x".into(),
            harness_version: "0.1.0".into(),
            loadgen_version: "0.1.0".into(),
            chart_version: "0.6.0".into(),
            image_tag: "t".into(),
            git_commit: "abc".into(),
            run_id: "r".into(),
            started_at: "0".into(),
            duration_s: 300.0,
            total_requests: 30000,
            metrics: baseline_metrics,
            chaos_events: vec![],
            verdict: None,
        };
        // p95 +50% → exceeds default fail threshold 25%
        let result = ScenarioResult::with_metrics(
            "S01_x".into(),
            ResultMetrics { p95_ms: Some(150.0), ..baseline.metrics.clone() },
        );
        let report = compare(&scen, &result, Some(&baseline));
        assert!(matches!(report.overall, Verdict::Fail), "got {:?}", report.overall);
    }
}
