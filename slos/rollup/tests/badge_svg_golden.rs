//! PR #22 § 25 — 4 SVG badge byte-stability golden.

use efe_slo_rollup::badge;
use efe_slo_rollup::sli::SliVerdict;
use efe_slo_rollup::SloId;

#[test]
fn all_4_badges_render_deterministically() {
    for id in SloId::all() {
        let a = badge::render(id, 0.999, SliVerdict::Pass);
        let b = badge::render(id, 0.999, SliVerdict::Pass);
        assert_eq!(a, b, "badge for {id:?} must be byte-stable");
    }
}

#[test]
fn badge_pass_is_green_and_contains_value() {
    let svg = badge::render(SloId::L2Availability, 0.9994, SliVerdict::Pass);
    assert!(svg.contains("#3fb950"));
    assert!(svg.contains("99.940%"));
}

#[test]
fn badge_fail_is_red() {
    let svg = badge::render(SloId::L2LatencyP95, 0.95, SliVerdict::Fail);
    assert!(svg.contains("#da3633"));
}

#[test]
fn badge_indeterminate_says_no_data() {
    let svg = badge::render(SloId::SwarmRecoveryTime, f64::NAN, SliVerdict::Indeterminate);
    assert!(svg.contains("no data"));
    assert!(svg.contains("#8b949e"));
}

#[test]
fn badge_filenames_are_frozen_v1_4() {
    assert_eq!(badge::filename(SloId::L2Availability), "slo-1-availability.svg");
    assert_eq!(badge::filename(SloId::L2LatencyP95), "slo-2-latency-p95.svg");
    assert_eq!(badge::filename(SloId::SwarmRecoveryTime), "slo-3-recovery-time.svg");
    assert_eq!(badge::filename(SloId::ReplicationLagP99), "slo-4-replication-lag.svg");
}

#[test]
fn no_script_or_js_in_any_badge_t100() {
    for id in SloId::all() {
        for verdict in [SliVerdict::Pass, SliVerdict::Fail, SliVerdict::Indeterminate] {
            let svg = badge::render(id, 0.99, verdict);
            assert!(!svg.to_lowercase().contains("<script"));
            assert!(!svg.to_lowercase().contains("onclick"));
            assert!(!svg.to_lowercase().contains("javascript:"));
        }
    }
}
