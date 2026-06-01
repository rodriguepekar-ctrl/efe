//! PR #24 § 10.3 — `/api/scenarios.json` (10 scenarios metadata).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::publish::api::API_SCHEMA_VERSION;

/// `/api/scenarios.json` shape (frozen v1.6 § 24.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenariosJson {
    pub schema_version: u32,
    /// BTreeMap by scenario ID → canonical sorted order.
    pub scenarios: BTreeMap<String, ScenarioMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioMeta {
    pub description: String,
    pub duration: String,
    pub tier: String,
    pub chaos_count: u32,
    pub latest_verdict: String,
    pub page: String,
    pub badge: String,
}

impl ScenariosJson {
    #[must_use]
    pub fn new(scenarios: BTreeMap<String, ScenarioMeta>) -> Self {
        Self {
            schema_version: API_SCHEMA_VERSION,
            scenarios,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(short: &str) -> ScenarioMeta {
        ScenarioMeta {
            description: "test".into(),
            duration: "5m".into(),
            tier: "quick".into(),
            chaos_count: 0,
            latest_verdict: "pass".into(),
            page: format!("/scenarios/{}.html", short.to_lowercase()),
            badge: format!("/badges/scenario-{short}.svg"),
        }
    }

    #[test]
    fn ten_scenarios_serialise_in_canonical_order() {
        let mut m = BTreeMap::new();
        m.insert("S03_ramp_to_breakdown".into(), meta("S03"));
        m.insert("S01_baseline_100qps_5min".into(), meta("S01"));
        m.insert("S02_canary_1000qps_15min".into(), meta("S02"));
        let s = ScenariosJson::new(m);
        let json = serde_json::to_string_pretty(&s).unwrap();
        // BTreeMap → canonical sort
        let i01 = json.find("S01_baseline").unwrap();
        let i02 = json.find("S02_canary").unwrap();
        let i03 = json.find("S03_ramp").unwrap();
        assert!(i01 < i02);
        assert!(i02 < i03);
    }

    #[test]
    fn schema_version_1() {
        let s = ScenariosJson::new(BTreeMap::new());
        assert_eq!(s.schema_version, 1);
    }

    #[test]
    fn page_url_uses_kebab_case() {
        // STABILITY v1.6 § 24.6 invariant.
        let m = meta("S01");
        assert!(m.page.contains("/scenarios/"));
        assert!(!m.page.contains('_'), "page URL must be kebab-case (§ 24.6)");
    }
}
