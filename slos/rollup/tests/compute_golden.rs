//! PR #22 § 25 — golden compute inputs → outputs.
//!
//! 12 fixture inputs that drive each kernel through its critical
//! branches.  Failure here means the SLI math drifted.

use efe_slo_rollup::sli::{dispatch_for, SliContext, SliVerdict};
use efe_slo_rollup::source::{MockSource, RecoveryEvent};
use efe_slo_rollup::window::WindowSpec;
use efe_slo_rollup::SloId;

fn ctx<'a>(mock: &'a MockSource, w: WindowSpec) -> SliContext<'a> {
    SliContext {
        window: w,
        window_start_unix: 1_779_667_200, // 2026-05-15T03:00:00Z
        source: mock,
        source_label: "prod".into(),
        region: "global".into(),
        chart_version: "0.6.0".into(),
    }
}

// ─────────────────────────────────────────────────────────────────────
// SLO-1 — L2 availability
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn golden_slo1_exact_target() {
    let mut m = MockSource::default();
    m.set_counter("efe_l2_request_total", "status", "success", 995);
    m.set_counter("efe_l2_request_total", "status", "server_error", 5);
    m.set_counter("efe_l2_request_total", "status", "transport_error", 0);
    m.set_coverage(1.0);
    let p = dispatch_for(SloId::L2Availability).compute(&ctx(&m, WindowSpec::FiveMin)).await.unwrap();
    assert!((p.value - 0.995).abs() < 1e-12);
    assert_eq!(p.verdict, SliVerdict::Pass);
    assert_eq!(p.numerator, 995);
    assert_eq!(p.denominator, 1000);
}

#[tokio::test]
async fn golden_slo1_just_below_target() {
    let mut m = MockSource::default();
    m.set_counter("efe_l2_request_total", "status", "success", 994);
    m.set_counter("efe_l2_request_total", "status", "server_error", 6);
    m.set_counter("efe_l2_request_total", "status", "transport_error", 0);
    m.set_coverage(1.0);
    let p = dispatch_for(SloId::L2Availability).compute(&ctx(&m, WindowSpec::FiveMin)).await.unwrap();
    assert_eq!(p.verdict, SliVerdict::Fail);
}

#[tokio::test]
async fn golden_slo1_no_client_error_inflation_t96() {
    let mut m = MockSource::default();
    m.set_counter("efe_l2_request_total", "status", "success", 100);
    m.set_counter("efe_l2_request_total", "status", "server_error", 0);
    m.set_counter("efe_l2_request_total", "status", "transport_error", 0);
    // 100_000 client errors must NOT inflate denominator.
    m.set_counter("efe_l2_request_total", "status", "client_error", 100_000);
    m.set_coverage(1.0);
    let p = dispatch_for(SloId::L2Availability).compute(&ctx(&m, WindowSpec::FiveMin)).await.unwrap();
    assert_eq!(p.denominator, 100);
    assert!((p.value - 1.0).abs() < 1e-12);
}

// ─────────────────────────────────────────────────────────────────────
// SLO-2 — L2 latency p95
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn golden_slo2_at_50ms_boundary_passes() {
    let mut m = MockSource::default();
    m.set_p95("efe_l2_request_duration_seconds_bucket", 0.050);
    m.set_coverage(1.0);
    let p = dispatch_for(SloId::L2LatencyP95).compute(&ctx(&m, WindowSpec::FiveMin)).await.unwrap();
    assert_eq!(p.compliance_bit, 1);
}

#[tokio::test]
async fn golden_slo2_just_over_50ms_fails_window() {
    let mut m = MockSource::default();
    m.set_p95("efe_l2_request_duration_seconds_bucket", 0.051);
    m.set_coverage(1.0);
    let p = dispatch_for(SloId::L2LatencyP95).compute(&ctx(&m, WindowSpec::FiveMin)).await.unwrap();
    assert_eq!(p.compliance_bit, 0);
}

#[tokio::test]
async fn golden_slo2_coverage_50pct_excludes_window() {
    let mut m = MockSource::default();
    m.set_p95("efe_l2_request_duration_seconds_bucket", 0.020);
    m.set_coverage(0.50);
    let p = dispatch_for(SloId::L2LatencyP95).compute(&ctx(&m, WindowSpec::FiveMin)).await.unwrap();
    assert_eq!(p.compliance_bit, 255);
    assert_eq!(p.verdict, SliVerdict::Indeterminate);
}

// ─────────────────────────────────────────────────────────────────────
// SLO-3 — recovery time
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn golden_slo3_99_within_100_passes() {
    let mut m = MockSource::default();
    for _ in 0..99 {
        m.push_recovery_event(RecoveryEvent { duration_seconds: 30.0, at_unix: 0 });
    }
    m.push_recovery_event(RecoveryEvent { duration_seconds: 120.0, at_unix: 0 });
    let p = dispatch_for(SloId::SwarmRecoveryTime).compute(&ctx(&m, WindowSpec::TwentyFourHours)).await.unwrap();
    assert_eq!(p.numerator, 99);
    assert_eq!(p.denominator, 100);
    assert_eq!(p.verdict, SliVerdict::Pass);
}

#[tokio::test]
async fn golden_slo3_under_10_events_indeterminate() {
    let mut m = MockSource::default();
    for _ in 0..5 {
        m.push_recovery_event(RecoveryEvent { duration_seconds: 30.0, at_unix: 0 });
    }
    let p = dispatch_for(SloId::SwarmRecoveryTime).compute(&ctx(&m, WindowSpec::TwentyFourHours)).await.unwrap();
    assert_eq!(p.verdict, SliVerdict::Indeterminate);
}

// ─────────────────────────────────────────────────────────────────────
// SLO-4 — replication lag
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn golden_slo4_min_participant_above_target_fails() {
    let mut m = MockSource::default();
    m.set_participants(vec!["p1".into(), "p2".into(), "p3".into()]);
    m.set_participant_p99("p1", 110.0);
    m.set_participant_p99("p2", 130.0);
    m.set_participant_p99("p3", 150.0);
    m.set_coverage(1.0);
    let p = dispatch_for(SloId::ReplicationLagP99).compute(&ctx(&m, WindowSpec::OneMin)).await.unwrap();
    assert!((p.value - 110.0).abs() < 1e-9);
    assert_eq!(p.verdict, SliVerdict::Fail);
}

#[tokio::test]
async fn golden_slo4_one_healthy_participant_passes() {
    let mut m = MockSource::default();
    m.set_participants(vec!["p1".into(), "p2".into()]);
    m.set_participant_p99("p1", 50.0);
    m.set_participant_p99("p2", 200.0);
    m.set_coverage(1.0);
    let p = dispatch_for(SloId::ReplicationLagP99).compute(&ctx(&m, WindowSpec::OneMin)).await.unwrap();
    assert_eq!(p.value, 50.0);
    assert_eq!(p.verdict, SliVerdict::Pass);
}

// ─────────────────────────────────────────────────────────────────────
// Cross-SLO : every SLO returns Indeterminate on a fresh mock.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn fresh_mock_yields_indeterminate_for_all_slos() {
    let m = MockSource::default();
    for slo in SloId::all() {
        let p = dispatch_for(slo).compute(&ctx(&m, WindowSpec::FiveMin)).await.unwrap();
        assert_eq!(
            p.verdict,
            SliVerdict::Indeterminate,
            "{slo:?} should be indeterminate on empty mock"
        );
    }
}
