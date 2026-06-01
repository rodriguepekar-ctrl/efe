//! PR #24 § 10.5 — `/api/version.json` (build metadata).

use serde::{Deserialize, Serialize};

use crate::publish::api::API_SCHEMA_VERSION;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionJson {
    pub schema_version: u32,
    pub harness_version: String,
    pub chart_version: String,
    pub git_sha: String,
    pub stability_contract: String,
    pub build_date: String,
    pub published_at: String,
}

impl VersionJson {
    #[must_use]
    pub fn build(chart_version: String) -> Self {
        Self {
            schema_version: API_SCHEMA_VERSION,
            harness_version: crate::build::VERSION.into(),
            chart_version,
            git_sha: crate::build::GIT_SHA.into(),
            stability_contract: "1.6".into(),
            build_date: crate::build::BUILD_DATE.into(),
            published_at: chrono::Utc::now()
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_version_1() {
        let v = VersionJson::build("0.6.0".into());
        assert_eq!(v.schema_version, 1);
    }

    #[test]
    fn stability_contract_string_v1_6() {
        let v = VersionJson::build("0.6.0".into());
        assert_eq!(v.stability_contract, "1.6");
    }

    #[test]
    fn carries_harness_version_from_build_module() {
        let v = VersionJson::build("0.6.0".into());
        // Validate non-empty (full value is build-time dependent).
        assert!(!v.harness_version.is_empty());
    }

    #[test]
    fn d_ah_published_at_is_rfc3339_utc_seconds_resolution() {
        // **D-AH** — published_at MUST be RFC 3339 UTC at second
        // resolution.  Downstream consumers parse it as a timestamp ;
        // a stray sub-second component or local-tz suffix breaks them.
        let v = VersionJson::build("0.6.0".into());
        let pa = &v.published_at;
        // RFC 3339 ends with 'Z' for UTC at seconds resolution.
        assert!(pa.ends_with('Z'), "D-AH : must end with 'Z' (UTC) — got '{pa}'");
        // Parseable as a chrono timestamp.
        let parsed = chrono::DateTime::parse_from_rfc3339(pa)
            .unwrap_or_else(|e| panic!("D-AH : not RFC 3339 : {e} ; raw='{pa}'"));
        // Seconds-only : no fractional component.
        assert!(
            !pa.contains('.'),
            "D-AH : seconds resolution required, got fractional : '{pa}'"
        );
        // UTC offset = 0.
        assert_eq!(parsed.offset().local_minus_utc(), 0);
    }

    #[test]
    fn d_ah_published_at_monotonic_with_physical_time() {
        // **D-AH** — two successive `build()` calls cannot produce a
        // published_at that goes BACKWARDS.  Sleep across a second
        // boundary to bypass the seconds-truncation.
        let a = VersionJson::build("0.6.0".into());
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let b = VersionJson::build("0.6.0".into());
        assert!(
            b.published_at >= a.published_at,
            "D-AH : timestamps regressed ; a='{}', b='{}'",
            a.published_at,
            b.published_at
        );
    }

    #[test]
    fn forward_compat_unknown_field() {
        let body = r#"{
            "schema_version": 1,
            "harness_version": "0.2.0",
            "chart_version": "0.6.0",
            "git_sha": "abc",
            "stability_contract": "1.6",
            "build_date": "2026-05-16",
            "published_at": "2026-05-16T00:00:00Z",
            "future_field": "ignored"
        }"#;
        let v: VersionJson = serde_json::from_str(body).unwrap();
        assert_eq!(v.chart_version, "0.6.0");
    }
}
