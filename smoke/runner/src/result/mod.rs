//! PR #23 § 9 — smoke result aggregator + JSON output + markdown PR
//! comment + gh-pages SVG badge.

pub mod badge;
pub mod json;
pub mod markdown;

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::checks::{CheckResult, CheckVerdict, SmokeCheckId};
use crate::error::{SmokeError, SmokeResult};

/// Canonical smoke result payload (frozen schema v1 — STABILITY v1.5 § 23.5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmokeReport {
    pub schema_version: u32,
    pub chart_version: String,
    pub git_sha: String,
    pub harness_version: String,
    pub run_id: String,
    pub started_at: String,
    pub completed_at: String,
    pub duration_s: f64,
    pub overall_verdict: CheckVerdict,
    /// **Sorted** by SmokeCheckId (canonical form — AD24).
    pub checks: BTreeMap<String, CheckResult>,
    pub stability_contract_version: String,
}

impl SmokeReport {
    /// Construct a canonical-form report.
    #[must_use]
    pub fn build(
        run_id: String,
        chart_version: String,
        git_sha: String,
        duration: Duration,
        overall_verdict: CheckVerdict,
        results: Vec<CheckResult>,
    ) -> Self {
        let now = chrono::Utc::now();
        let mut checks = BTreeMap::new();
        for r in &results {
            checks.insert(r.id.short().to_string(), r.clone());
        }
        Self {
            schema_version: crate::SMOKE_RESULT_SCHEMA_VERSION,
            chart_version,
            git_sha,
            harness_version: crate::build::VERSION.into(),
            run_id,
            started_at: (now - chrono::Duration::from_std(duration).unwrap_or_default())
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            completed_at: now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            duration_s: duration.as_secs_f64(),
            overall_verdict,
            checks,
            stability_contract_version: "1.5".into(),
        }
    }

    /// Number of failing checks.
    #[must_use]
    pub fn failed_count(&self) -> usize {
        self.checks
            .values()
            .filter(|r| matches!(r.verdict, CheckVerdict::Fail))
            .count()
    }

    /// Identify the failing checks (stable order).
    #[must_use]
    pub fn failing_ids(&self) -> Vec<SmokeCheckId> {
        let mut out = Vec::new();
        for id in SmokeCheckId::all() {
            if let Some(r) = self.checks.get(id.short()) {
                if matches!(r.verdict, CheckVerdict::Fail) {
                    out.push(id);
                }
            }
        }
        out
    }
}

/// Write `report` to `path` as pretty-printed JSON (canonical form).
pub fn write_report_json(report: &SmokeReport, path: &Path) -> SmokeResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| SmokeError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    }
    let body = serde_json::to_vec_pretty(report)?;
    std::fs::write(path, body).map_err(|source| SmokeError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Read a smoke result JSON from disk + validate schema version.
pub fn read_report_json(path: &Path) -> SmokeResult<SmokeReport> {
    let raw = std::fs::read(path).map_err(|source| SmokeError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let r: SmokeReport = serde_json::from_slice(&raw)?;
    if r.schema_version != crate::SMOKE_RESULT_SCHEMA_VERSION {
        return Err(SmokeError::Config(format!(
            "smoke-result.json schema_version {} not supported (expected {})",
            r.schema_version,
            crate::SMOKE_RESULT_SCHEMA_VERSION
        )));
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_pass(id: SmokeCheckId) -> CheckResult {
        CheckResult::pass(id, Duration::from_secs(5), serde_json::json!({}))
    }

    fn sample_fail(id: SmokeCheckId) -> CheckResult {
        CheckResult::fail(
            id,
            Duration::from_secs(5),
            "boom".into(),
            serde_json::json!({"err": true}),
        )
    }

    #[test]
    fn build_report_keys_in_canonical_order() {
        let results = vec![
            sample_pass(SmokeCheckId::S2),
            sample_pass(SmokeCheckId::S1),
            sample_pass(SmokeCheckId::S5),
        ];
        let r = SmokeReport::build(
            "r".into(),
            "0.6.0".into(),
            "abc".into(),
            Duration::from_secs(120),
            CheckVerdict::Pass,
            results,
        );
        // BTreeMap → keys in alphanumeric order.
        let keys: Vec<_> = r.checks.keys().cloned().collect();
        assert_eq!(keys, vec!["S1", "S2", "S5"]);
    }

    #[test]
    fn failed_count_sums_correctly() {
        let r = SmokeReport::build(
            "r".into(),
            "0.6.0".into(),
            "abc".into(),
            Duration::from_secs(10),
            CheckVerdict::Fail,
            vec![sample_pass(SmokeCheckId::S1), sample_fail(SmokeCheckId::S2)],
        );
        assert_eq!(r.failed_count(), 1);
    }

    #[test]
    fn failing_ids_returns_stable_order() {
        let r = SmokeReport::build(
            "r".into(),
            "0.6.0".into(),
            "abc".into(),
            Duration::from_secs(10),
            CheckVerdict::Fail,
            vec![
                sample_fail(SmokeCheckId::S4),
                sample_fail(SmokeCheckId::S1),
                sample_pass(SmokeCheckId::S2),
            ],
        );
        assert_eq!(r.failing_ids(), vec![SmokeCheckId::S1, SmokeCheckId::S4]);
    }

    #[test]
    fn write_then_read_roundtrips() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("smoke.json");
        let r = SmokeReport::build(
            "r".into(),
            "0.6.0".into(),
            "abc".into(),
            Duration::from_secs(10),
            CheckVerdict::Pass,
            vec![sample_pass(SmokeCheckId::S1)],
        );
        write_report_json(&r, &path).unwrap();
        let back = read_report_json(&path).unwrap();
        assert_eq!(back.run_id, "r");
        assert_eq!(back.checks.len(), 1);
    }

    #[test]
    fn read_rejects_wrong_schema_version() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("bad.json");
        std::fs::write(&path, r#"{"schema_version":99,"chart_version":"x","git_sha":"y","harness_version":"z","run_id":"r","started_at":"t","completed_at":"t","duration_s":0,"overall_verdict":"pass","checks":{},"stability_contract_version":"1.5"}"#).unwrap();
        let e = read_report_json(&path).unwrap_err();
        assert!(matches!(e, SmokeError::Config(_)));
    }
}
