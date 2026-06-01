//! PR #23 § 25 — property-based budget invariants.

use efe_smoke::budgets;
use efe_smoke::checks::SmokeCheckId;
use proptest::prelude::*;

proptest! {
    /// Each check's budget equals the const in `budgets::*`.
    #[test]
    fn budget_consistent_with_const(idx in 0usize..9) {
        let id = SmokeCheckId::all()[idx];
        let const_v = match id {
            SmokeCheckId::S1 => budgets::S1_BOOT,
            SmokeCheckId::S2 => budgets::S2_L1_SWARM,
            SmokeCheckId::S3 => budgets::S3_L2_META_SWARM,
            SmokeCheckId::S4 => budgets::S4_MTLS_ROTATION,
            SmokeCheckId::S5 => budgets::S5_RELOAD_SIGHUP,
            SmokeCheckId::S6 => budgets::S6_METRICS,
            SmokeCheckId::S7 => budgets::S7_OTEL,
            SmokeCheckId::S8 => budgets::S8_RESTART,
            SmokeCheckId::S9 => budgets::S9_SHUTDOWN,
        };
        prop_assert_eq!(id.budget(), const_v);
    }

    /// Round-trip : parse(as_str(id)) returns the same id.
    #[test]
    fn id_string_roundtrips(idx in 0usize..9) {
        let id = SmokeCheckId::all()[idx];
        prop_assert_eq!(SmokeCheckId::parse(id.as_str()), Some(id));
        prop_assert_eq!(SmokeCheckId::parse(id.short()), Some(id));
    }
}

#[test]
fn total_budget_never_exceeds_global_cap() {
    assert!(budgets::TOTAL <= budgets::GLOBAL_HARD_CAP);
}

#[test]
fn sum_of_per_check_budgets_matches_total() {
    let sum: u64 = SmokeCheckId::all().iter().map(|c| c.budget().as_secs()).sum();
    assert_eq!(sum, budgets::TOTAL.as_secs());
}
