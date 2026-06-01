//! PR #21 § 12.1 + AC23 — verdict + comparison report.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::compare::tolerance::MetricStatus;
use crate::result::ResultMetrics;
use crate::scenario::ToleranceMetric;

/// 4-level verdict (closed taxonomy — STABILITY_CONTRACT v1.3 § 21).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    Pass,
    Warn,
    Fail,
    NoBaseline,
}

impl Verdict {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Warn => "warn",
            Self::Fail => "fail",
            Self::NoBaseline => "no-baseline",
        }
    }

    /// Combine two verdicts taking the worst (Fail > Warn > Pass > NoBaseline).
    #[must_use]
    pub const fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Self::Fail, _) | (_, Self::Fail) => Self::Fail,
            (Self::Warn, _) | (_, Self::Warn) => Self::Warn,
            (Self::Pass, _) | (_, Self::Pass) => Self::Pass,
            _ => Self::NoBaseline,
        }
    }

    /// Map to the harness exit code.
    #[must_use]
    pub const fn exit_code(self) -> crate::error::ExitCode {
        match self {
            Self::Fail => crate::error::ExitCode::BenchFail,
            _ => crate::error::ExitCode::Success,
        }
    }

    /// Renders the verdict glyph used in PR-comment tables.
    #[must_use]
    pub const fn glyph(self) -> &'static str {
        match self {
            Self::Pass => "✅",
            Self::Warn => "⚠",
            Self::Fail => "❌",
            Self::NoBaseline => "ℹ",
        }
    }
}

impl From<MetricStatus> for Verdict {
    fn from(s: MetricStatus) -> Self {
        match s {
            MetricStatus::Pass => Self::Pass,
            MetricStatus::Warn => Self::Warn,
            MetricStatus::Fail => Self::Fail,
            MetricStatus::NoBaseline => Self::NoBaseline,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonRow {
    pub metric: ToleranceMetric,
    pub baseline: Option<f64>,
    pub actual: Option<f64>,
    pub delta_pct: Option<f64>,
    pub delta_abs: Option<f64>,
    pub status: MetricStatus,
    pub tolerance_used: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonReport {
    pub scenario_id: String,
    pub baseline_chart_version: Option<String>,
    pub baseline_git_commit: Option<String>,
    pub rows: BTreeMap<ToleranceMetric, ComparisonRow>,
    pub overall: Verdict,
}

impl ComparisonReport {
    /// Empty report for the "first run, no baseline yet" case.
    #[must_use]
    pub fn no_baseline(scenario_id: String, metrics: ResultMetrics) -> Self {
        let mut rows = BTreeMap::new();
        for m in ToleranceMetric::all() {
            if let Some(v) = metrics.get(m) {
                rows.insert(m, ComparisonRow {
                    metric: m,
                    baseline: None,
                    actual: Some(v),
                    delta_pct: None,
                    delta_abs: None,
                    status: MetricStatus::NoBaseline,
                    tolerance_used: None,
                });
            }
        }
        Self {
            scenario_id,
            baseline_chart_version: None,
            baseline_git_commit: None,
            rows,
            overall: Verdict::NoBaseline,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_combine_takes_worst() {
        assert_eq!(Verdict::Pass.combine(Verdict::Pass), Verdict::Pass);
        assert_eq!(Verdict::Pass.combine(Verdict::Warn), Verdict::Warn);
        assert_eq!(Verdict::Warn.combine(Verdict::Fail), Verdict::Fail);
        assert_eq!(Verdict::Fail.combine(Verdict::Pass), Verdict::Fail);
        assert_eq!(Verdict::NoBaseline.combine(Verdict::Pass), Verdict::Pass);
    }

    #[test]
    fn verdict_taxonomy_strings_stable() {
        assert_eq!(Verdict::Pass.as_str(), "pass");
        assert_eq!(Verdict::Warn.as_str(), "warn");
        assert_eq!(Verdict::Fail.as_str(), "fail");
        assert_eq!(Verdict::NoBaseline.as_str(), "no-baseline");
    }

    #[test]
    fn d_bg_verdict_rejects_unknown_variant_at_deserialize() {
        // **D-BG** — if a future PR adds a 5th verdict (e.g. "unstable"),
        // the CURRENT harness must reject it loudly rather than
        // silently coerce to a default.  Test pins the rejection.
        let bad = "\"unstable\"";
        let r: Result<Verdict, _> = serde_json::from_str(bad);
        assert!(
            r.is_err(),
            "D-BG : unknown variant must be rejected — got {r:?}"
        );
    }

    #[test]
    fn d_bg_verdict_rejects_wrong_case_at_deserialize() {
        // Serde is configured for kebab-case lowercase ; "Pass" or
        // "PASS" must be rejected (case-sensitive).
        for bad in ["\"Pass\"", "\"PASS\"", "\"no_baseline\""] {
            let r: Result<Verdict, _> = serde_json::from_str(bad);
            assert!(
                r.is_err(),
                "D-BG : '{bad}' must be rejected (case-sensitive)"
            );
        }
    }

    #[test]
    fn d_bg_verdict_accepts_all_four_canonical_variants() {
        for (s, expected) in [
            ("\"pass\"", Verdict::Pass),
            ("\"warn\"", Verdict::Warn),
            ("\"fail\"", Verdict::Fail),
            ("\"no-baseline\"", Verdict::NoBaseline),
        ] {
            let v: Verdict = serde_json::from_str(s).unwrap_or_else(|e| {
                panic!("D-BG : canonical variant {s} must deserialize : {e}")
            });
            assert_eq!(v, expected);
        }
    }

    #[test]
    fn d_ad_serde_kebab_case_matches_as_str_for_every_variant() {
        // **D-AD invariant** — `Verdict::as_str()` and the serde
        // `kebab-case` serialization MUST emit the same string.
        // Without this test, the two paths can drift silently :
        // current.json uses `as_str()` while some future code path
        // might serialize via serde_json directly — frontends would
        // see two different literals for the same verdict.
        for v in [
            Verdict::Pass,
            Verdict::Warn,
            Verdict::Fail,
            Verdict::NoBaseline,
        ] {
            let via_as_str = v.as_str();
            // serde_json emits the variant as a JSON string `"..."` ;
            // trim the quotes to compare with `as_str()`.
            let serde = serde_json::to_string(&v).expect("serializable");
            let serde_unq = serde.trim_matches('"');
            assert_eq!(
                via_as_str, serde_unq,
                "D-AD : as_str() == serde for {v:?} ; got '{via_as_str}' vs '{serde_unq}'"
            );
        }
    }

    #[test]
    fn verdict_exit_codes() {
        assert_eq!(Verdict::Pass.exit_code(), crate::error::ExitCode::Success);
        assert_eq!(Verdict::Warn.exit_code(), crate::error::ExitCode::Success);
        assert_eq!(Verdict::Fail.exit_code(), crate::error::ExitCode::BenchFail);
        assert_eq!(Verdict::NoBaseline.exit_code(), crate::error::ExitCode::Success);
    }
}
