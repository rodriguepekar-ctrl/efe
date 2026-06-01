//! PR #24 § 10.4 — `/api/baselines.json` (published baselines).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::publish::api::API_SCHEMA_VERSION;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselinesJson {
    pub schema_version: u32,
    pub baselines: BTreeMap<String, BaselineSet>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineSet {
    pub released_at: String,
    pub scenarios: Vec<BaselineEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineEntry {
    pub id: String,
    pub url: String,
}

impl BaselinesJson {
    #[must_use]
    pub fn new(baselines: BTreeMap<String, BaselineSet>) -> Self {
        Self {
            schema_version: API_SCHEMA_VERSION,
            baselines,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_version_1() {
        let b = BaselinesJson::new(BTreeMap::new());
        assert_eq!(b.schema_version, 1);
    }

    #[test]
    fn chart_versions_sort_canonically() {
        let mut m = BTreeMap::new();
        m.insert("0.7.0".into(), BaselineSet {
            released_at: "2026-07-01T00:00:00Z".into(),
            scenarios: vec![],
        });
        m.insert("0.6.0".into(), BaselineSet {
            released_at: "2026-04-01T00:00:00Z".into(),
            scenarios: vec![],
        });
        let s = BaselinesJson::new(m);
        let json = serde_json::to_string_pretty(&s).unwrap();
        let i06 = json.find("0.6.0").unwrap();
        let i07 = json.find("0.7.0").unwrap();
        assert!(i06 < i07);
    }
}
