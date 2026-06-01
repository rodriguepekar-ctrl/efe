//! In-memory `BudgetEnforcer` stub for Sprint 5.
//!
//! Sprint 11 swaps this for the real `efe-billing` (M31) integration.
//! The trait surface is stable ; only the impl changes.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use dashmap::DashMap;
use efe_nano_sdk::TenantId;
use std::sync::Mutex;

use super::{BudgetEnforcer, BudgetError, BudgetSnapshot, ReservationId};

/// Default cap per tenant per window.  $1 000 (100 000 cents) — generous
/// enough that tests + small demos don't hit the cap.
pub const DEFAULT_CAP_CENTS: f64 = 100_000.0;

/// Default window length : 30 days.
pub const DEFAULT_WINDOW_DAYS: i64 = 30;

/// Reservation TTL : 60 seconds.  Orphaned reservations (caller crashed
/// before commit/release) are pruned on the next `pre_check` call.
pub const RESERVATION_TTL_SEC: i64 = 60;

// ─────────────────────────────────────────────────────────────────────
//  Per-tenant state
// ─────────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct TenantState {
    cap_cents: f64,
    used_cents: Mutex<f64>,
    reserved_cents: Mutex<f64>,
    window_resets_at: Mutex<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
struct Reservation {
    tenant: TenantId,
    cents: f64,
    created_at: DateTime<Utc>,
}

// ─────────────────────────────────────────────────────────────────────
//  InMemoryBudgetEnforcer
// ─────────────────────────────────────────────────────────────────────

/// In-process budget enforcer.
///
/// State is lost on restart.  For production budget enforcement, use
/// the M31 integration in Sprint 11.
pub struct InMemoryBudgetEnforcer {
    state: DashMap<TenantId, TenantState>,
    reservations: DashMap<ReservationId, Reservation>,
    default_cap_cents: f64,
    window_days: i64,
}

impl InMemoryBudgetEnforcer {
    /// Construct with default $1 000 / month cap.
    pub fn new() -> Self {
        Self::with_cap(DEFAULT_CAP_CENTS)
    }

    /// Construct with a custom per-tenant cap (cents).
    pub fn with_cap(default_cap_cents: f64) -> Self {
        Self {
            state: DashMap::new(),
            reservations: DashMap::new(),
            default_cap_cents,
            window_days: DEFAULT_WINDOW_DAYS,
        }
    }

    /// Override a specific tenant's cap (test helper + admin path).
    pub fn set_cap(&self, tenant: TenantId, cap_cents: f64) {
        self.ensure_state(tenant);
        // We need to take the entry by mut ref to mutate cap.
        if let Some(mut entry) = self.state.get_mut(&tenant) {
            entry.cap_cents = cap_cents;
        }
    }

    /// Force a window reset (test helper).
    pub async fn force_window_reset(&self, tenant: TenantId) {
        self.ensure_state(tenant);
        if let Some(entry) = self.state.get(&tenant) {
            *entry.used_cents.lock().unwrap() = 0.0;
            *entry.reserved_cents.lock().unwrap() = 0.0;
            *entry.window_resets_at.lock().unwrap() =
                Utc::now() + Duration::days(self.window_days);
        }
    }

    /// Number of active reservations (across all tenants).
    pub fn active_reservation_count(&self) -> usize {
        self.reservations.len()
    }

    fn ensure_state(&self, tenant: TenantId) {
        if !self.state.contains_key(&tenant) {
            self.state.insert(
                tenant,
                TenantState {
                    cap_cents: self.default_cap_cents,
                    used_cents: Mutex::new(0.0),
                    reserved_cents: Mutex::new(0.0),
                    window_resets_at: Mutex::new(
                        Utc::now() + Duration::days(self.window_days),
                    ),
                },
            );
        }
    }

    fn prune_expired_reservations(&self) {
        let cutoff = Utc::now() - Duration::seconds(RESERVATION_TTL_SEC);
        // Collect expired IDs first to avoid holding iterator + mutating
        // simultaneously.
        let expired: Vec<(ReservationId, Reservation)> = self
            .reservations
            .iter()
            .filter(|kv| kv.value().created_at < cutoff)
            .map(|kv| (*kv.key(), kv.value().clone()))
            .collect();
        for (id, r) in expired {
            // Auto-release : refund reserved cents to the tenant.
            if let Some(state) = self.state.get(&r.tenant) {
                let mut reserved = state.reserved_cents.lock().unwrap();
                *reserved = (*reserved - r.cents).max(0.0);
            }
            self.reservations.remove(&id);
        }
    }

    /// Auto-reset window if past window_resets_at.
    fn maybe_reset_window(&self, tenant: TenantId) {
        if let Some(entry) = self.state.get(&tenant) {
            let resets_at = *entry.window_resets_at.lock().unwrap();
            if Utc::now() >= resets_at {
                *entry.used_cents.lock().unwrap() = 0.0;
                *entry.window_resets_at.lock().unwrap() =
                    Utc::now() + Duration::days(self.window_days);
            }
        }
    }
}

impl Default for InMemoryBudgetEnforcer {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for InMemoryBudgetEnforcer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryBudgetEnforcer")
            .field("tenants", &self.state.len())
            .field("active_reservations", &self.reservations.len())
            .field("default_cap_cents", &self.default_cap_cents)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl BudgetEnforcer for InMemoryBudgetEnforcer {
    async fn pre_check(
        &self,
        tenant: TenantId,
        cents: f64,
    ) -> Result<ReservationId, BudgetError> {
        if cents < 0.0 {
            return Err(BudgetError::Internal(format!(
                "pre_check with negative cents : {cents}"
            )));
        }
        self.ensure_state(tenant);
        self.prune_expired_reservations();
        self.maybe_reset_window(tenant);

        let state = self.state.get(&tenant).expect("ensure_state inserted");
        let used = *state.used_cents.lock().unwrap();
        let mut reserved = state.reserved_cents.lock().unwrap();

        let remaining = state.cap_cents - used - *reserved;
        if remaining < cents {
            return Err(BudgetError::Insufficient {
                requested_cents: cents,
                remaining_cents: remaining,
            });
        }
        *reserved += cents;
        drop(reserved);     // release lock before insert

        let res_id = ReservationId::new();
        self.reservations.insert(
            res_id,
            Reservation {
                tenant,
                cents,
                created_at: Utc::now(),
            },
        );
        Ok(res_id)
    }

    async fn commit(
        &self,
        reservation: ReservationId,
        actual_cents: f64,
    ) -> Result<(), BudgetError> {
        let r = self
            .reservations
            .remove(&reservation)
            .ok_or_else(|| BudgetError::UnknownReservation(reservation.0))?
            .1;

        let state = self
            .state
            .get(&r.tenant)
            .ok_or_else(|| BudgetError::Internal("tenant state missing".into()))?;
        let mut used = state.used_cents.lock().unwrap();
        let mut reserved = state.reserved_cents.lock().unwrap();

        // Release the reservation.
        *reserved = (*reserved - r.cents).max(0.0);
        // Charge actual cost (clamped to ≥ 0).
        *used += actual_cents.max(0.0);
        if actual_cents > r.cents {
            tracing::warn!(
                target: "efe_nano::budget",
                tenant = %r.tenant,
                reserved = r.cents,
                actual = actual_cents,
                "actual cost exceeded reservation"
            );
        }
        Ok(())
    }

    async fn release(&self, reservation: ReservationId) -> Result<(), BudgetError> {
        let r = self
            .reservations
            .remove(&reservation)
            .ok_or_else(|| BudgetError::UnknownReservation(reservation.0))?
            .1;

        let state = self
            .state
            .get(&r.tenant)
            .ok_or_else(|| BudgetError::Internal("tenant state missing".into()))?;
        let mut reserved = state.reserved_cents.lock().unwrap();
        *reserved = (*reserved - r.cents).max(0.0);
        Ok(())
    }

    async fn snapshot(&self, tenant: TenantId) -> Result<BudgetSnapshot, BudgetError> {
        self.ensure_state(tenant);
        self.maybe_reset_window(tenant);
        let state = self.state.get(&tenant).expect("ensure_state inserted");
        let used = *state.used_cents.lock().unwrap();
        let reserved = *state.reserved_cents.lock().unwrap();
        let resets = *state.window_resets_at.lock().unwrap();
        Ok(BudgetSnapshot {
            tenant,
            cap_cents: state.cap_cents,
            used_cents: used,
            reserved_cents: reserved,
            remaining_cents: state.cap_cents - used - reserved,
            window_resets_at: resets,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn pre_check_succeeds_within_cap() {
        let b = InMemoryBudgetEnforcer::with_cap(100.0);
        let tenant = TenantId::system();
        let res = b.pre_check(tenant, 30.0).await.unwrap();
        let snap = b.snapshot(tenant).await.unwrap();
        assert_eq!(snap.reserved_cents, 30.0);
        assert_eq!(snap.used_cents, 0.0);
        assert_eq!(snap.remaining_cents, 70.0);
        b.commit(res, 30.0).await.unwrap();
    }

    #[tokio::test]
    async fn pre_check_fails_when_insufficient() {
        let b = InMemoryBudgetEnforcer::with_cap(50.0);
        let tenant = TenantId::system();
        let err = b.pre_check(tenant, 100.0).await.unwrap_err();
        match err {
            BudgetError::Insufficient {
                requested_cents,
                remaining_cents,
            } => {
                assert_eq!(requested_cents, 100.0);
                assert_eq!(remaining_cents, 50.0);
            }
            other => panic!("expected Insufficient, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn commit_charges_actual_cost() {
        let b = InMemoryBudgetEnforcer::with_cap(100.0);
        let tenant = TenantId::system();
        let res = b.pre_check(tenant, 30.0).await.unwrap();
        // Commit with actual cost 20 (under reservation).
        b.commit(res, 20.0).await.unwrap();
        let snap = b.snapshot(tenant).await.unwrap();
        assert_eq!(snap.used_cents, 20.0);
        assert_eq!(snap.reserved_cents, 0.0);    // released
        assert_eq!(snap.remaining_cents, 80.0);
    }

    #[tokio::test]
    async fn commit_over_reserved_still_succeeds() {
        let b = InMemoryBudgetEnforcer::with_cap(100.0);
        let tenant = TenantId::system();
        let res = b.pre_check(tenant, 20.0).await.unwrap();
        // Provider reported 25 cents (over our estimate).
        b.commit(res, 25.0).await.unwrap();
        let snap = b.snapshot(tenant).await.unwrap();
        assert_eq!(snap.used_cents, 25.0);
    }

    #[tokio::test]
    async fn release_refunds_reservation() {
        let b = InMemoryBudgetEnforcer::with_cap(100.0);
        let tenant = TenantId::system();
        let res = b.pre_check(tenant, 30.0).await.unwrap();
        b.release(res).await.unwrap();
        let snap = b.snapshot(tenant).await.unwrap();
        assert_eq!(snap.used_cents, 0.0);
        assert_eq!(snap.reserved_cents, 0.0);
        assert_eq!(snap.remaining_cents, 100.0);
    }

    #[tokio::test]
    async fn double_commit_returns_unknown_reservation() {
        let b = InMemoryBudgetEnforcer::with_cap(100.0);
        let tenant = TenantId::system();
        let res = b.pre_check(tenant, 30.0).await.unwrap();
        b.commit(res, 30.0).await.unwrap();
        let err = b.commit(res, 30.0).await.unwrap_err();
        assert!(matches!(err, BudgetError::UnknownReservation(_)));
    }

    #[tokio::test]
    async fn release_after_commit_returns_unknown_reservation() {
        let b = InMemoryBudgetEnforcer::with_cap(100.0);
        let tenant = TenantId::system();
        let res = b.pre_check(tenant, 30.0).await.unwrap();
        b.commit(res, 30.0).await.unwrap();
        let err = b.release(res).await.unwrap_err();
        assert!(matches!(err, BudgetError::UnknownReservation(_)));
    }

    #[tokio::test]
    async fn multi_tenant_isolation() {
        let b = InMemoryBudgetEnforcer::with_cap(50.0);
        let t1 = TenantId::new();
        let t2 = TenantId::new();
        b.pre_check(t1, 30.0).await.unwrap();
        b.pre_check(t2, 30.0).await.unwrap();
        let s1 = b.snapshot(t1).await.unwrap();
        let s2 = b.snapshot(t2).await.unwrap();
        assert_eq!(s1.reserved_cents, 30.0);
        assert_eq!(s2.reserved_cents, 30.0);
        // t1's pre_check doesn't eat into t2's budget.
        assert_eq!(s1.remaining_cents, 20.0);
        assert_eq!(s2.remaining_cents, 20.0);
    }

    #[tokio::test]
    async fn negative_cents_pre_check_errors() {
        let b = InMemoryBudgetEnforcer::with_cap(100.0);
        let err = b.pre_check(TenantId::system(), -10.0).await.unwrap_err();
        match err {
            BudgetError::Internal(msg) => assert!(msg.contains("negative")),
            other => panic!("expected Internal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn set_cap_overrides_default() {
        let b = InMemoryBudgetEnforcer::with_cap(100.0);
        let tenant = TenantId::system();
        // Reserve some first to establish state.
        let res = b.pre_check(tenant, 10.0).await.unwrap();
        b.commit(res, 10.0).await.unwrap();
        // Override cap.
        b.set_cap(tenant, 500.0);
        let snap = b.snapshot(tenant).await.unwrap();
        assert_eq!(snap.cap_cents, 500.0);
        // Remaining = 500 - 10 (used) = 490.
        assert_eq!(snap.remaining_cents, 490.0);
    }

    #[tokio::test]
    async fn force_window_reset_zeroes_usage() {
        let b = InMemoryBudgetEnforcer::with_cap(100.0);
        let tenant = TenantId::system();
        let res = b.pre_check(tenant, 50.0).await.unwrap();
        b.commit(res, 50.0).await.unwrap();
        b.force_window_reset(tenant).await;
        let snap = b.snapshot(tenant).await.unwrap();
        assert_eq!(snap.used_cents, 0.0);
        assert_eq!(snap.reserved_cents, 0.0);
    }

    #[tokio::test]
    async fn concurrent_pre_checks_serialize_per_tenant() {
        // 32 concurrent pre_checks of 5 cents each on a tenant with cap 100.
        // Exactly 20 must succeed (100 / 5).  Some will fail with Insufficient.
        let b = Arc::new(InMemoryBudgetEnforcer::with_cap(100.0));
        let tenant = TenantId::system();
        let mut handles = Vec::new();
        for _ in 0..32 {
            let b = Arc::clone(&b);
            handles.push(tokio::spawn(async move {
                b.pre_check(tenant, 5.0).await
            }));
        }
        let mut ok = 0;
        let mut err = 0;
        for h in handles {
            match h.await.unwrap() {
                Ok(_) => ok += 1,
                Err(BudgetError::Insufficient { .. }) => err += 1,
                Err(e) => panic!("unexpected error : {e:?}"),
            }
        }
        // Total cap = 100, each reservation = 5, max successes = 20.
        assert_eq!(ok, 20, "expected 20 successful pre_checks, got {ok}");
        assert_eq!(err, 12, "expected 12 failed pre_checks, got {err}");
    }

    #[tokio::test]
    async fn active_reservation_count_tracks() {
        let b = InMemoryBudgetEnforcer::with_cap(1_000.0);
        let tenant = TenantId::system();
        assert_eq!(b.active_reservation_count(), 0);
        let r1 = b.pre_check(tenant, 5.0).await.unwrap();
        let r2 = b.pre_check(tenant, 5.0).await.unwrap();
        assert_eq!(b.active_reservation_count(), 2);
        b.commit(r1, 5.0).await.unwrap();
        assert_eq!(b.active_reservation_count(), 1);
        b.release(r2).await.unwrap();
        assert_eq!(b.active_reservation_count(), 0);
    }

    #[tokio::test]
    async fn snapshot_for_new_tenant_creates_state() {
        let b = InMemoryBudgetEnforcer::with_cap(500.0);
        let snap = b.snapshot(TenantId::new()).await.unwrap();
        assert_eq!(snap.cap_cents, 500.0);
        assert_eq!(snap.used_cents, 0.0);
        assert_eq!(snap.remaining_cents, 500.0);
    }
}
