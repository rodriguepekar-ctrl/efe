//! PR #23 § 4.6 — gate decision logic (AD30 – AD33).
//!
//! `efe-smoke gate --input <smoke.json>` reads a result, applies the
//! merge-blocking criterion (overall_verdict=fail ⇒ exit 1), and
//! supports the explicit `--allow-skip` override.

use std::path::Path;

use crate::checks::CheckVerdict;
use crate::error::{ExitCode, SmokeError, SmokeResult};
use crate::result::read_report_json;

pub struct GateInput<'a> {
    pub report_path: &'a Path,
    pub allow_skip: bool,
    pub reason: Option<&'a str>,
}

pub fn decide(input: GateInput<'_>) -> SmokeResult<ExitCode> {
    // AD33 — `--allow-skip` requires a reason ; CI workflows enforce
    // the same constraint via shell scripting.
    if input.allow_skip {
        let reason = input
            .reason
            .filter(|r| !r.trim().is_empty())
            .ok_or_else(|| {
                SmokeError::Config(
                    "--allow-skip requires --reason <TEXT> (AD33 — gate-bypass audit)".into(),
                )
            })?;
        tracing::warn!(reason = %reason, "smoke gate BYPASSED by operator (AD32) ; reason logged for audit");
        return Ok(ExitCode::Success);
    }

    let report = read_report_json(input.report_path)?;
    crate::result::json::validate_schema(&report)?;
    match report.overall_verdict {
        CheckVerdict::Pass => Ok(ExitCode::Success),
        CheckVerdict::Fail => Err(SmokeError::SmokeVerdict {
            failed_count: report.failed_count(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::{CheckResult, SmokeCheckId};
    use crate::result::{write_report_json, SmokeReport};
    use std::time::Duration;

    fn write_pass(tmp: &tempfile::TempDir) -> std::path::PathBuf {
        let path = tmp.path().join("pass.json");
        let r = SmokeReport::build(
            "01J3F7XGGYM5NZ0CWY3Q7RH3JE".into(),
            "0.6.0".into(),
            "abc".into(),
            Duration::from_secs(10),
            CheckVerdict::Pass,
            SmokeCheckId::all()
                .iter()
                .map(|id| CheckResult::pass(*id, Duration::from_secs(1), serde_json::json!({})))
                .collect(),
        );
        write_report_json(&r, &path).unwrap();
        path
    }

    fn write_fail(tmp: &tempfile::TempDir) -> std::path::PathBuf {
        let path = tmp.path().join("fail.json");
        let mut results: Vec<CheckResult> = SmokeCheckId::all()
            .iter()
            .map(|id| CheckResult::pass(*id, Duration::from_secs(1), serde_json::json!({})))
            .collect();
        results[3] = CheckResult::fail(
            SmokeCheckId::S4,
            Duration::from_secs(22),
            "2 dropped".into(),
            serde_json::json!({}),
        );
        let r = SmokeReport::build(
            "01J3F7XGGYM5NZ0CWY3Q7RH3JE".into(),
            "0.6.0".into(),
            "abc".into(),
            Duration::from_secs(10),
            CheckVerdict::Fail,
            results,
        );
        write_report_json(&r, &path).unwrap();
        path
    }

    #[test]
    fn pass_report_exits_success() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_pass(&tmp);
        let rc = decide(GateInput {
            report_path: &path,
            allow_skip: false,
            reason: None,
        })
        .unwrap();
        assert_eq!(rc, ExitCode::Success);
    }

    #[test]
    fn fail_report_returns_smoke_fail() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_fail(&tmp);
        let e = decide(GateInput {
            report_path: &path,
            allow_skip: false,
            reason: None,
        })
        .unwrap_err();
        assert_eq!(e.exit_code(), ExitCode::SmokeFail);
    }

    #[test]
    fn allow_skip_without_reason_is_config_error() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_fail(&tmp);
        let e = decide(GateInput {
            report_path: &path,
            allow_skip: true,
            reason: None,
        })
        .unwrap_err();
        assert_eq!(e.exit_code(), ExitCode::Config);
    }

    #[test]
    fn allow_skip_with_reason_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_fail(&tmp);
        let rc = decide(GateInput {
            report_path: &path,
            allow_skip: true,
            reason: Some("emergency unblock — incident I-2026-05-16"),
        })
        .unwrap();
        assert_eq!(rc, ExitCode::Success);
    }

    #[test]
    fn allow_skip_with_empty_reason_is_config_error() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_fail(&tmp);
        let e = decide(GateInput {
            report_path: &path,
            allow_skip: true,
            reason: Some("   "),
        })
        .unwrap_err();
        assert_eq!(e.exit_code(), ExitCode::Config);
    }
}
