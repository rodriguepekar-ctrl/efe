//! PR #24 § 10.1 — `/api/current.json` builder.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::compare::Verdict;
use crate::publish::api::API_SCHEMA_VERSION;

/// Canonical shape of `/api/current.json` (frozen v1.6 § 24.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurrentJson {
    pub schema_version: u32,
    pub as_of: String,
    pub chart_version: String,
    pub harness_version: String,
    pub run_id: String,
    pub git_sha: String,
    pub overall_verdict: String,
    /// BTreeMap so keys serialize in canonical (sorted) order.
    pub scenarios: BTreeMap<String, CurrentScenarioEntry>,
    pub stability_contract_version: String,
    /// **Optional** PR #22 SLO snapshot — only present when
    /// `--embed-slos` is passed (AD34).  Forward-compatible : readers
    /// ignore the field if absent.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub slo_snapshot: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurrentScenarioEntry {
    pub verdict: String,
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub p99_ms: Option<f64>,
    pub error_rate: Option<f64>,
    pub throughput_qps: Option<f64>,
    pub tolerance_used: BTreeMap<String, f64>,
}

impl CurrentJson {
    /// Build a canonical `current.json` payload.
    #[must_use]
    pub fn build(
        chart_version: String,
        run_id: String,
        git_sha: String,
        overall_verdict: Verdict,
        scenarios: BTreeMap<String, CurrentScenarioEntry>,
        slo_snapshot: Option<serde_json::Value>,
    ) -> Self {
        Self {
            schema_version: API_SCHEMA_VERSION,
            as_of: chrono::Utc::now()
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            chart_version,
            harness_version: crate::build::VERSION.into(),
            run_id,
            git_sha,
            overall_verdict: overall_verdict.as_str().into(),
            scenarios,
            stability_contract_version: "1.6".into(),
            slo_snapshot,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn sample_entry() -> CurrentScenarioEntry {
        let mut tolerance = BTreeMap::new();
        tolerance.insert("p95_ms_max_delta_pct".into(), 10.0);
        CurrentScenarioEntry {
            verdict: "pass".into(),
            p50_ms: Some(4.1),
            p95_ms: Some(8.2),
            p99_ms: Some(12.1),
            error_rate: Some(0.0001),
            throughput_qps: Some(99.8),
            tolerance_used: tolerance,
        }
    }

    #[test]
    fn build_includes_schema_version_1() {
        let c = CurrentJson::build(
            "0.6.0".into(),
            "01J3F".into(),
            "abc".into(),
            Verdict::Pass,
            BTreeMap::new(),
            None,
        );
        assert_eq!(c.schema_version, 1);
        assert_eq!(c.stability_contract_version, "1.6");
    }

    #[test]
    fn serialize_keys_sorted() {
        let mut scen = BTreeMap::new();
        // Insert out of order — BTreeMap re-sorts.
        scen.insert("S04_chaos_participant_loss".into(), sample_entry());
        scen.insert("S01_baseline_100qps_5min".into(), sample_entry());
        let c = CurrentJson::build("0.6.0".into(), "r".into(), "abc".into(), Verdict::Pass, scen, None);
        let json = serde_json::to_string_pretty(&c).unwrap();
        let idx_s01 = json.find("S01_baseline").unwrap();
        let idx_s04 = json.find("S04_chaos").unwrap();
        assert!(idx_s01 < idx_s04, "scenario keys must be alphabetically sorted");
    }

    #[test]
    fn slo_snapshot_omitted_when_none() {
        let c = CurrentJson::build("0.6.0".into(), "r".into(), "a".into(), Verdict::Pass, BTreeMap::new(), None);
        let json = serde_json::to_string(&c).unwrap();
        assert!(!json.contains("slo_snapshot"));
    }

    #[test]
    fn slo_snapshot_included_when_some() {
        let snap = serde_json::json!({"slos": {"L2_AVAILABILITY": {"verdict": "pass"}}});
        let c = CurrentJson::build("0.6.0".into(), "r".into(), "a".into(), Verdict::Pass, BTreeMap::new(), Some(snap));
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("slo_snapshot"));
        assert!(json.contains("L2_AVAILABILITY"));
    }

    #[test]
    fn forward_compat_unknown_fields_ignored() {
        // AD16 — readers must ignore unknown fields.
        let body = r#"{
            "schema_version": 1,
            "as_of": "2026-05-16T16:00:00Z",
            "chart_version": "0.6.0",
            "harness_version": "0.2.0",
            "run_id": "r",
            "git_sha": "a",
            "overall_verdict": "pass",
            "scenarios": {},
            "stability_contract_version": "1.6",
            "future_field_v2": "ignored"
        }"#;
        let parsed: CurrentJson = serde_json::from_str(body).expect("unknown field tolerated");
        assert_eq!(parsed.run_id, "r");
    }
}
