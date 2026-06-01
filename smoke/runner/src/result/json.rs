//! PR #23 § 9 — JSON output helpers (canonical form).
//!
//! The write/read primitives live in `super::*` ; this module exposes
//! schema-validation helpers for the `efe-smoke validate` subcommand.

use crate::checks::SmokeCheckId;
use crate::error::{SmokeError, SmokeResult};
use crate::result::SmokeReport;

/// Validate a `SmokeReport` carries every required v1.5 field with
/// reasonable values.  Used by `efe-smoke validate`.
pub fn validate_schema(report: &SmokeReport) -> SmokeResult<()> {
    if report.schema_version != crate::SMOKE_RESULT_SCHEMA_VERSION {
        return Err(SmokeError::Config(format!(
            "schema_version {} not supported (expected {})",
            report.schema_version,
            crate::SMOKE_RESULT_SCHEMA_VERSION
        )));
    }
    if report.chart_version.is_empty() {
        return Err(SmokeError::Config("chart_version is empty".into()));
    }
    if report.run_id.len() < 8 {
        return Err(SmokeError::Config("run_id too short (< 8 chars)".into()));
    }
    if report.stability_contract_version != "1.5" {
        return Err(SmokeError::Config(format!(
            "stability_contract_version '{}' != '1.5'",
            report.stability_contract_version
        )));
    }
    // At most 9 checks (closed taxonomy — AD1).
    if report.checks.len() > 9 {
        return Err(SmokeError::Config(format!(
            "checks map has {} entries, max 9 (AD1)",
            report.checks.len()
        )));
    }
    // Every key must be a valid SmokeCheckId short form.
    for key in report.checks.keys() {
        if SmokeCheckId::parse(key).is_none() {
            return Err(SmokeError::Config(format!(
                "unknown check id '{key}' in report (AD1 — closed taxonomy)"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::{CheckResult, CheckVerdict, SmokeCheckId};
    use std::time::Duration;

    fn valid_report() -> SmokeReport {
        SmokeReport::build(
            "01J3F7XGGYM5NZ0CWY3Q7RH3JE".into(),
            "0.6.0".into(),
            "abc1234".into(),
            Duration::from_secs(120),
            CheckVerdict::Pass,
            SmokeCheckId::all()
                .iter()
                .map(|id| CheckResult::pass(*id, Duration::from_secs(5), serde_json::json!({})))
                .collect(),
        )
    }

    #[test]
    fn valid_report_passes_validation() {
        validate_schema(&valid_report()).unwrap();
    }

    #[test]
    fn rejects_bad_schema_version() {
        let mut r = valid_report();
        r.schema_version = 99;
        let e = validate_schema(&r).unwrap_err();
        assert!(matches!(e, SmokeError::Config(_)));
    }

    #[test]
    fn rejects_empty_chart_version() {
        let mut r = valid_report();
        r.chart_version = String::new();
        let e = validate_schema(&r).unwrap_err();
        assert!(matches!(e, SmokeError::Config(_)));
    }

    #[test]
    fn rejects_unknown_check_id() {
        let mut r = valid_report();
        r.checks.insert(
            "S99".into(),
            CheckResult::pass(SmokeCheckId::S1, Duration::from_secs(1), serde_json::json!({})),
        );
        let e = validate_schema(&r).unwrap_err();
        assert!(matches!(e, SmokeError::Config(_)));
    }

    #[test]
    fn rejects_wrong_stability_version() {
        let mut r = valid_report();
        r.stability_contract_version = "9.9".into();
        let e = validate_schema(&r).unwrap_err();
        assert!(matches!(e, SmokeError::Config(_)));
    }
}
