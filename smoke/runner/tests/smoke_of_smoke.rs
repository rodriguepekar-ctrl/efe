//! PR #23 § 25 — smoke-of-smoke : meta-test that runs the orchestrator
//! end-to-end (mock path) and validates every artefact path.

use std::path::PathBuf;

use efe_smoke::checks::CheckVerdict;
use efe_smoke::orchestrator::{SmokeConfig, SmokeOrchestrator};
use efe_smoke::result::{
    badge, json::validate_schema, markdown::build_pr_comment, write_report_json,
};

#[tokio::test]
async fn smoke_of_smoke_full_flow() {
    let tmp = tempfile::tempdir().unwrap();

    // 1. Run the orchestrator in mock mode.
    let mut cfg = SmokeConfig::defaults("0.6.0".into());
    cfg.mock = true;
    let o = SmokeOrchestrator::new(PathBuf::from("/dev/null"), cfg);
    let report = o.run().await.expect("mock smoke run must pass");

    // 2. Verdict + check count.
    assert_eq!(report.overall_verdict, CheckVerdict::Pass);
    assert_eq!(report.checks.len(), 9);

    // 3. Schema validation.
    validate_schema(&report).expect("schema must validate");

    // 4. JSON write + read roundtrip.
    let json_path = tmp.path().join("smoke.json");
    write_report_json(&report, &json_path).unwrap();
    let back = efe_smoke::result::read_report_json(&json_path).unwrap();
    assert_eq!(back.checks.len(), 9);

    // 5. PR-comment markdown.
    let md = build_pr_comment(&report);
    assert!(md.contains("✅ Smoke"));
    assert!(md.contains("S1_boot"));

    // 6. Badge SVG byte-stable.
    let badge_svg = badge::render(report.overall_verdict, report.failed_count());
    assert!(badge_svg.starts_with("<?xml"));
    assert!(badge_svg.contains("smoke"));
    assert!(badge_svg.contains("#3fb950"));

    // 7. Gate exit code = 0 for a pass report.
    let rc = efe_smoke::gate::decide(efe_smoke::gate::GateInput {
        report_path: &json_path,
        allow_skip: false,
        reason: None,
    })
    .unwrap();
    assert_eq!(rc, efe_smoke::ExitCode::Success);
}

#[tokio::test]
async fn smoke_of_smoke_with_failing_subset() {
    // Force a fail by setting an impossibly-tight timeout on a single check.
    // The mock-path checks return instantly, so this doesn't trigger ;
    // we instead simulate the report-side path by constructing a Fail
    // report directly and verifying the gate rejects it.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("fail.json");

    let mut results: Vec<efe_smoke::checks::CheckResult> = efe_smoke::checks::SmokeCheckId::all()
        .iter()
        .map(|id| {
            efe_smoke::checks::CheckResult::pass(*id, std::time::Duration::from_secs(1), serde_json::json!({}))
        })
        .collect();
    results[3] = efe_smoke::checks::CheckResult::fail(
        efe_smoke::checks::SmokeCheckId::S4,
        std::time::Duration::from_secs(22),
        "synthetic fail for smoke-of-smoke".into(),
        serde_json::json!({"synthetic": true}),
    );
    let report = efe_smoke::result::SmokeReport::build(
        "01J3F7XGGYM5NZ0CWY3Q7RH3JE".into(),
        "0.6.0".into(),
        "abc1234".into(),
        std::time::Duration::from_secs(120),
        CheckVerdict::Fail,
        results,
    );
    write_report_json(&report, &path).unwrap();

    // Gate must reject (return Err with SmokeFail exit).
    let err = efe_smoke::gate::decide(efe_smoke::gate::GateInput {
        report_path: &path,
        allow_skip: false,
        reason: None,
    })
    .unwrap_err();
    assert_eq!(err.exit_code(), efe_smoke::ExitCode::SmokeFail);

    // Allow-skip with reason overrides → exit 0.
    let rc = efe_smoke::gate::decide(efe_smoke::gate::GateInput {
        report_path: &path,
        allow_skip: true,
        reason: Some("smoke-of-smoke override test"),
    })
    .unwrap();
    assert_eq!(rc, efe_smoke::ExitCode::Success);
}
