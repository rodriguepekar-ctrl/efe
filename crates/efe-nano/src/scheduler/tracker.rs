// SPDX-License-Identifier: UNLICENSED
//! **Sprint 8 — N15** : Inflight tracker — per-tenant atomic counters
//! for ops / swarms / queued swarms.
//!
//! # Purpose (architecture § 5.5)
//!
//! The DRR scheduler controls **slot allocation** ; the tracker
//! controls **per-tenant accounting** : "does tenant T currently
//! have more than `max_inflight_ops` outstanding ?"  These are
//! separate concerns because :
//!
//! - DRR works on a **global** capacity pool.
//! - Per-tenant caps need to be enforced **before** the global pool
//!   is touched — a tenant at their op-cap is denied immediately
//!   without consuming a global slot.
//!
//! # Atomicity guarantee (G3 + I6)
//!
//! Reserving a swarm is a **two-counter atomic** operation : bumping
//! `inflight_ops += op_count` AND `inflight_swarms += 1` must succeed
//! or fail together.  We achieve this with a per-tenant mutex held
//! for the duration of the check+bump.  Reads (for metrics) remain
//! lock-free via [`AtomicU32`] loads.
//!
//! # Memory bound (G10)
//!
//! Per-tenant entries are created lazily on first reservation.  They
//! are NEVER garbage-collected in Sprint 8 — bounded at ~`tenant_count
//! × 24 bytes` (~240 KB for 10 000 tenants).  Sprint 9 will add GC
//! when all three counters reach zero.
//!
//! # API surface
//!
//! - [`InflightTracker::try_reserve_swarm`] : atomic reservation.
//! - [`InflightTracker::release_swarm`] : atomic release.
//! - [`InflightTracker::try_enqueue_swarm`] / [`Self::dequeue_swarm`]
//!   for the swarm-intake queue depth.
//! - [`InflightTracker::snapshot`] : lock-free read for metrics.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use dashmap::DashMap;
use efe_nano_sdk::{BackpressureKind, TenantId};

// ─────────────────────────────────────────────────────────────────────
//  InflightSnapshot
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N15** : lock-free snapshot of a tenant's counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InflightSnapshot {
    /// Operations currently executing for this tenant.
    pub inflight_ops: u32,
    /// Swarms currently executing for this tenant.
    pub inflight_swarms: u32,
    /// Swarms enqueued (admitted to queue, not yet dispatched).
    pub queued_swarms: u32,
}

impl InflightSnapshot {
    /// Zero snapshot — for unknown tenants.
    pub const ZERO: Self = Self {
        inflight_ops: 0,
        inflight_swarms: 0,
        queued_swarms: 0,
    };

    /// True if every counter is zero.
    pub fn is_empty(&self) -> bool {
        self.inflight_ops == 0 && self.inflight_swarms == 0 && self.queued_swarms == 0
    }
}

// ─────────────────────────────────────────────────────────────────────
//  TenantSlot
// ─────────────────────────────────────────────────────────────────────

/// Per-tenant counter state.  Reads via the atomics are lock-free ;
/// writes go through `bump_lock` to keep the multi-counter reservation
/// atomic.
struct TenantSlot {
    inflight_ops: AtomicU32,
    inflight_swarms: AtomicU32,
    queued_swarms: AtomicU32,
    bump_lock: Mutex<()>,
}

impl Default for TenantSlot {
    fn default() -> Self {
        Self {
            inflight_ops: AtomicU32::new(0),
            inflight_swarms: AtomicU32::new(0),
            queued_swarms: AtomicU32::new(0),
            bump_lock: Mutex::new(()),
        }
    }
}

impl TenantSlot {
    fn snapshot(&self) -> InflightSnapshot {
        InflightSnapshot {
            inflight_ops: self.inflight_ops.load(Ordering::Relaxed),
            inflight_swarms: self.inflight_swarms.load(Ordering::Relaxed),
            queued_swarms: self.queued_swarms.load(Ordering::Relaxed),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  InflightTracker
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N15** : per-tenant inflight accountant.
///
/// Construction returns `Arc<Self>` for shared ownership across the
/// dispatcher / executor / metrics surface.  All API methods take
/// `&self` and are safe to call concurrently from multiple tasks.
pub struct InflightTracker {
    tenants: DashMap<TenantId, Arc<TenantSlot>>,
}

impl std::fmt::Debug for InflightTracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InflightTracker")
            .field("tenants", &self.tenants.len())
            .finish()
    }
}

impl Default for InflightTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl InflightTracker {
    /// **N15** : construct an empty tracker.
    pub fn new() -> Self {
        Self {
            tenants: DashMap::new(),
        }
    }

    /// **N15** : wrap in an `Arc` for trait-object sharing.
    pub fn arc() -> Arc<Self> {
        Arc::new(Self::new())
    }

    /// **N15** : number of tenants with at least one entry.
    pub fn tenants_len(&self) -> usize {
        self.tenants.len()
    }

    /// **N15** : true if no tenant entries.
    pub fn is_empty(&self) -> bool {
        self.tenants.is_empty()
    }

    /// **N15** : lock-free snapshot of `tenant`'s counters.  Returns
    /// [`InflightSnapshot::ZERO`] if the tenant has no entry.
    pub fn snapshot(&self, tenant: TenantId) -> InflightSnapshot {
        self.tenants
            .get(&tenant)
            .map_or(InflightSnapshot::ZERO, |slot| slot.snapshot())
    }

    /// Internal : get-or-create the tenant slot.
    ///
    /// Optimized to avoid wasted allocations under concurrent
    /// creation : the construction closure runs at most once per
    /// shard, and the fast `get`-path returns immediately on the
    /// common hit case.
    fn slot(&self, tenant: TenantId) -> Arc<TenantSlot> {
        if let Some(s) = self.tenants.get(&tenant) {
            return Arc::clone(s.value());
        }
        Arc::clone(
            self.tenants
                .entry(tenant)
                .or_insert_with(|| Arc::new(TenantSlot::default()))
                .value(),
        )
    }

    /// **N15** : atomically reserve `op_count` ops + 1 swarm slot for
    /// `tenant`.  Returns `Ok(())` if both fit under the caps,
    /// otherwise returns the precise [`BackpressureKind`].
    ///
    /// On `Err`, **no** counter is bumped — atomicity is preserved.
    pub fn try_reserve_swarm(
        &self,
        tenant: TenantId,
        op_count: u32,
        max_inflight_ops: u32,
        max_inflight_swarms: u32,
    ) -> Result<(), BackpressureKind> {
        let slot = self.slot(tenant);
        let _g = slot.bump_lock.lock().expect("tracker bump_lock poisoned");
        let ops_now = slot.inflight_ops.load(Ordering::Relaxed);
        let swarms_now = slot.inflight_swarms.load(Ordering::Relaxed);

        let ops_after = ops_now.saturating_add(op_count);
        if ops_after > max_inflight_ops {
            return Err(BackpressureKind::OpInflightFull);
        }
        let swarms_after = swarms_now.saturating_add(1);
        if swarms_after > max_inflight_swarms {
            return Err(BackpressureKind::SwarmInflightFull);
        }
        slot.inflight_ops.store(ops_after, Ordering::Release);
        slot.inflight_swarms.store(swarms_after, Ordering::Release);
        Ok(())
    }

    /// **N15** : release a previously-reserved swarm.  Decrements
    /// `inflight_ops` by `op_count` and `inflight_swarms` by 1,
    /// saturating at 0 (defensive — should never underflow in
    /// practice).
    pub fn release_swarm(&self, tenant: TenantId, op_count: u32) {
        let Some(slot) = self.tenants.get(&tenant) else {
            return;
        };
        let _g = slot.bump_lock.lock().expect("tracker bump_lock poisoned");
        let ops = slot.inflight_ops.load(Ordering::Relaxed);
        let swarms = slot.inflight_swarms.load(Ordering::Relaxed);
        slot.inflight_ops
            .store(ops.saturating_sub(op_count), Ordering::Release);
        slot.inflight_swarms
            .store(swarms.saturating_sub(1), Ordering::Release);
    }

    /// **N15** : try to enqueue a swarm in the per-tenant intake
    /// queue.  Returns `Err(BackpressureKind::QueueFull)` if the
    /// queue depth cap is reached.
    pub fn try_enqueue_swarm(
        &self,
        tenant: TenantId,
        queue_depth_cap: u32,
    ) -> Result<(), BackpressureKind> {
        let slot = self.slot(tenant);
        let _g = slot.bump_lock.lock().expect("tracker bump_lock poisoned");
        let q = slot.queued_swarms.load(Ordering::Relaxed);
        let next = q.saturating_add(1);
        if next > queue_depth_cap {
            return Err(BackpressureKind::QueueFull);
        }
        slot.queued_swarms.store(next, Ordering::Release);
        Ok(())
    }

    /// **N15** : remove one swarm from the intake queue (called when
    /// the swarm transitions from "queued" to "running" or is
    /// abandoned).  Saturating-sub at 0.
    pub fn dequeue_swarm(&self, tenant: TenantId) {
        let Some(slot) = self.tenants.get(&tenant) else {
            return;
        };
        let _g = slot.bump_lock.lock().expect("tracker bump_lock poisoned");
        let q = slot.queued_swarms.load(Ordering::Relaxed);
        slot.queued_swarms.store(q.saturating_sub(1), Ordering::Release);
    }

    /// **N15** : forcibly drop the entry for `tenant`.  Used by
    /// tenant offboarding (M24 GDPR saga).  Caller MUST guarantee no
    /// in-flight ops belong to this tenant (otherwise the next
    /// release will underflow → saturate at zero, leaving the
    /// counters mismatched, but correct).
    pub fn forget(&self, tenant: TenantId) -> bool {
        self.tenants.remove(&tenant).is_some()
    }

    /// **N15** : visit every tenant entry.  Used by metrics surface.
    pub fn for_each<F: FnMut(TenantId, InflightSnapshot)>(&self, mut f: F) {
        for r in self.tenants.iter() {
            let snap = r.value().snapshot();
            f(*r.key(), snap);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn ten() -> TenantId {
        TenantId::new()
    }

    #[test]
    fn fresh_tracker_is_empty() {
        let t = InflightTracker::new();
        assert!(t.is_empty());
        assert_eq!(t.tenants_len(), 0);
    }

    #[test]
    fn snapshot_unknown_tenant_returns_zero() {
        let t = InflightTracker::new();
        let snap = t.snapshot(ten());
        assert_eq!(snap, InflightSnapshot::ZERO);
        assert!(snap.is_empty());
    }

    #[test]
    fn try_reserve_succeeds_within_caps() {
        let t = InflightTracker::new();
        let tn = ten();
        assert!(t.try_reserve_swarm(tn, 5, 100, 10).is_ok());
        let snap = t.snapshot(tn);
        assert_eq!(snap.inflight_ops, 5);
        assert_eq!(snap.inflight_swarms, 1);
    }

    #[test]
    fn try_reserve_op_overflow_returns_correct_kind() {
        let t = InflightTracker::new();
        let tn = ten();
        // Cap = 10 ops.  Reserve 9 ; then try 2 more → fails.
        assert!(t.try_reserve_swarm(tn, 9, 10, 100).is_ok());
        let err = t.try_reserve_swarm(tn, 2, 10, 100).unwrap_err();
        assert_eq!(err, BackpressureKind::OpInflightFull);
        // No counter movement.
        let snap = t.snapshot(tn);
        assert_eq!(snap.inflight_ops, 9);
        assert_eq!(snap.inflight_swarms, 1);
    }

    #[test]
    fn try_reserve_swarm_overflow_returns_correct_kind() {
        let t = InflightTracker::new();
        let tn = ten();
        // Cap = 2 swarms.
        assert!(t.try_reserve_swarm(tn, 1, 100, 2).is_ok());
        assert!(t.try_reserve_swarm(tn, 1, 100, 2).is_ok());
        let err = t.try_reserve_swarm(tn, 1, 100, 2).unwrap_err();
        assert_eq!(err, BackpressureKind::SwarmInflightFull);
    }

    #[test]
    fn release_swarm_decrements_counters() {
        let t = InflightTracker::new();
        let tn = ten();
        t.try_reserve_swarm(tn, 5, 100, 10).unwrap();
        t.release_swarm(tn, 5);
        let snap = t.snapshot(tn);
        assert_eq!(snap.inflight_ops, 0);
        assert_eq!(snap.inflight_swarms, 0);
    }

    #[test]
    fn release_swarm_unknown_tenant_is_noop() {
        let t = InflightTracker::new();
        t.release_swarm(ten(), 10); // shouldn't panic
        assert!(t.is_empty());
    }

    #[test]
    fn release_swarm_saturating_sub_no_underflow() {
        let t = InflightTracker::new();
        let tn = ten();
        t.try_reserve_swarm(tn, 1, 100, 10).unwrap();
        t.release_swarm(tn, 100); // op_count > inflight → saturates at 0
        let snap = t.snapshot(tn);
        assert_eq!(snap.inflight_ops, 0);
        assert_eq!(snap.inflight_swarms, 0);
    }

    #[test]
    fn try_enqueue_swarm_respects_depth_cap() {
        let t = InflightTracker::new();
        let tn = ten();
        assert!(t.try_enqueue_swarm(tn, 3).is_ok());
        assert!(t.try_enqueue_swarm(tn, 3).is_ok());
        assert!(t.try_enqueue_swarm(tn, 3).is_ok());
        let err = t.try_enqueue_swarm(tn, 3).unwrap_err();
        assert_eq!(err, BackpressureKind::QueueFull);
        assert_eq!(t.snapshot(tn).queued_swarms, 3);
    }

    #[test]
    fn dequeue_swarm_decrements() {
        let t = InflightTracker::new();
        let tn = ten();
        t.try_enqueue_swarm(tn, 5).unwrap();
        t.try_enqueue_swarm(tn, 5).unwrap();
        t.dequeue_swarm(tn);
        assert_eq!(t.snapshot(tn).queued_swarms, 1);
    }

    #[test]
    fn forget_drops_tenant_entry() {
        let t = InflightTracker::new();
        let tn = ten();
        t.try_reserve_swarm(tn, 1, 100, 10).unwrap();
        assert_eq!(t.tenants_len(), 1);
        assert!(t.forget(tn));
        assert_eq!(t.tenants_len(), 0);
        // Unknown forget returns false.
        assert!(!t.forget(tn));
    }

    #[test]
    fn for_each_visits_all_tenants() {
        let t = InflightTracker::new();
        let a = ten();
        let b = ten();
        t.try_reserve_swarm(a, 1, 100, 10).unwrap();
        t.try_reserve_swarm(b, 5, 100, 10).unwrap();
        let mut visited: std::collections::HashMap<TenantId, u32> =
            std::collections::HashMap::new();
        t.for_each(|tid, snap| {
            visited.insert(tid, snap.inflight_ops);
        });
        assert_eq!(visited.len(), 2);
        assert_eq!(visited[&a], 1);
        assert_eq!(visited[&b], 5);
    }

    #[test]
    fn separate_tenants_have_isolated_counters() {
        let t = InflightTracker::new();
        let a = ten();
        let b = ten();
        t.try_reserve_swarm(a, 5, 10, 10).unwrap();
        t.try_reserve_swarm(b, 7, 10, 10).unwrap();
        assert_eq!(t.snapshot(a).inflight_ops, 5);
        assert_eq!(t.snapshot(b).inflight_ops, 7);
        // Capping a's at 10 doesn't affect b.
        assert!(t.try_reserve_swarm(a, 6, 10, 10).is_err());
        assert!(t.try_reserve_swarm(b, 3, 10, 10).is_ok());
    }

    #[tokio::test]
    async fn concurrent_reserves_respect_cap() {
        let t = Arc::new(InflightTracker::new());
        let tn = ten();
        // Cap = 50 ops ; 200 tasks race to reserve 1 each.
        let mut handles = vec![];
        for _ in 0..200 {
            let t = Arc::clone(&t);
            handles.push(tokio::spawn(async move {
                t.try_reserve_swarm(tn, 1, 50, 1_000).is_ok()
            }));
        }
        let mut granted = 0;
        for h in handles {
            if h.await.unwrap() {
                granted += 1;
            }
        }
        assert_eq!(granted, 50);
        assert_eq!(t.snapshot(tn).inflight_ops, 50);
        assert_eq!(t.snapshot(tn).inflight_swarms, 50);
    }

    #[tokio::test]
    async fn concurrent_reserve_release_maintains_invariant() {
        let t = Arc::new(InflightTracker::new());
        let tn = ten();
        let n_iters = 100;
        let mut handles = vec![];
        for _ in 0..n_iters {
            let t = Arc::clone(&t);
            handles.push(tokio::spawn(async move {
                if t.try_reserve_swarm(tn, 1, 1_000, 1_000).is_ok() {
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                    t.release_swarm(tn, 1);
                }
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        let snap = t.snapshot(tn);
        assert_eq!(snap.inflight_ops, 0);
        assert_eq!(snap.inflight_swarms, 0);
    }

    #[test]
    fn op_count_zero_is_a_valid_reservation() {
        let t = InflightTracker::new();
        let tn = ten();
        assert!(t.try_reserve_swarm(tn, 0, 10, 10).is_ok());
        let snap = t.snapshot(tn);
        assert_eq!(snap.inflight_ops, 0);
        assert_eq!(snap.inflight_swarms, 1);
        t.release_swarm(tn, 0);
        assert_eq!(t.snapshot(tn).inflight_swarms, 0);
    }

    #[test]
    fn reserve_at_exact_cap_succeeds() {
        let t = InflightTracker::new();
        let tn = ten();
        assert!(t.try_reserve_swarm(tn, 10, 10, 1).is_ok());
        assert_eq!(t.snapshot(tn).inflight_ops, 10);
    }

    #[test]
    fn reserve_above_cap_by_one_fails() {
        let t = InflightTracker::new();
        let tn = ten();
        let err = t.try_reserve_swarm(tn, 11, 10, 1).unwrap_err();
        assert_eq!(err, BackpressureKind::OpInflightFull);
        assert_eq!(t.snapshot(tn).inflight_ops, 0);
    }

    #[test]
    fn debug_format() {
        let t = InflightTracker::new();
        let _ = format!("{t:?}");
    }
}
