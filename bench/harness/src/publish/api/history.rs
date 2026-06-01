//! PR #24 § 10.2 + AD19 — `/api/history.json` with tamper-evidence.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::publish::api::API_SCHEMA_VERSION;

/// `/api/history.json` shape (frozen v1.6 § 24.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryJson {
    pub schema_version: u32,
    pub entries: Vec<HistoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// `YYYY-MM-DD`.
    pub date: String,
    /// Stable URL e.g. `/history/2026-05-16.json`.
    pub url: String,
    /// SHA256 of the file content (AD19 — tamper-evidence).
    pub sha256: String,
    /// Aggregate verdict for that day's run.
    pub overall_verdict: String,
    /// AD52 — when this entry is an ERRATUM for an earlier date.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub replaces: Option<String>,
}

impl HistoryJson {
    #[must_use]
    pub fn new(entries: Vec<HistoryEntry>) -> Self {
        Self {
            schema_version: API_SCHEMA_VERSION,
            entries,
        }
    }
}

/// Compute SHA256 of a file body — used to populate
/// `HistoryEntry.sha256`.
#[must_use]
pub fn sha256_of(body: &[u8]) -> String {
    hex::encode(Sha256::digest(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_version_1() {
        let h = HistoryJson::new(vec![]);
        assert_eq!(h.schema_version, 1);
    }

    #[test]
    fn sha256_matches_canonical_for_hello() {
        let h = sha256_of(b"hello");
        // Pre-computed expected sha256 of "hello".
        assert_eq!(h, "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
    }

    #[test]
    fn replaces_omitted_when_none() {
        let e = HistoryEntry {
            date: "2026-05-16".into(),
            url: "/history/2026-05-16.json".into(),
            sha256: "abc".into(),
            overall_verdict: "pass".into(),
            replaces: None,
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(!json.contains("replaces"));
    }

    #[test]
    fn replaces_included_for_errata() {
        let e = HistoryEntry {
            date: "2026-05-15-rev2".into(),
            url: "/history/2026-05-15-rev2.json".into(),
            sha256: "abc".into(),
            overall_verdict: "pass".into(),
            replaces: Some("2026-05-15".into()),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"replaces\":\"2026-05-15\""));
    }

    #[test]
    fn forward_compat_unknown_field_ignored() {
        let body = r#"{
            "schema_version": 1,
            "entries": [],
            "future_field": 42
        }"#;
        let h: HistoryJson = serde_json::from_str(body).unwrap();
        assert_eq!(h.entries.len(), 0);
    }
}
