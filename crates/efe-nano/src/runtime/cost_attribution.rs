//! **Sprint 8 — N13** : Routes shadow / canary cost cents to the right
//! budget.
//!
//! See architecture doc § 9.
//!
//! # The dual ledger
//!
//! - **Production** cost always charges the tenant via the existing
//!   N7 swarm cost cap (`SwarmCostTracker`).  Unchanged by N13.
//! - **Shadow** cost charges a configurable budget — either the
//!   platform-wide budget (`CostAttribution::Platform`) or the tenant's
//!   dedicated shadow budget (`CostAttribution::Tenant`).  Enforced
//!   here.
//! - **Canary** cost is real production cost and goes through the
//!   normal N7 path.  N13 doesn't intervene.
//!
//! # Why a separate router ?
//!
//! 1. Shadow budgets are operator-set (vs tenant-self-set) — different
//!    auth path.
//! 2. Shadow budget exhaustion **drops the shadow** rather than
//!    failing the production call — different failure mode.
//! 3. Per-attribution counters need separate observability surfaces.

use dashmap::DashMap;
use efe_nano_sdk::{CostAttribution, TenantId};
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;

// ─────────────────────────────────────────────────────────────────────
//  CostBudgetError
// ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Error, Clone, PartialEq)]
pub enum CostBudgetError {
    #[error("shadow budget exhausted : attribution={attribution} need_cents={need:.6} have_cents={have:.6}")]
    Exhausted {
        attribution: &'static str,
        need: f64,
        have: f64,
    },
    #[error("invalid cost reservation : {0}")]
    Invalid(String),
}

// ─────────────────────────────────────────────────────────────────────
//  CostReservation
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N13** : opaque token representing a successfully-
/// reserved chunk of a budget.  Must be either `commit`-ed (records
/// the actual cost) or `cancel`-ed (refunds the reservation).
#[derive(Debug, Clone)]
pub struct CostReservation {
    pub(crate) attribution: CostAttribution,
    pub(crate) tenant: TenantId,
    pub(crate) reserved_cents: f64,
}

impl CostReservation {
    pub fn attribution(&self) -> CostAttribution {
        self.attribution
    }
    pub fn tenant(&self) -> TenantId {
        self.tenant
    }
    pub fn reserved_cents(&self) -> f64 {
        self.reserved_cents
    }
}

// ─────────────────────────────────────────────────────────────────────
//  CostRouter
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N13** : the dual-ledger router.
///
/// Holds two budget buckets :
/// - One `platform_budget` shared across all tenants (used by
///   `CostAttribution::Platform`).
/// - One per-tenant `tenant_budgets` map (used by
///   `CostAttribution::Tenant`).
///
/// Both are stored as `AtomicU64` representing **micro-cents** (cents
/// × 1 000 000) — gives 6 decimal places of precision while keeping
/// atomic ops simple (no float CAS).
pub struct CostRouter {
    /// Platform shadow budget in micro-cents.
    platform_budget_microcents: AtomicU64,

    /// Per-tenant shadow budget in micro-cents.  Missing key = unlimited.
    tenant_budgets_microcents: DashMap<TenantId, AtomicU64>,

    /// Cumulative spend per attribution, for metric reporting.
    platform_spent_microcents: AtomicU64,
    tenant_spent_microcents: AtomicU64,

    /// Cumulative drops (denials) per attribution.
    platform_drops: AtomicU64,
    tenant_drops: AtomicU64,
}

impl CostRouter {
    /// **Construct a new router with a finite platform budget.**
    /// `platform_budget_cents = f64::INFINITY` ⇒ unlimited platform
    /// shadow budget (dev / test default).
    pub fn new(platform_budget_cents: f64) -> Self {
        Self {
            platform_budget_microcents: AtomicU64::new(to_microcents(platform_budget_cents)),
            tenant_budgets_microcents: DashMap::new(),
            platform_spent_microcents: AtomicU64::new(0),
            tenant_spent_microcents: AtomicU64::new(0),
            platform_drops: AtomicU64::new(0),
            tenant_drops: AtomicU64::new(0),
        }
    }

    /// Convenience : router with unlimited platform budget (typical for
    /// tests + dev).
    pub fn unlimited() -> Self {
        Self::new(f64::INFINITY)
    }

    /// **Set the per-tenant shadow budget.**  `Some(c)` = `c` cents
    /// shared across all this tenant's shadow runs ; `None` = unlimited.
    pub fn set_tenant_budget(&self, tenant: TenantId, budget_cents: Option<f64>) {
        let micro = match budget_cents {
            Some(c) => to_microcents(c),
            None => u64::MAX,
        };
        self.tenant_budgets_microcents
            .entry(tenant)
            .and_modify(|b| b.store(micro, Ordering::Relaxed))
            .or_insert_with(|| AtomicU64::new(micro));
    }

    /// **Reserve `cents` against the right budget.**  Returns
    /// `Err(Exhausted)` if the budget cannot cover the reservation,
    /// in which case the caller drops the shadow.  Returns
    /// `Ok(CostReservation)` on success — caller must then call
    /// [`Self::commit`] with the actual cost.
    pub fn reserve(
        &self,
        tenant: TenantId,
        cents: f64,
        attribution: CostAttribution,
    ) -> Result<CostReservation, CostBudgetError> {
        if !cents.is_finite() || cents < 0.0 {
            return Err(CostBudgetError::Invalid(format!(
                "non-finite or negative cost {cents}"
            )));
        }
        let need = to_microcents(cents);
        match attribution {
            CostAttribution::Platform => {
                self.try_reserve(&self.platform_budget_microcents, need, attribution, tenant)
            }
            CostAttribution::Tenant => {
                // Ensure the tenant bucket exists, then reserve against it.
                self.tenant_budgets_microcents
                    .entry(tenant)
                    .or_insert_with(|| AtomicU64::new(u64::MAX));
                let bucket = self.tenant_budgets_microcents.get(&tenant).unwrap();
                self.try_reserve(&bucket, need, attribution, tenant)
            }
        }
    }

    /// **Bug-fix N13** : `tenant` is now explicit so the reservation
    /// carries the correct tenant id, not the hardcoded `system()`.
    /// Required for `commit` / `cancel` to refund the right bucket.
    fn try_reserve(
        &self,
        budget: &AtomicU64,
        need: u64,
        attribution: CostAttribution,
        tenant: TenantId,
    ) -> Result<CostReservation, CostBudgetError> {
        // CAS loop : load, check, subtract.
        loop {
            let current = budget.load(Ordering::Acquire);
            if current < need {
                // Drop counter.
                let counter = match attribution {
                    CostAttribution::Platform => &self.platform_drops,
                    CostAttribution::Tenant => &self.tenant_drops,
                };
                counter.fetch_add(1, Ordering::Relaxed);
                return Err(CostBudgetError::Exhausted {
                    attribution: attribution.as_str(),
                    need: from_microcents(need),
                    have: from_microcents(current),
                });
            }
            let next = current.saturating_sub(need);
            match budget.compare_exchange(current, next, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => {
                    return Ok(CostReservation {
                        attribution,
                        tenant,
                        reserved_cents: from_microcents(need),
                    });
                }
                Err(_) => continue, // someone else reserved, retry
            }
        }
    }

    /// **Commit the reservation after the shadow / canary completes.**
    ///
    /// If `actual_cents < reserved_cents`, the unused portion is
    /// refunded to the budget.  If `actual_cents > reserved_cents`,
    /// the OVER-spend is recorded but NOT charged back to the budget
    /// (would let attackers escape via under-estimating ; for shadow
    /// cost the over-spend is small and platform-absorbed).
    pub fn commit(&self, reservation: CostReservation, actual_cents: f64) {
        let actual = to_microcents(actual_cents.max(0.0));
        let reserved = to_microcents(reservation.reserved_cents);
        // Refund unused portion to the correct budget bucket.
        if actual < reserved {
            let refund = reserved - actual;
            self.refund(reservation.attribution, reservation.tenant, refund);
        }
        // Track spend.
        let spent = match reservation.attribution {
            CostAttribution::Platform => &self.platform_spent_microcents,
            CostAttribution::Tenant => &self.tenant_spent_microcents,
        };
        spent.fetch_add(actual, Ordering::Relaxed);
    }

    /// **Cancel a reservation, refunding the full reserved amount.**
    /// Used when the shadow couldn't be spawned at all.
    pub fn cancel(&self, reservation: CostReservation) {
        let reserved = to_microcents(reservation.reserved_cents);
        self.refund(reservation.attribution, reservation.tenant, reserved);
    }

    fn refund(&self, attribution: CostAttribution, tenant: TenantId, microcents: u64) {
        match attribution {
            CostAttribution::Platform => {
                self.platform_budget_microcents
                    .fetch_add(microcents, Ordering::AcqRel);
            }
            CostAttribution::Tenant => {
                self.tenant_budgets_microcents
                    .entry(tenant)
                    .or_insert_with(|| AtomicU64::new(u64::MAX));
                let slot = self.tenant_budgets_microcents.get(&tenant).unwrap();
                slot.fetch_add(microcents, Ordering::AcqRel);
            }
        }
    }

    // ─────────────────────────────────────────────────────────────────
    //  Snapshot accessors (metric pipeline)
    // ─────────────────────────────────────────────────────────────────

    pub fn platform_remaining_cents(&self) -> f64 {
        from_microcents(self.platform_budget_microcents.load(Ordering::Acquire))
    }

    pub fn tenant_remaining_cents(&self, tenant: TenantId) -> Option<f64> {
        self.tenant_budgets_microcents
            .get(&tenant)
            .map(|b| from_microcents(b.load(Ordering::Acquire)))
    }

    pub fn platform_spent_cents(&self) -> f64 {
        from_microcents(self.platform_spent_microcents.load(Ordering::Relaxed))
    }

    pub fn tenant_spent_cents_total(&self) -> f64 {
        from_microcents(self.tenant_spent_microcents.load(Ordering::Relaxed))
    }

    pub fn platform_drops_total(&self) -> u64 {
        self.platform_drops.load(Ordering::Relaxed)
    }

    pub fn tenant_drops_total(&self) -> u64 {
        self.tenant_drops.load(Ordering::Relaxed)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  micro-cent conversion
// ─────────────────────────────────────────────────────────────────────

const MICROCENT_PER_CENT: f64 = 1_000_000.0;

fn to_microcents(c: f64) -> u64 {
    if !c.is_finite() {
        return u64::MAX;
    }
    (c * MICROCENT_PER_CENT).round().max(0.0).min(u64::MAX as f64) as u64
}

fn from_microcents(m: u64) -> f64 {
    if m == u64::MAX {
        return f64::INFINITY;
    }
    (m as f64) / MICROCENT_PER_CENT
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserve_within_budget_succeeds() {
        let r = CostRouter::new(1.0);
        let res = r.reserve(TenantId::system(), 0.5, CostAttribution::Platform).unwrap();
        assert_eq!(res.reserved_cents, 0.5);
        assert_eq!(res.attribution, CostAttribution::Platform);
    }

    #[test]
    fn reserve_above_budget_errors() {
        let r = CostRouter::new(1.0);
        let _r1 = r.reserve(TenantId::system(), 0.5, CostAttribution::Platform).unwrap();
        let _r2 = r.reserve(TenantId::system(), 0.5, CostAttribution::Platform).unwrap();
        let err = r.reserve(TenantId::system(), 0.1, CostAttribution::Platform);
        assert!(matches!(err, Err(CostBudgetError::Exhausted { .. })));
        assert_eq!(r.platform_drops_total(), 1);
    }

    #[test]
    fn unlimited_budget_never_exhausts() {
        let r = CostRouter::unlimited();
        for _ in 0..100 {
            let _ = r.reserve(TenantId::system(), 1_000_000.0, CostAttribution::Platform).unwrap();
        }
    }

    #[test]
    fn commit_records_actual_spend() {
        let r = CostRouter::new(10.0);
        let res = r.reserve(TenantId::system(), 1.0, CostAttribution::Platform).unwrap();
        r.commit(res, 0.5);
        assert!((r.platform_spent_cents() - 0.5).abs() < 1e-9);
        // Refund 0.5 went back to budget.
        assert!((r.platform_remaining_cents() - 9.5).abs() < 1e-9);
    }

    #[test]
    fn cancel_refunds_reservation() {
        let r = CostRouter::new(10.0);
        let res = r.reserve(TenantId::system(), 3.0, CostAttribution::Platform).unwrap();
        assert!((r.platform_remaining_cents() - 7.0).abs() < 1e-9);
        r.cancel(res);
        assert!((r.platform_remaining_cents() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn per_tenant_budget_isolated() {
        let r = CostRouter::unlimited();
        let t1 = TenantId::new();
        let t2 = TenantId::new();
        r.set_tenant_budget(t1, Some(1.0));
        r.set_tenant_budget(t2, Some(5.0));
        let _ = r.reserve(t1, 0.8, CostAttribution::Tenant).unwrap();
        // t2 should still have its full budget.
        let _ = r.reserve(t2, 4.0, CostAttribution::Tenant).unwrap();
        let exhausted = r.reserve(t1, 0.5, CostAttribution::Tenant);
        assert!(matches!(exhausted, Err(CostBudgetError::Exhausted { .. })));
    }

    /// **N13 bug-fix regression test** : Tenant-attributed reservations
    /// must carry the actual `tenant` id, NOT the hardcoded
    /// `TenantId::system()`.  Otherwise `commit` and `cancel` refund
    /// the wrong tenant bucket.
    #[test]
    fn tenant_reservation_carries_correct_tenant_id() {
        let r = CostRouter::unlimited();
        let t1 = TenantId::new();
        r.set_tenant_budget(t1, Some(2.0));
        let res = r.reserve(t1, 1.0, CostAttribution::Tenant).unwrap();
        // The reservation MUST carry t1, not system().
        assert_eq!(res.tenant(), t1);
        assert_ne!(res.tenant(), TenantId::system());
    }

    /// **N13 bug-fix regression test** : Tenant commit refunds the
    /// right bucket (the original tenant), not system's bucket.
    #[test]
    fn tenant_commit_refunds_correct_bucket() {
        let r = CostRouter::unlimited();
        let t1 = TenantId::new();
        r.set_tenant_budget(t1, Some(2.0));
        let res = r.reserve(t1, 1.0, CostAttribution::Tenant).unwrap();
        // Reserve 1.0 → t1 has 1.0 remaining.
        assert!((r.tenant_remaining_cents(t1).unwrap() - 1.0).abs() < 1e-9);
        r.commit(res, 0.3);
        // Refund 0.7 → t1 should have 1.7 remaining.
        let remaining = r.tenant_remaining_cents(t1).unwrap();
        assert!(
            (remaining - 1.7).abs() < 1e-9,
            "expected 1.7 remaining for t1 after refund, got {remaining}"
        );
    }

    /// **N13 bug-fix regression test** : Tenant cancel refunds to the
    /// right bucket.
    #[test]
    fn tenant_cancel_refunds_correct_bucket() {
        let r = CostRouter::unlimited();
        let t1 = TenantId::new();
        r.set_tenant_budget(t1, Some(5.0));
        let res = r.reserve(t1, 3.0, CostAttribution::Tenant).unwrap();
        assert!((r.tenant_remaining_cents(t1).unwrap() - 2.0).abs() < 1e-9);
        r.cancel(res);
        assert!((r.tenant_remaining_cents(t1).unwrap() - 5.0).abs() < 1e-9);
    }

    #[test]
    fn invalid_cost_rejected() {
        let r = CostRouter::unlimited();
        assert!(r.reserve(TenantId::system(), -1.0, CostAttribution::Platform).is_err());
        assert!(r.reserve(TenantId::system(), f64::NAN, CostAttribution::Platform).is_err());
        assert!(r.reserve(TenantId::system(), f64::INFINITY, CostAttribution::Platform).is_err());
    }

    #[test]
    fn microcent_round_trip() {
        for c in [0.0, 0.000001, 0.5, 1.0, 100.0, 1_000_000.0] {
            let m = to_microcents(c);
            let r = from_microcents(m);
            assert!((c - r).abs() < 1e-6, "{c} → {m} → {r}");
        }
    }

    #[test]
    fn microcent_infinity_handling() {
        assert_eq!(to_microcents(f64::INFINITY), u64::MAX);
        assert!(from_microcents(u64::MAX).is_infinite());
    }

    #[test]
    fn drop_counter_increments_on_each_denial() {
        let r = CostRouter::new(1.0);
        let _ = r.reserve(TenantId::system(), 0.5, CostAttribution::Platform).unwrap();
        let _ = r.reserve(TenantId::system(), 0.5, CostAttribution::Platform).unwrap();
        for _ in 0..5 {
            let _ = r.reserve(TenantId::system(), 0.1, CostAttribution::Platform);
        }
        assert_eq!(r.platform_drops_total(), 5);
    }

    #[test]
    fn over_commit_does_not_overdraft_budget() {
        let r = CostRouter::new(10.0);
        let res = r.reserve(TenantId::system(), 1.0, CostAttribution::Platform).unwrap();
        // Actual ended up higher than estimate — that's recorded as spent
        // but not double-charged against the budget.
        r.commit(res, 5.0);
        // No refund (actual >= reserved), budget stays at 9.0.
        assert!((r.platform_remaining_cents() - 9.0).abs() < 1e-9);
        // But the spend tracker shows the truthful actual.
        assert!((r.platform_spent_cents() - 5.0).abs() < 1e-9);
    }
}
