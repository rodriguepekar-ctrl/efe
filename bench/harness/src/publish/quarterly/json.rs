//! PR #24 § 12 — quarterly JSON sidecar (alongside the signed PDF).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuarterlyJson {
    pub schema_version: u32,
    pub chart_version: String,
    pub quarter: String,
    pub computed_at: String,
    pub months: Vec<String>,
    pub scenarios: BTreeMap<String, QuarterlyScenarioEntry>,
    pub stability_contract_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuarterlyScenarioEntry {
    pub days_observed: u32,
    pub days_pass: u32,
    pub days_fail: u32,
    pub days_warn: u32,
    pub days_no_data: u32,
    pub verdict: String,
}

impl QuarterlyJson {
    #[must_use]
    pub fn build(
        chart_version: String,
        quarter: String,
        months: Vec<String>,
        scenarios: BTreeMap<String, QuarterlyScenarioEntry>,
    ) -> Self {
        Self {
            schema_version: 1,
            chart_version,
            quarter,
            computed_at: chrono::Utc::now()
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            months,
            scenarios,
            stability_contract_version: "1.6".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quarterly_json_carries_v1_6() {
        let q = QuarterlyJson::build(
            "0.6.0".into(),
            "Q2-2026".into(),
            vec!["2026-04".into(), "2026-05".into(), "2026-06".into()],
            BTreeMap::new(),
        );
        assert_eq!(q.schema_version, 1);
        assert_eq!(q.stability_contract_version, "1.6");
        assert_eq!(q.months.len(), 3);
    }

    #[test]
    fn scenarios_canonical_order() {
        let mut s = BTreeMap::new();
        s.insert("S03".into(), QuarterlyScenarioEntry {
            days_observed: 90, days_pass: 88, days_fail: 1, days_warn: 1, days_no_data: 0,
            verdict: "pass".into(),
        });
        s.insert("S01".into(), QuarterlyScenarioEntry {
            days_observed: 90, days_pass: 90, days_fail: 0, days_warn: 0, days_no_data: 0,
            verdict: "pass".into(),
        });
        let q = QuarterlyJson::build("0.6.0".into(), "Q2-2026".into(), vec![], s);
        let json = serde_json::to_string_pretty(&q).unwrap();
        let i01 = json.find("\"S01\"").unwrap();
        let i03 = json.find("\"S03\"").unwrap();
        assert!(i01 < i03);
    }
}
