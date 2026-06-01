//! PR #23 § 21 — per-check edge case integration tests.
//!
//! Arch doc § 21 budgets 9 × 4 = 36 unit tests across S1–S9.  Many of
//! these are inside `src/checks/sN_*.rs::tests`.  This file consolidates
//! the **cross-cutting** edge cases that exercise the dispatch +
//! mock-fixture + result-shape contracts uniformly across all 9 checks.

use std::time::Duration;

use efe_smoke::checks::{all_checks, dispatch_for, CheckContext, CheckVerdict, SmokeCheckId};

mod helpers {
    use super::*;
    pub fn mock_ctx() -> CheckContext {
        CheckContext {
            kubeconfig: std::path::PathBuf::from("/dev/null"),
            chart_version: "0.6.0".into(),
            namespace: "n".into(),
            timeout: Duration::from_secs(30),
            mock: true,
            redact_logs: false,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
// Cross-cutting : all 9 checks in mock mode return Pass.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn all_9_checks_pass_in_mock_mode() {
    for id in SmokeCheckId::all() {
        let kernel = dispatch_for(id);
        let r = kernel.run(&helpers::mock_ctx()).await.unwrap();
        assert_eq!(
            r.verdict,
            CheckVerdict::Pass,
            "mock-mode {} must pass, got {:?} : {}",
            id.short(),
            r.verdict,
            r.error_summary
        );
    }
}

// ─────────────────────────────────────────────────────────────────────
// Cross-cutting : every CheckResult carries its own id + budget.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn every_check_result_carries_correct_id_and_budget() {
    for id in SmokeCheckId::all() {
        let kernel = dispatch_for(id);
        let r = kernel.run(&helpers::mock_ctx()).await.unwrap();
        assert_eq!(r.id, id, "check {} mislabelled in result", id.short());
        assert_eq!(
            r.budget_s,
            id.budget().as_secs_f64(),
            "check {} budget mismatch",
            id.short()
        );
    }
}

// ─────────────────────────────────────────────────────────────────────
// Cross-cutting : duration_s <= budget_s for every check in mock.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn mock_duration_never_exceeds_budget() {
    for id in SmokeCheckId::all() {
        let kernel = dispatch_for(id);
        let r = kernel.run(&helpers::mock_ctx()).await.unwrap();
        assert!(
            r.duration_s <= r.budget_s,
            "mock {} took {:.6}s > budget {}s",
            id.short(),
            r.duration_s,
            r.budget_s
        );
    }
}

// ─────────────────────────────────────────────────────────────────────
// Cross-cutting : `all_checks()` returns 9 kernels in stable order.
// ─────────────────────────────────────────────────────────────────────

#[test]
fn all_checks_returns_9_kernels_in_stable_order() {
    let kernels = all_checks();
    let ids: Vec<_> = kernels.iter().map(|k| k.id()).collect();
    assert_eq!(ids, SmokeCheckId::all().to_vec());
}

// ─────────────────────────────────────────────────────────────────────
// S1-specific : details JSON carries controller_healthz + participants_ready.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn s1_mock_details_contain_controller_health_and_participants() {
    let r = dispatch_for(SmokeCheckId::S1).run(&helpers::mock_ctx()).await.unwrap();
    assert!(r.details.get("controller_healthz").is_some());
    assert!(r.details.get("participants_ready").is_some());
}

// ─────────────────────────────────────────────────────────────────────
// S2-specific : details carry rpcs_succeeded == 5.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn s2_mock_details_carry_5_rpcs() {
    let r = dispatch_for(SmokeCheckId::S2).run(&helpers::mock_ctx()).await.unwrap();
    assert_eq!(r.details["rpcs_succeeded"], 5);
    assert_eq!(r.details["error_rate"], 0.0);
}

// ─────────────────────────────────────────────────────────────────────
// S3-specific : details carry chain_complete.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn s3_mock_details_indicate_chain_complete() {
    let r = dispatch_for(SmokeCheckId::S3).run(&helpers::mock_ctx()).await.unwrap();
    assert_eq!(r.details["chain_complete"], true);
}

// ─────────────────────────────────────────────────────────────────────
// S4-specific : 0 dropped + cert_cn_before != cert_cn_after.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn s4_mock_zero_dropped_with_cert_rotation() {
    let r = dispatch_for(SmokeCheckId::S4).run(&helpers::mock_ctx()).await.unwrap();
    assert_eq!(r.details["total_failed"], 0);
    assert_ne!(
        r.details["cert_cn_before"], r.details["cert_cn_after"],
        "cert rotation must observe a CN swap"
    );
}

// ─────────────────────────────────────────────────────────────────────
// S5-specific : manifest version increments.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn s5_mock_manifest_version_increments() {
    let r = dispatch_for(SmokeCheckId::S5).run(&helpers::mock_ctx()).await.unwrap();
    let pre = r.details["manifest_version_pre"].as_u64().unwrap();
    let post = r.details["manifest_version_post"].as_u64().unwrap();
    assert!(post > pre, "manifest version must increment : pre={pre}, post={post}");
}

// ─────────────────────────────────────────────────────────────────────
// S6-specific : 14 families observed, no missing.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn s6_mock_observes_14_metric_families() {
    let r = dispatch_for(SmokeCheckId::S6).run(&helpers::mock_ctx()).await.unwrap();
    assert_eq!(r.details["controller_families_observed"], 14);
    assert_eq!(r.details["missing_families"].as_array().unwrap().len(), 0);
}

// ─────────────────────────────────────────────────────────────────────
// S7-specific : ≥ 2 spans, both expected service names.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn s7_mock_observes_2_spans_with_expected_services() {
    let r = dispatch_for(SmokeCheckId::S7).run(&helpers::mock_ctx()).await.unwrap();
    let spans = r.details["spans"].as_u64().unwrap();
    assert!(spans >= 2);
    let services = r.details["services"].as_array().unwrap();
    assert!(services.iter().any(|s| s == "efe-meta-swarm-controller"));
    assert!(services.iter().any(|s| s == "efe-meta-swarm-participant"));
}

// ─────────────────────────────────────────────────────────────────────
// S8-specific : new pod ready < 30s, participants_total = 3, error_rate < 5%.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn s8_mock_meets_restart_targets() {
    let r = dispatch_for(SmokeCheckId::S8).run(&helpers::mock_ctx()).await.unwrap();
    assert!(r.details["new_pod_ready_s"].as_f64().unwrap() < 30.0);
    assert_eq!(r.details["participants_total_post_restart"], 3);
    assert!(r.details["error_rate_during"].as_f64().unwrap() < 0.05);
}

// ─────────────────────────────────────────────────────────────────────
// S9-specific : exit_code == 0, shutdown_s < 10s, 20 gracefully completed.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn s9_mock_meets_shutdown_targets() {
    let r = dispatch_for(SmokeCheckId::S9).run(&helpers::mock_ctx()).await.unwrap();
    assert_eq!(r.details["exit_code"], 0);
    assert!(r.details["shutdown_s"].as_f64().unwrap() < 10.0);
    assert_eq!(r.details["gracefully_completed"], 20);
}

// ─────────────────────────────────────────────────────────────────────
// Real-path : without --mock + without cluster, each non-trivial check
// surfaces a SmokeError::Source (gracefully degraded — testable).
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn real_path_s2_returns_source_error_without_loadgen_feature() {
    let ctx = CheckContext {
        kubeconfig: std::path::PathBuf::from("/dev/null"),
        chart_version: "0.6.0".into(),
        namespace: "n".into(),
        timeout: Duration::from_secs(15),
        mock: false,
        redact_logs: false,
    };
    // Loadgen-source feature disabled by default → Source error
    // (we don't expect a CheckResult fail ; we expect the kernel to
    // bail with an error which the orchestrator turns into a fail row).
    let r = dispatch_for(SmokeCheckId::S2).run(&ctx).await;
    assert!(r.is_err(), "S2 without loadgen-source must fail or error");
}
