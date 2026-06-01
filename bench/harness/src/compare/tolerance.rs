//! PR #21 § 12.2 + Appendix D — per-metric tolerance check.

use serde::{Deserialize, Serialize};

use crate::scenario::{ToleranceMetric, ToleranceSpec};

/// Verdict for a single metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MetricStatus {
    Pass,
    Warn,
    Fail,
    NoBaseline,
}

/// Per-metric comparison result.
#[derive(Debug, Clone)]
pub struct MetricResult {
    pub status: MetricStatus,
    pub delta_pct: Option<f64>,
    pub delta_abs: Option<f64>,
    pub tolerance_used: Option<f64>,
}

/// Check one metric against the baseline + tolerance spec.
///
/// Directionality matters : latency / error_rate are *lower-is-better*,
/// throughput is *higher-is-better*.
#[must_use]
pub fn check_metric(
    metric: ToleranceMetric,
    baseline: f64,
    actual: f64,
    spec: &ToleranceSpec,
) -> MetricResult {
    use crate::compare::resolve_tolerance;
    use crate::tolerance_defaults::*;

    // Compute deltas.
    let delta_abs = actual - baseline;
    let delta_pct = if baseline.abs() > f64::EPSILON {
        Some((delta_abs / baseline) * 100.0)
    } else {
        None
    };

    // Per-metric logic.
    let status = match metric {
        // ── Percent-based (lower-is-better) ──
        ToleranceMetric::P50Ms | ToleranceMetric::P95Ms | ToleranceMetric::P99Ms => {
            let warn_pct = match metric {
                ToleranceMetric::P50Ms => P50_WARN_PCT,
                ToleranceMetric::P95Ms => P95_WARN_PCT,
                ToleranceMetric::P99Ms => P99_WARN_PCT,
                _ => unreachable!(),
            };
            let fail_pct = resolve_tolerance(spec, metric, MetricStatus::Fail);
            classify_pct(delta_pct, warn_pct, fail_pct)
        }
        // ── Percentage-point-based (lower-is-better) ──
        ToleranceMetric::ErrorRate => {
            let warn_pp = ERROR_RATE_WARN_PP / 100.0; // pp → fraction
            let fail_pp = resolve_tolerance(spec, metric, MetricStatus::Fail) / 100.0;
            classify_abs(delta_abs, warn_pp, fail_pp)
        }
        // ── Percent-based (higher-is-better) ──
        ToleranceMetric::ThroughputQps => {
            // Verdict on negative delta (regression = drop).
            let warn_pct = -THROUGHPUT_WARN_PCT.abs();
            let fail_pct = -resolve_tolerance(spec, metric, MetricStatus::Fail).abs();
            classify_pct_higher_is_better(delta_pct, warn_pct, fail_pct)
        }
        // ── Absolute seconds (lower-is-better) ──
        ToleranceMetric::RecoveryTimeS => {
            let warn_s = RECOVERY_TIME_WARN_S;
            let fail_s = resolve_tolerance(spec, metric, MetricStatus::Fail);
            classify_abs(delta_abs, warn_s, fail_s)
        }
    };

    let tolerance_used = Some(resolve_tolerance(spec, metric, MetricStatus::Fail));

    MetricResult {
        status,
        delta_pct,
        delta_abs: Some(delta_abs),
        tolerance_used,
    }
}

/// Classify a relative delta (lower-is-better : higher delta is worse).
fn classify_pct(delta_pct: Option<f64>, warn_pct: f64, fail_pct: f64) -> MetricStatus {
    match delta_pct {
        Some(d) if d > fail_pct => MetricStatus::Fail,
        Some(d) if d > warn_pct => MetricStatus::Warn,
        _ => MetricStatus::Pass,
    }
}

/// Classify a relative delta (higher-is-better : lower delta is worse).
fn classify_pct_higher_is_better(
    delta_pct: Option<f64>,
    warn_pct: f64,
    fail_pct: f64,
) -> MetricStatus {
    match delta_pct {
        Some(d) if d < fail_pct => MetricStatus::Fail,
        Some(d) if d < warn_pct => MetricStatus::Warn,
        _ => MetricStatus::Pass,
    }
}

/// Classify an absolute delta (lower-is-better).
fn classify_abs(delta_abs: f64, warn: f64, fail: f64) -> MetricStatus {
    if delta_abs > fail {
        MetricStatus::Fail
    } else if delta_abs > warn {
        MetricStatus::Warn
    } else {
        MetricStatus::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p95_50pct_above_baseline_fails() {
        let r = check_metric(
            ToleranceMetric::P95Ms,
            100.0, 150.0,
            &ToleranceSpec::default(),
        );
        assert_eq!(r.status, MetricStatus::Fail);
    }

    #[test]
    fn p95_5pct_above_baseline_passes() {
        let r = check_metric(
            ToleranceMetric::P95Ms,
            100.0, 105.0,
            &ToleranceSpec::default(),
        );
        assert_eq!(r.status, MetricStatus::Pass);
    }

    #[test]
    fn p95_15pct_above_baseline_warns() {
        let r = check_metric(
            ToleranceMetric::P95Ms,
            100.0, 115.0,
            &ToleranceSpec::default(),
        );
        assert_eq!(r.status, MetricStatus::Warn);
    }

    #[test]
    fn throughput_drop_of_30pct_fails() {
        let r = check_metric(
            ToleranceMetric::ThroughputQps,
            100.0, 70.0,
            &ToleranceSpec::default(),
        );
        assert_eq!(r.status, MetricStatus::Fail);
    }

    #[test]
    fn throughput_increase_passes() {
        let r = check_metric(
            ToleranceMetric::ThroughputQps,
            100.0, 110.0,
            &ToleranceSpec::default(),
        );
        assert_eq!(r.status, MetricStatus::Pass);
    }

    #[test]
    fn error_rate_2pp_above_fails() {
        // baseline 0.001 (0.1%) → actual 0.021 (2.1%) → +2pp
        let r = check_metric(
            ToleranceMetric::ErrorRate,
            0.001, 0.021,
            &ToleranceSpec::default(),
        );
        assert_eq!(r.status, MetricStatus::Fail);
    }

    #[test]
    fn recovery_time_within_threshold_passes() {
        let r = check_metric(
            ToleranceMetric::RecoveryTimeS,
            10.0, 13.0,
            &ToleranceSpec::default(),
        );
        assert_eq!(r.status, MetricStatus::Pass);
    }

    #[test]
    fn scenario_override_relaxes_threshold() {
        let spec = ToleranceSpec {
            p95_ms_max_delta_pct: Some(50.0),
            ..Default::default()
        };
        let r = check_metric(ToleranceMetric::P95Ms, 100.0, 140.0, &spec);
        // 40% delta is still under the relaxed 50% fail threshold.
        assert_ne!(r.status, MetricStatus::Fail);
    }
}
