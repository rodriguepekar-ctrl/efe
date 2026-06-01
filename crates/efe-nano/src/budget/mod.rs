//! Per-tenant budget enforcement for Mode C (LLM) calls.
//!
//! # Reservation lifecycle
//!
//! ```text
//! Mode C executor                          BudgetEnforcer
//! ────────────────                          ──────────────
//! 1. estimate cost  ─────────────────►
//! 2. pre_check(tenant, cents) ──────►   reserve cents → ReservationId
//! 3. execute LLM call
//! 4. commit(reservation_id, actual) ►   charge actual, release reservation
//!    OR
//! 4. release(reservation_id) ───────►   release without charge (on error)
//! ```
//!
//! Reservations expire after 60 s ; orphaned reservations (caller crash)
//! auto-release on the next `prune_expired_reservations` call (invoked
//! transparently from `pre_check`).
//!
//! # Sprint 5 ship vs Sprint 11 future
//!
//! Sprint 5 ships [`stub::InMemoryBudgetEnforcer`] — process-local state
//! lost on restart, configurable per-tenant cap defaulting to $1 000 /
//! month.  Sprint 11 (Phase 4) swaps in the real `efe-billing` (M31)
//! integration with persistent Postgres-backed counters + Stripe-driven
//! tier caps.
//!
//! The trait surface is **frozen** as of Sprint 5 : Sprint 11 changes the
//! impl, not the contract.

pub mod stub;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use efe_nano_sdk::TenantId;
use uuid::Uuid;

pub use stub::InMemoryBudgetEnforcer;

pub use crate::error::BudgetError;

// ─────────────────────────────────────────────────────────────────────
//  Trait
// ─────────────────────────────────────────────────────────────────────

/// Per-tenant budget enforcer.
#[async_trait]
pub trait BudgetEnforcer: Send + Sync + 'static {
    /// Reserve `cents` from the tenant's budget.
    ///
    /// Returns a `ReservationId` that must be later passed to either
    /// `commit` (when the LLM call succeeded) or `release` (when it
    /// failed).  Returns [`BudgetError::Insufficient`] when the tenant
    /// doesn't have enough remaining headroom.
    async fn pre_check(
        &self,
        tenant: TenantId,
        cents: f64,
    ) -> Result<ReservationId, BudgetError>;

    /// Convert the reservation into a final charge.
    ///
    /// `actual_cents` is the provider-reported cost (from
    /// `LlmCallResponse::cost_cents`).  If `actual_cents` < reserved,
    /// the difference is refunded.  If `actual_cents` > reserved, an
    /// over-budget warning is logged but the charge is committed
    /// (the provider already billed us).
    async fn commit(
        &self,
        reservation: ReservationId,
        actual_cents: f64,
    ) -> Result<(), BudgetError>;

    /// Refund the reservation without any charge.  Used when the LLM
    /// call errors out before any provider-side cost was incurred.
    async fn release(&self, reservation: ReservationId) -> Result<(), BudgetError>;

    /// Current budget snapshot for a tenant.  Returns the configured
    /// cap, current used + reserved, and the window-reset timestamp.
    async fn snapshot(&self, tenant: TenantId) -> Result<BudgetSnapshot, BudgetError>;
}

// ─────────────────────────────────────────────────────────────────────
//  Value types
// ─────────────────────────────────────────────────────────────────────

/// Opaque reservation ID returned by `pre_check`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReservationId(pub Uuid);

impl ReservationId {
    /// Create a new random reservation ID.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Get the underlying UUID.
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for ReservationId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ReservationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Snapshot of a tenant's budget state.
#[derive(Debug, Clone)]
pub struct BudgetSnapshot {
    pub tenant: TenantId,
    /// Total budget for the current window (cents).
    pub cap_cents: f64,
    /// Already-committed spend in the window.
    pub used_cents: f64,
    /// Reserved (pre_check'd, not yet committed) cents.
    pub reserved_cents: f64,
    /// Available headroom = cap - used - reserved.
    pub remaining_cents: f64,
    /// When the budget window resets (typically start of next month).
    pub window_resets_at: DateTime<Utc>,
}

impl BudgetSnapshot {
    /// True if `cents` can be reserved right now.
    pub fn can_reserve(&self, cents: f64) -> bool {
        self.remaining_cents >= cents
    }

    /// Percentage used (0.0 - 1.0).
    pub fn usage_ratio(&self) -> f64 {
        if self.cap_cents == 0.0 {
            return 0.0;
        }
        (self.used_cents / self.cap_cents).min(1.0)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservation_id_unique() {
        let a = ReservationId::new();
        let b = ReservationId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn reservation_id_display() {
        let r = ReservationId(Uuid::nil());
        assert_eq!(r.to_string(), "00000000-0000-0000-0000-000000000000");
    }

    #[test]
    fn reservation_id_default() {
        let r: ReservationId = Default::default();
        assert_ne!(r, ReservationId(Uuid::nil()));
    }

    #[test]
    fn budget_snapshot_can_reserve() {
        let s = BudgetSnapshot {
            tenant: TenantId::system(),
            cap_cents: 1_000.0,
            used_cents: 600.0,
            reserved_cents: 100.0,
            remaining_cents: 300.0,
            window_resets_at: Utc::now() + chrono::Duration::days(30),
        };
        assert!(s.can_reserve(100.0));
        assert!(s.can_reserve(300.0));
        assert!(!s.can_reserve(301.0));
    }

    #[test]
    fn budget_snapshot_usage_ratio() {
        let s = BudgetSnapshot {
            tenant: TenantId::system(),
            cap_cents: 1_000.0,
            used_cents: 250.0,
            reserved_cents: 0.0,
            remaining_cents: 750.0,
            window_resets_at: Utc::now(),
        };
        assert!((s.usage_ratio() - 0.25).abs() < 1e-9);
    }

    #[test]
    fn budget_snapshot_usage_ratio_zero_cap() {
        let s = BudgetSnapshot {
            tenant: TenantId::system(),
            cap_cents: 0.0,
            used_cents: 100.0,    // edge case
            reserved_cents: 0.0,
            remaining_cents: -100.0,
            window_resets_at: Utc::now(),
        };
        // Zero cap → ratio = 0.0 (avoid div-by-zero).
        assert_eq!(s.usage_ratio(), 0.0);
    }

    #[test]
    fn budget_snapshot_usage_ratio_capped_at_one() {
        let s = BudgetSnapshot {
            tenant: TenantId::system(),
            cap_cents: 100.0,
            used_cents: 200.0,    // over-committed (shouldn't happen but defensive)
            reserved_cents: 0.0,
            remaining_cents: -100.0,
            window_resets_at: Utc::now(),
        };
        assert_eq!(s.usage_ratio(), 1.0);
    }
}
