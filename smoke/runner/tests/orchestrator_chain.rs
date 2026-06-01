//! PR #23 § 25 — orchestrator + budget enforcement integration tests.

use std::path::PathBuf;
use std::time::Duration;

use efe_smoke::checks::{CheckVerdict, SmokeCheckId};
use efe_smoke::orchestrator::{SmokeConfig, SmokeOrchestrator};

fn mock_cfg() -> SmokeConfig {
    let mut c = SmokeConfig::defaults("0.6.0".into());
    c.mock = true;
    c
}

#[tokio::test]
async fn mock_full_run_emits_9_results_all_pass() {
    let cfg = mock_cfg();
    let o = SmokeOrchestrator::new(PathBuf::from("/dev/null"), cfg);
    let r = o.run().await.unwrap();
    assert_eq!(r.checks.len(), 9);
    assert_eq!(r.overall_verdict, CheckVerdict::Pass);
    assert!(r.failed_count() == 0);
}

#[tokio::test]
async fn subset_only_runs_selected() {
    let mut cfg = mock_cfg();
    cfg.only_checks = Some(vec![SmokeCheckId::S1, SmokeCheckId::S9]);
    let o = SmokeOrchestrator::new(PathBuf::from("/dev/null"), cfg);
    let r = o.run().await.unwrap();
    assert_eq!(r.checks.len(), 2);
    assert!(r.checks.contains_key("S1"));
    assert!(r.checks.contains_key("S9"));
    assert!(!r.checks.contains_key("S5"));
}

#[tokio::test]
async fn global_timeout_field_is_240s_by_default() {
    // Mock-checks complete in nanoseconds — we can't actually exhaust
    // the deadline before the first check returns.  This test pins the
    // AD3-frozen default value instead of trying to race the orchestrator.
    let cfg = mock_cfg();
    assert_eq!(cfg.global_timeout, efe_smoke::budgets::GLOBAL_HARD_CAP);
    assert_eq!(cfg.global_timeout.as_secs(), 240);
}

#[tokio::test]
async fn tight_per_check_timeout_returns_ok_report_with_some_verdict() {
    // D1 — orchestrator always returns Ok(report) ; the verdict is
    // encoded inside.  A 1-ns timeout MAY race-and-lose to the mock
    // check ; either way the report is produced and surfaces the
    // outcome via `overall_verdict`.
    let mut cfg = mock_cfg();
    cfg.per_check_timeout_override = Some(Duration::from_nanos(1));
    let o = SmokeOrchestrator::new(PathBuf::from("/dev/null"), cfg);
    let r = o.run().await.expect("orchestrator must return Ok even on fail");
    assert!(matches!(
        r.overall_verdict,
        efe_smoke::checks::CheckVerdict::Pass | efe_smoke::checks::CheckVerdict::Fail
    ));
}

#[tokio::test]
async fn per_check_timeout_override_applies() {
    let mut cfg = mock_cfg();
    // Generous : the mock check returns instantly.
    cfg.per_check_timeout_override = Some(Duration::from_secs(5));
    let o = SmokeOrchestrator::new(PathBuf::from("/dev/null"), cfg);
    let r = o.run().await.unwrap();
    assert_eq!(r.checks.len(), 9);
}

#[tokio::test]
async fn report_keys_sorted_canonical_form() {
    let cfg = mock_cfg();
    let o = SmokeOrchestrator::new(PathBuf::from("/dev/null"), cfg);
    let r = o.run().await.unwrap();
    let keys: Vec<_> = r.checks.keys().cloned().collect();
    assert_eq!(keys, vec!["S1", "S2", "S3", "S4", "S5", "S6", "S7", "S8", "S9"]);
}

#[tokio::test]
async fn report_carries_v1_5_stability_string() {
    let cfg = mock_cfg();
    let o = SmokeOrchestrator::new(PathBuf::from("/dev/null"), cfg);
    let r = o.run().await.unwrap();
    assert_eq!(r.stability_contract_version, "1.5");
    assert_eq!(r.schema_version, 1);
}

#[tokio::test]
async fn report_chart_version_propagates() {
    let mut cfg = SmokeConfig::defaults("v0.7.0-rc1".into());
    cfg.mock = true;
    let o = SmokeOrchestrator::new(PathBuf::from("/dev/null"), cfg);
    let r = o.run().await.unwrap();
    assert_eq!(r.chart_version, "v0.7.0-rc1");
}

#[tokio::test]
async fn report_run_id_is_uuid_v7() {
    let cfg = mock_cfg();
    let o = SmokeOrchestrator::new(PathBuf::from("/dev/null"), cfg);
    let r = o.run().await.unwrap();
    // UUID v7 timestamp-ordered : 36-char canonical form.
    assert_eq!(r.run_id.len(), 36);
    assert_eq!(r.run_id.matches('-').count(), 4);
}
