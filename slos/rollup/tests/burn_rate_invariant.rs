//! PR #22 § 25 — burn-rate invariants.

use efe_slo_rollup::burn_rate::{evaluate_2window, fast_threshold, slow_threshold, BurnRate, BurnRateVerdict};

const TARGET: f64 = 0.995;

fn br(sli: f64) -> BurnRate {
    BurnRate::from_sli(sli, TARGET)
}

#[test]
fn no_burn_when_perfect_sli() {
    let v = evaluate_2window(br(1.0), br(1.0), br(1.0), br(1.0));
    assert_eq!(v, BurnRateVerdict::Healthy);
}

#[test]
fn fast_burn_needs_both_windows_to_be_hot() {
    // 5m hot, 1h cold → no fast.
    let v = evaluate_2window(br(0.85), br(1.0), br(1.0), br(1.0));
    assert_eq!(v, BurnRateVerdict::Healthy);
    // 1h hot, 5m cold → no fast.
    let v = evaluate_2window(br(1.0), br(1.0), br(0.85), br(1.0));
    assert_eq!(v, BurnRateVerdict::Healthy);
}

#[test]
fn fast_burn_fires_when_both_hot() {
    // Both 5m and 1h at burn = 30 (well above 14.4).
    // burn = (1-SLI)/(1-target) → SLI = 1 - 30*(1-0.995) = 1 - 0.15 = 0.85.
    let v = evaluate_2window(br(0.85), br(0.85), br(0.85), br(0.85));
    assert_eq!(v, BurnRateVerdict::FastBurn);
}

#[test]
fn slow_burn_fires_at_6x_both_30m_and_6h() {
    // burn = 6 → SLI = 1 - 6 * 0.005 = 0.97
    let v = evaluate_2window(br(1.0), br(0.97), br(1.0), br(0.97));
    assert_eq!(v, BurnRateVerdict::SlowBurn);
}

#[test]
fn thresholds_are_14_4_and_6_v1_4() {
    assert!((fast_threshold() - 14.4).abs() < f64::EPSILON);
    assert!((slow_threshold() - 6.0).abs() < f64::EPSILON);
}

#[test]
fn yaml_emit_lock() {
    // Frozen output : exactly 8 alerts, both fast & slow groups.
    let yaml = efe_slo_rollup::burn_rate::prom_rule::emit_yaml();
    assert!(yaml.contains("name: slo-burn-rate-fast"));
    assert!(yaml.contains("name: slo-burn-rate-slow"));
    assert!(yaml.contains("SLOBurnRateFast_L2_AVAILABILITY"));
    assert!(yaml.contains("SLOBurnRateFast_L2_LATENCY_P95"));
    assert!(yaml.contains("SLOBurnRateFast_SWARM_RECOVERY_TIME"));
    assert!(yaml.contains("SLOBurnRateFast_REPLICATION_LAG_P99"));
    assert!(yaml.contains("SLOBurnRateSlow_L2_AVAILABILITY"));
    assert!(yaml.contains("SLOBurnRateSlow_L2_LATENCY_P95"));
    assert!(yaml.contains("SLOBurnRateSlow_SWARM_RECOVERY_TIME"));
    assert!(yaml.contains("SLOBurnRateSlow_REPLICATION_LAG_P99"));
}

#[test]
fn fast_precedes_slow_when_both_apply() {
    let v = evaluate_2window(br(0.85), br(0.85), br(0.85), br(0.85));
    // 14.4× burn implies 6× burn too. Fast takes precedence.
    assert_eq!(v, BurnRateVerdict::FastBurn);
}
