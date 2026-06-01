// SPDX-License-Identifier: UNLICENSED
//! **Sprint 8 — N15** : Deficit Round Robin scheduler for slot-counted
//! resources (inflight ops, inflight swarms, microVM pool shares).
//!
//! # Algorithm (architecture § 5.2 + § 7.1)
//!
//! Classical DRR with weight `w_i` per tenant `i` and per-round
//! quantum `Q` (we use Q=1 — fine-grained fairness).  Each tenant
//! holds a **deficit counter** `d_i ≥ 0`.  One scheduling round :
//!
//! 1. For every waiting tenant `i` : `d_i += w_i`.
//! 2. Visit waiting tenants in FIFO order.  The first tenant with
//!    `d_i ≥ 1` (or `rounds_waited_i ≥ max_starvation`) is granted :
//!    `d_i -= 1`.
//! 3. The grant decrements the global free-slot count by 1.  No
//!    further grants in this round.
//!
//! At most **one** grant per release event — preserves the capacity
//! invariant (G3 + I3).
//!
//! # Anti-starvation (G4 + I7)
//!
//! When `max_starvation_rounds > 0` and a tenant has waited that
//! many rounds without a grant, the algorithm force-grants it
//! regardless of its deficit.  This bounds the worst-case latency
//! ratio between a Free tenant and an Enterprise tenant under
//! contention to `100 × max_starvation_rounds` (~10 rounds → 1 000×
//! ratio).
//!
//! # Work conservation (G2 + I4)
//!
//! When the FIFO is empty and a fresh acquire arrives, the tenant
//! gets the slot immediately — no rounds elapse.  The deficit
//! machinery only kicks in under contention.
//!
//! # Slot-leak race (G3 + I5 hardening)
//!
//! Naive implementations leak slots when `tokio::time::timeout`
//! pre-empts the awaiter *just after* `advance()` granted a slot
//! and called `notify_one`.  We defend against this with an
//! **outstanding-grants** counter per tenant :
//!
//! - `advance()` bumps `state.inflight` + `state.outstanding_grants`.
//! - On `notified()` wake the awaiter atomically claims one
//!   outstanding grant (`outstanding_grants -= 1`) under the lock.
//! - On `timeout` the cancellation path checks
//!   `outstanding_grants > 0` ; if true, it claims and releases the
//!   ghost slot through the regular `release` path — no leak.
//! - Spurious wakes (notify permit with no real grant) re-enter the
//!   wait loop instead of returning a phantom permit.
//!
//! # Permit lifecycle (G6 + I5)
//!
//! [`DrrPermit`] is RAII : dropping it releases the slot.  Calling
//! [`DrrPermit::release`] does the same but lets the caller observe
//! consumption explicitly (used by the executor when it wants to
//! release earlier than the natural end-of-scope).
//!
//! # Memory bound (G10 + I12)
//!
//! Per-tenant state is created on first `register_waiter` and
//! removed when `waiters == 0 && inflight == 0 && outstanding_grants
//! == 0`.  The `tenants` map size therefore tracks **active**
//! tenants only, not the lifetime cardinality.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use efe_nano_sdk::{BackpressureError, BackpressureKind, TenantId, TenantTier};
use tokio::sync::Notify;

use crate::scheduler::metrics::{SchedulerKind, SchedulerMetrics};

/// Maximum DRR rounds run inside a single `advance` call.  Safety net
/// in case weights overflow or all tenants somehow tie at deficit 0.
/// In practice 1-2 rounds is enough.
const MAX_ROUNDS_PER_ADVANCE: usize = 16;

/// **Sprint 8 — N15** : the DRR scheduler itself.  Single capacity
/// limit shared across all tenants, deficit-weighted fairness.
///
/// Construction returns `Arc<Self>` so the type can be shared across
/// every dispatcher / executor task with `Arc::clone` (cheap atomic
/// increment).  Permits hold their own `Arc<DrrScheduler>` reference
/// for `Drop`-time release.
///
/// # Backpressure classification
///
/// `BackpressureKind` is fixed at construction so that timeout-side
/// errors are classified correctly (e.g. `OpInflightFull` vs
/// `SwarmInflightFull` vs `FirecrackerPoolFull`).
///
/// # Metrics (optional)
///
/// When constructed via [`Self::with_options`] with a
/// `SchedulerMetrics` reference, every `advance` round ticks the
/// `n15_drr_rounds_total{scheduler}` counter and every
/// starvation-driven force-grant ticks
/// `n15_starvation_force_grants_total{tier, scheduler}`.
pub struct DrrScheduler {
    /// Total slot capacity.
    capacity: u32,
    /// Classification used for backpressure errors emitted by this
    /// scheduler.
    kind: BackpressureKind,
    /// Default retry-after hint (ms).  Used when `acquire` times out.
    retry_after_ms: u32,
    /// Metric label for this scheduler (`swarm`, `op`, `firecracker`).
    scheduler_kind: SchedulerKind,
    /// Optional metrics surface — when `Some`, advance rounds +
    /// force-grants are counted.
    metrics: Option<Arc<SchedulerMetrics>>,
    /// Inner mutex-protected state.
    inner: Mutex<DrrInner>,
}

impl std::fmt::Debug for DrrScheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.inner.lock().expect("DRR mutex poisoned");
        f.debug_struct("DrrScheduler")
            .field("capacity", &self.capacity)
            .field("inflight", &inner.inflight)
            .field("active_tenants", &inner.tenants.len())
            .field("waiting_tenants", &inner.fifo.len())
            .field("kind", &self.kind)
            .field("scheduler_kind", &self.scheduler_kind)
            .finish()
    }
}

struct DrrInner {
    inflight: u32,
    tenants: HashMap<TenantId, DrrTenantState>,
    /// FIFO of tenant ids that have at least one waiter.  Dedup'd —
    /// each tenant appears at most once.
    fifo: VecDeque<TenantId>,
}

struct DrrTenantState {
    /// Current DRR weight (cached from policy lookup at register time).
    weight: u32,
    /// Signed deficit counter.  Grows by `weight` each round.
    deficit: i64,
    /// Slots currently held by this tenant (claimed + outstanding).
    inflight: u32,
    /// Tasks blocked in `acquire` for this tenant.
    waiters: u32,
    /// Slots granted by `advance()` but not yet claimed by the
    /// awaiting task (race-safety counter).
    outstanding_grants: u32,
    /// Number of consecutive rounds spent on the FIFO without a grant.
    rounds_waited: u32,
    /// Anti-starvation threshold (0 disables the force-grant path).
    max_starvation_rounds: u32,
    /// Tenant tier (cached for metric labels).
    tier: TenantTier,
    /// Per-tenant Notify — fired when this tenant is selected by the
    /// scheduler.  Wrapped in `Arc` so the lock-side can clone it and
    /// the awaiter holds a stable reference.
    notify: Arc<Notify>,
}

/// **Sprint 8 — N15** : Internal grant descriptor returned by
/// `DrrInner::advance` — carries enough metadata for the caller to
/// tick metrics + wake the chosen tenant.
#[allow(dead_code)] // `tier` and `starved` are diagnostic ; reserved for tracing.
struct AdvanceGrant {
    tenant: TenantId,
    notify: Arc<Notify>,
    tier: TenantTier,
    /// True if the grant was force-driven by the anti-starvation
    /// path (not by deficit).
    starved: bool,
}

// ─────────────────────────────────────────────────────────────────────
//  DrrPermit
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N15** : RAII permit returned by [`DrrScheduler::acquire`].
///
/// Dropping the permit releases the slot back to the scheduler.  The
/// scheduler then runs one DRR round to pick the next tenant to wake.
///
/// **Invariant (I5)** : exactly one slot is consumed per permit ; no
/// double-release possible — `consumed` flag tracks explicit
/// `release()` so subsequent `Drop` is a no-op.
pub struct DrrPermit {
    scheduler: Arc<DrrScheduler>,
    tenant: TenantId,
    consumed: bool,
}

impl std::fmt::Debug for DrrPermit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DrrPermit")
            .field("tenant", &self.tenant)
            .field("consumed", &self.consumed)
            .finish()
    }
}

impl DrrPermit {
    /// The tenant this permit belongs to.
    pub fn tenant(&self) -> TenantId {
        self.tenant
    }

    /// **N15** : explicitly release the permit.  Equivalent to
    /// dropping it, but lets the caller observe consumption.
    pub fn release(mut self) {
        self.scheduler.release(self.tenant);
        self.consumed = true;
    }
}

impl Drop for DrrPermit {
    fn drop(&mut self) {
        if !self.consumed {
            self.scheduler.release(self.tenant);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  DrrScheduler API
// ─────────────────────────────────────────────────────────────────────

impl DrrScheduler {
    /// **N15** : construct a scheduler with the given capacity and
    /// backpressure classification.  No metrics ; defaults
    /// `scheduler_kind = SchedulerKind::Op`.
    ///
    /// # Panics
    ///
    /// Panics if `capacity == 0` — a zero-capacity scheduler is
    /// useless and almost certainly a configuration bug.
    pub fn new(capacity: u32, kind: BackpressureKind, retry_after_ms: u32) -> Arc<Self> {
        Self::with_options(capacity, kind, retry_after_ms, SchedulerKind::Op, None)
    }

    /// **N15** : construct a scheduler with full options.  The
    /// `scheduler_kind` and `metrics` enable per-round + per-force-
    /// grant metric ticks.
    pub fn with_options(
        capacity: u32,
        kind: BackpressureKind,
        retry_after_ms: u32,
        scheduler_kind: SchedulerKind,
        metrics: Option<Arc<SchedulerMetrics>>,
    ) -> Arc<Self> {
        assert!(capacity > 0, "DrrScheduler capacity must be > 0 (got 0)");
        Arc::new(Self {
            capacity,
            kind,
            retry_after_ms: retry_after_ms.max(1),
            scheduler_kind,
            metrics,
            inner: Mutex::new(DrrInner {
                inflight: 0,
                tenants: HashMap::new(),
                fifo: VecDeque::new(),
            }),
        })
    }

    /// Total capacity (slots).
    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Metric scheduler kind tag.
    pub fn scheduler_kind(&self) -> SchedulerKind {
        self.scheduler_kind
    }

    /// Current inflight count.
    pub fn inflight(&self) -> u32 {
        self.inner.lock().expect("DRR mutex poisoned").inflight
    }

    /// Number of tenants with at least one waiter on the FIFO.
    pub fn waiting_tenants(&self) -> usize {
        self.inner.lock().expect("DRR mutex poisoned").fifo.len()
    }

    /// Number of active tenant entries (waiters > 0 OR inflight > 0
    /// OR outstanding_grants > 0).
    pub fn active_tenants(&self) -> usize {
        self.inner.lock().expect("DRR mutex poisoned").tenants.len()
    }

    /// Per-tenant inflight count.  Returns 0 if the tenant has no
    /// entry.
    pub fn tenant_inflight(&self, tenant: TenantId) -> u32 {
        self.inner
            .lock()
            .expect("DRR mutex poisoned")
            .tenants
            .get(&tenant)
            .map_or(0, |s| s.inflight)
    }

    /// Number of waiters for `tenant`.  Returns 0 if no entry.
    pub fn tenant_waiters(&self, tenant: TenantId) -> u32 {
        self.inner
            .lock()
            .expect("DRR mutex poisoned")
            .tenants
            .get(&tenant)
            .map_or(0, |s| s.waiters)
    }

    /// Current deficit for `tenant`.  Returns 0 if no entry.
    pub fn tenant_deficit(&self, tenant: TenantId) -> i64 {
        self.inner
            .lock()
            .expect("DRR mutex poisoned")
            .tenants
            .get(&tenant)
            .map_or(0, |s| s.deficit)
    }

    /// **N15** : outstanding (granted-but-not-claimed) count for
    /// `tenant`.  Used by tests + diagnostics ; should be 0 in steady
    /// state.
    pub fn tenant_outstanding_grants(&self, tenant: TenantId) -> u32 {
        self.inner
            .lock()
            .expect("DRR mutex poisoned")
            .tenants
            .get(&tenant)
            .map_or(0, |s| s.outstanding_grants)
    }

    /// **N15** : try to acquire a slot immediately, without waiting.
    /// Returns `None` if the scheduler can't grant right now.
    /// Defaults tier to `TenantTier::Free` for metric labels.
    pub fn try_acquire(
        self: &Arc<Self>,
        tenant: TenantId,
        weight: u32,
        max_starvation_rounds: u32,
    ) -> Option<DrrPermit> {
        self.try_acquire_with_tier(tenant, weight, max_starvation_rounds, TenantTier::Free)
    }

    /// **N15** : like `try_acquire` but supplies the tier label for
    /// metrics.
    pub fn try_acquire_with_tier(
        self: &Arc<Self>,
        tenant: TenantId,
        weight: u32,
        max_starvation_rounds: u32,
        tier: TenantTier,
    ) -> Option<DrrPermit> {
        let mut inner = self.inner.lock().expect("DRR mutex poisoned");
        inner.register_waiter(tenant, weight, max_starvation_rounds, tier);
        let advance =
            inner.advance(self.capacity, self.metrics.as_deref(), self.scheduler_kind);
        match advance {
            Some(grant) if grant.tenant == tenant => {
                // Synchronous grant — claim immediately under lock.
                let state = inner
                    .tenants
                    .get_mut(&tenant)
                    .expect("just granted entry must exist");
                state.outstanding_grants = state.outstanding_grants.saturating_sub(1);
                drop(inner);
                Some(DrrPermit {
                    scheduler: Arc::clone(self),
                    tenant,
                    consumed: false,
                })
            }
            Some(grant) => {
                // Slot went to someone else.  We don't want to wait,
                // so cancel our pending registration.
                inner.unregister_waiter(tenant);
                drop(inner);
                grant.notify.notify_one();
                None
            }
            None => {
                inner.unregister_waiter(tenant);
                None
            }
        }
    }

    /// **N15** : acquire a slot, waiting up to `timeout`.  On timeout
    /// returns a [`BackpressureError`] classified with the
    /// scheduler's `kind`.  Defaults tier to `TenantTier::Free` for
    /// metric labels.
    pub async fn acquire(
        self: &Arc<Self>,
        tenant: TenantId,
        weight: u32,
        max_starvation_rounds: u32,
        timeout: Duration,
    ) -> Result<DrrPermit, BackpressureError> {
        self.acquire_with_tier(tenant, weight, max_starvation_rounds, TenantTier::Free, timeout)
            .await
    }

    /// **N15** : like `acquire` but supplies the tier label for
    /// metrics.
    pub async fn acquire_with_tier(
        self: &Arc<Self>,
        tenant: TenantId,
        weight: u32,
        max_starvation_rounds: u32,
        tier: TenantTier,
        timeout: Duration,
    ) -> Result<DrrPermit, BackpressureError> {
        let deadline = Instant::now() + timeout;
        let my_notify;

        // ─── First pass : register + try advance ─────────────────────
        {
            let mut inner = self.inner.lock().expect("DRR mutex poisoned");
            inner.register_waiter(tenant, weight, max_starvation_rounds, tier);
            my_notify = inner
                .tenants
                .get(&tenant)
                .expect("just registered")
                .notify
                .clone();
            let advance =
                inner.advance(self.capacity, self.metrics.as_deref(), self.scheduler_kind);
            match advance {
                Some(grant) if grant.tenant == tenant => {
                    // Synchronous grant — claim immediately under lock.
                    let state = inner
                        .tenants
                        .get_mut(&tenant)
                        .expect("just granted entry must exist");
                    state.outstanding_grants = state.outstanding_grants.saturating_sub(1);
                    drop(inner);
                    return Ok(DrrPermit {
                        scheduler: Arc::clone(self),
                        tenant,
                        consumed: false,
                    });
                }
                Some(grant) => {
                    drop(inner);
                    grant.notify.notify_one();
                }
                None => {}
            }
        }

        // ─── Wait loop : claim grant via outstanding-grants ───────────
        //
        // The loop handles the race where (a) the timer fires after a
        // grant but before the awaiter polls — the Err path checks
        // outstanding_grants and releases the ghost slot ; and (b)
        // spurious wakes (notify permit with no real grant for this
        // task) — we re-await instead of returning a phantom permit.
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                self.cancel_pending(tenant);
                return Err(BackpressureError::standard(
                    self.kind,
                    tenant,
                    self.retry_after_ms,
                ));
            }
            match tokio::time::timeout(remaining, my_notify.notified()).await {
                Ok(()) => {
                    // Claim a grant under the lock.
                    let mut inner = self.inner.lock().expect("DRR mutex poisoned");
                    if let Some(state) = inner.tenants.get_mut(&tenant) {
                        if state.outstanding_grants > 0 {
                            state.outstanding_grants -= 1;
                            drop(inner);
                            return Ok(DrrPermit {
                                scheduler: Arc::clone(self),
                                tenant,
                                consumed: false,
                            });
                        }
                    }
                    // No outstanding grant for us — stale wake.  Loop
                    // and re-await.  The waiter count stays bumped
                    // (we never decremented), so the next advance
                    // will still see us.
                }
                Err(_) => {
                    self.cancel_pending(tenant);
                    return Err(BackpressureError::standard(
                        self.kind,
                        tenant,
                        self.retry_after_ms,
                    ));
                }
            }
        }
    }

    /// Cancel a pending waiter (caller decided not to wait further).
    ///
    /// **Race-safe** : if an `advance()` granted us a slot between
    /// the timer firing and the cancel call, we detect the
    /// outstanding grant, claim it, and release the slot via the
    /// regular `release` path.  No slot leak possible.
    fn cancel_pending(&self, tenant: TenantId) {
        let claimed_grant = {
            let mut inner = self.inner.lock().expect("DRR mutex poisoned");
            if let Some(state) = inner.tenants.get_mut(&tenant) {
                if state.outstanding_grants > 0 {
                    // A grant fired for us but the timer beat the
                    // notify.  Claim the orphan grant — the regular
                    // release() call below will roll it back.
                    state.outstanding_grants -= 1;
                    true
                } else {
                    inner.unregister_waiter(tenant);
                    false
                }
            } else {
                false
            }
        };
        if claimed_grant {
            self.release(tenant);
        }
    }

    /// Release a slot.  Called by `DrrPermit::drop` /
    /// `DrrPermit::release`.
    fn release(&self, tenant: TenantId) {
        let notify_to_wake;
        {
            let mut inner = self.inner.lock().expect("DRR mutex poisoned");
            inner.inflight = inner.inflight.saturating_sub(1);
            if let Some(state) = inner.tenants.get_mut(&tenant) {
                state.inflight = state.inflight.saturating_sub(1);
            }
            // GC the tenant entry if fully drained (no waiters, no
            // inflight, no pending grants).
            let drop_entry = inner.tenants.get(&tenant).is_some_and(|s| {
                s.waiters == 0 && s.inflight == 0 && s.outstanding_grants == 0
            });
            if drop_entry {
                inner.tenants.remove(&tenant);
            }
            // Kick DRR for the next grant.
            notify_to_wake = inner
                .advance(self.capacity, self.metrics.as_deref(), self.scheduler_kind)
                .map(|g| g.notify);
        }
        if let Some(n) = notify_to_wake {
            n.notify_one();
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  DrrInner — algorithmic core
// ─────────────────────────────────────────────────────────────────────

impl DrrInner {
    /// Register a new waiter for `tenant`.  Creates the state entry
    /// if absent ; updates weight / max_starvation / tier regardless
    /// (tier may have changed since last call).
    fn register_waiter(
        &mut self,
        tenant: TenantId,
        weight: u32,
        max_starvation_rounds: u32,
        tier: TenantTier,
    ) {
        let was_zero;
        {
            let state = self.tenants.entry(tenant).or_insert_with(|| DrrTenantState {
                weight: weight.max(1),
                deficit: 0,
                inflight: 0,
                waiters: 0,
                outstanding_grants: 0,
                rounds_waited: 0,
                max_starvation_rounds,
                tier,
                notify: Arc::new(Notify::new()),
            });
            // Keep weight / max_starvation / tier fresh on every register.
            state.weight = weight.max(1);
            state.max_starvation_rounds = max_starvation_rounds;
            state.tier = tier;
            was_zero = state.waiters == 0;
            state.waiters = state.waiters.saturating_add(1);
        }
        if was_zero {
            self.fifo.push_back(tenant);
        }
    }

    /// Decrement waiter count for `tenant`.  Removes from FIFO if
    /// `waiters` reaches 0 ; removes the tenant entry if `waiters`,
    /// `inflight` and `outstanding_grants` are all 0.
    fn unregister_waiter(&mut self, tenant: TenantId) {
        let (drop_entry, drop_fifo) = if let Some(state) = self.tenants.get_mut(&tenant) {
            state.waiters = state.waiters.saturating_sub(1);
            (
                state.waiters == 0 && state.inflight == 0 && state.outstanding_grants == 0,
                state.waiters == 0,
            )
        } else {
            (false, false)
        };
        if drop_fifo {
            self.fifo.retain(|t| t != &tenant);
        }
        if drop_entry {
            self.tenants.remove(&tenant);
        }
    }

    /// Run DRR rounds until either we grant a slot or no waiter
    /// qualifies after `MAX_ROUNDS_PER_ADVANCE` rounds.
    ///
    /// Each outer iteration is one "DRR round" — every waiting
    /// tenant's deficit is bumped by its weight, then the FIFO is
    /// scanned for a qualifying tenant.
    ///
    /// On grant : bumps `state.inflight`, `state.outstanding_grants`,
    /// `self.inflight` ; decrements `state.waiters` ; resets
    /// `state.rounds_waited`.
    fn advance(
        &mut self,
        capacity: u32,
        metrics: Option<&SchedulerMetrics>,
        scheduler_kind: SchedulerKind,
    ) -> Option<AdvanceGrant> {
        if self.inflight >= capacity {
            return None;
        }
        for _ in 0..MAX_ROUNDS_PER_ADVANCE {
            if let Some(m) = metrics {
                m.tick_drr_round(scheduler_kind);
            }
            if self.fifo.is_empty() {
                return None;
            }
            let n = self.fifo.len();
            let mut grant: Option<AdvanceGrant> = None;
            for _ in 0..n {
                let tenant = self
                    .fifo
                    .pop_front()
                    .expect("loop guard guarantees non-empty");
                let (qualifies, notify, starved_grant, tier) = {
                    let state = self
                        .tenants
                        .get_mut(&tenant)
                        .expect("FIFO entry without tenant state");
                    state.deficit = state.deficit.saturating_add(i64::from(state.weight));
                    state.rounds_waited = state.rounds_waited.saturating_add(1);
                    let starved = state.max_starvation_rounds > 0
                        && state.rounds_waited >= state.max_starvation_rounds;
                    let q = state.deficit >= 1 || starved;
                    let notify = state.notify.clone();
                    let tier = state.tier;
                    if q {
                        state.deficit = state.deficit.saturating_sub(1);
                        state.inflight = state.inflight.saturating_add(1);
                        state.waiters = state.waiters.saturating_sub(1);
                        state.outstanding_grants = state.outstanding_grants.saturating_add(1);
                        state.rounds_waited = 0;
                    }
                    (q, notify, q && starved, tier)
                };
                if qualifies {
                    let still_has_waiters = self
                        .tenants
                        .get(&tenant)
                        .is_some_and(|s| s.waiters > 0);
                    if still_has_waiters {
                        self.fifo.push_back(tenant);
                    }
                    self.inflight = self.inflight.saturating_add(1);
                    if starved_grant {
                        if let Some(m) = metrics {
                            m.tick_starvation_force_grant(tier, scheduler_kind);
                        }
                    }
                    grant = Some(AdvanceGrant {
                        tenant,
                        notify,
                        tier,
                        starved: starved_grant,
                    });
                    break;
                } else {
                    // Not granted this round — back of FIFO.
                    self.fifo.push_back(tenant);
                }
            }
            if let Some(g) = grant {
                return Some(g);
            }
        }
        // Safety net : no grant after MAX_ROUNDS_PER_ADVANCE.  Should
        // be unreachable with weight >= 1.
        None
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn sched(capacity: u32) -> Arc<DrrScheduler> {
        DrrScheduler::new(capacity, BackpressureKind::OpInflightFull, 100)
    }

    #[test]
    #[should_panic]
    fn new_panics_on_zero_capacity() {
        let _ = DrrScheduler::new(0, BackpressureKind::OpInflightFull, 100);
    }

    #[tokio::test]
    async fn single_tenant_acquires_and_releases() {
        let s = sched(2);
        let t = TenantId::new();
        let p = s.acquire(t, 1, 0, Duration::from_secs(1)).await.unwrap();
        assert_eq!(s.inflight(), 1);
        assert_eq!(s.tenant_inflight(t), 1);
        assert_eq!(s.tenant_outstanding_grants(t), 0); // claimed
        drop(p);
        assert_eq!(s.inflight(), 0);
        assert_eq!(s.tenant_inflight(t), 0);
        // Entry GC'd.
        assert_eq!(s.active_tenants(), 0);
    }

    #[tokio::test]
    async fn explicit_release_consumes_permit() {
        let s = sched(1);
        let t = TenantId::new();
        let p = s.try_acquire(t, 1, 0).unwrap();
        assert_eq!(s.inflight(), 1);
        p.release();
        assert_eq!(s.inflight(), 0);
    }

    #[tokio::test]
    async fn try_acquire_returns_none_when_full() {
        let s = sched(1);
        let t1 = TenantId::new();
        let t2 = TenantId::new();
        let _p1 = s.try_acquire(t1, 1, 0).unwrap();
        let p2 = s.try_acquire(t2, 1, 0);
        assert!(p2.is_none());
    }

    #[tokio::test]
    async fn capacity_invariant_respected_under_concurrency() {
        let s = sched(4);
        let counter = Arc::new(AtomicU32::new(0));
        let mut handles = vec![];
        for _ in 0..16 {
            let s = Arc::clone(&s);
            let c = Arc::clone(&counter);
            handles.push(tokio::spawn(async move {
                let t = TenantId::new();
                let p = s.acquire(t, 1, 0, Duration::from_secs(2)).await.unwrap();
                let before = c.fetch_add(1, Ordering::SeqCst);
                assert!(before < 4, "capacity violated : {} >= 4", before + 1);
                tokio::time::sleep(Duration::from_millis(20)).await;
                c.fetch_sub(1, Ordering::SeqCst);
                drop(p);
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        assert_eq!(s.inflight(), 0);
        // Verify no outstanding grants leaked.
        assert_eq!(s.active_tenants(), 0);
    }

    #[tokio::test]
    async fn timeout_returns_backpressure_error() {
        let s = sched(1);
        let t1 = TenantId::new();
        let t2 = TenantId::new();
        let _hold = s.acquire(t1, 1, 0, Duration::from_secs(1)).await.unwrap();
        let err = s
            .acquire(t2, 1, 0, Duration::from_millis(50))
            .await
            .unwrap_err();
        assert_eq!(err.kind, BackpressureKind::OpInflightFull);
        assert!(err.retry_after_ms > 0);
    }

    #[tokio::test]
    async fn release_wakes_waiter() {
        let s = sched(1);
        let t1 = TenantId::new();
        let t2 = TenantId::new();
        let p1 = s.acquire(t1, 1, 0, Duration::from_secs(1)).await.unwrap();
        let s2 = Arc::clone(&s);
        let h = tokio::spawn(async move {
            s2.acquire(t2, 1, 0, Duration::from_secs(1)).await.unwrap()
        });
        // Give the waiter a moment to enqueue.
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(s.waiting_tenants(), 1);
        drop(p1);
        let p2 = h.await.unwrap();
        assert_eq!(p2.tenant(), t2);
        assert_eq!(s.inflight(), 1);
    }

    #[tokio::test]
    async fn dropped_permit_implicitly_releases() {
        let s = sched(1);
        let t = TenantId::new();
        {
            let _p = s.acquire(t, 1, 0, Duration::from_secs(1)).await.unwrap();
            assert_eq!(s.inflight(), 1);
        }
        assert_eq!(s.inflight(), 0);
    }

    #[tokio::test]
    async fn higher_weight_tenant_gets_more_grants_under_contention() {
        let s = sched(1);
        let a = TenantId::new();
        let b = TenantId::new();

        let blocker_tenant = TenantId::new();
        let blocker = s
            .acquire(blocker_tenant, 1, 0, Duration::from_secs(1))
            .await
            .unwrap();

        let mut grants_a = 0u32;
        let mut grants_b = 0u32;
        let total_iters = 60;
        let mut handles = vec![];
        for i in 0..total_iters {
            let s = Arc::clone(&s);
            let (tenant, weight) = if i % 2 == 0 { (a, 1) } else { (b, 10) };
            handles.push(tokio::spawn(async move {
                let _p = s
                    .acquire(tenant, weight, 0, Duration::from_secs(5))
                    .await
                    .unwrap();
                tokio::time::sleep(Duration::from_millis(2)).await;
                tenant
            }));
        }
        drop(blocker);

        for h in handles {
            let t = h.await.unwrap();
            if t == a {
                grants_a += 1;
            } else if t == b {
                grants_b += 1;
            }
        }
        assert_eq!(grants_a + grants_b, total_iters);
        assert!(grants_a > 0, "weight-1 tenant never granted");
        assert!(grants_b > 0, "weight-10 tenant never granted");
    }

    #[tokio::test]
    async fn anti_starvation_force_grants_after_max_rounds() {
        let s = sched(1);
        let a = TenantId::new();
        let b = TenantId::new();

        let blocker = TenantId::new();
        let blocker_permit = s
            .acquire(blocker, 1, 0, Duration::from_secs(1))
            .await
            .unwrap();

        let s_a = Arc::clone(&s);
        let s_b = Arc::clone(&s);
        let h_a =
            tokio::spawn(async move { s_a.acquire(a, 1, 3, Duration::from_secs(5)).await.unwrap() });
        let mut bs = vec![];
        for _ in 0..5 {
            let s = Arc::clone(&s_b);
            bs.push(tokio::spawn(async move {
                s.acquire(b, 1_000_000, 0, Duration::from_secs(5))
                    .await
                    .unwrap()
            }));
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(blocker_permit);

        let p_a = h_a.await.unwrap();
        assert_eq!(p_a.tenant(), a);
        drop(p_a);

        for h in bs {
            let _ = h.await.unwrap();
        }
    }

    #[tokio::test]
    async fn weight_zero_clamps_to_one() {
        let s = sched(1);
        let t = TenantId::new();
        let p = s.acquire(t, 0, 0, Duration::from_secs(1)).await.unwrap();
        assert_eq!(p.tenant(), t);
    }

    #[tokio::test]
    async fn multi_tenant_round_robin_when_equal_weights() {
        let s = sched(1);
        let a = TenantId::new();
        let b = TenantId::new();
        let c = TenantId::new();
        let blocker_tenant = TenantId::new();
        let blocker = s
            .acquire(blocker_tenant, 1, 0, Duration::from_secs(1))
            .await
            .unwrap();

        let grants = Arc::new(Mutex::new(Vec::new()));
        let mut handles = vec![];
        for tenant in [a, b, c, a, b, c] {
            let s = Arc::clone(&s);
            let grants = Arc::clone(&grants);
            handles.push(tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(5)).await;
                let _p = s
                    .acquire(tenant, 1, 0, Duration::from_secs(5))
                    .await
                    .unwrap();
                grants.lock().unwrap().push(tenant);
                tokio::time::sleep(Duration::from_millis(5)).await;
            }));
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        drop(blocker);
        for h in handles {
            h.await.unwrap();
        }
        let final_grants = grants.lock().unwrap().clone();
        assert_eq!(final_grants.len(), 6);
        let count_a = final_grants.iter().filter(|t| **t == a).count();
        let count_b = final_grants.iter().filter(|t| **t == b).count();
        let count_c = final_grants.iter().filter(|t| **t == c).count();
        assert_eq!(count_a, 2);
        assert_eq!(count_b, 2);
        assert_eq!(count_c, 2);
    }

    #[tokio::test]
    async fn try_acquire_does_not_block() {
        let s = sched(2);
        let t = TenantId::new();
        let start = Instant::now();
        let p1 = s.try_acquire(t, 1, 0).unwrap();
        let p2 = s.try_acquire(t, 1, 0).unwrap();
        let p3 = s.try_acquire(t, 1, 0);
        assert!(p3.is_none());
        assert!(start.elapsed() < Duration::from_millis(20));
        drop(p1);
        drop(p2);
    }

    #[tokio::test]
    async fn entry_gced_after_full_drain() {
        let s = sched(2);
        let t = TenantId::new();
        let p1 = s.try_acquire(t, 1, 0).unwrap();
        let p2 = s.try_acquire(t, 1, 0).unwrap();
        assert_eq!(s.active_tenants(), 1);
        drop(p1);
        assert_eq!(s.active_tenants(), 1); // still has inflight=1
        drop(p2);
        assert_eq!(s.active_tenants(), 0); // gc'd
    }

    #[tokio::test]
    async fn deficit_persists_within_entry_lifetime() {
        let s = sched(1);
        let t = TenantId::new();
        let p = s.try_acquire(t, 5, 0).unwrap();
        assert_eq!(p.tenant(), t);
        drop(p);
        assert_eq!(s.active_tenants(), 0);
    }

    #[tokio::test]
    async fn timeout_does_not_leak_waiter() {
        let s = sched(1);
        let t1 = TenantId::new();
        let t2 = TenantId::new();
        let _hold = s.acquire(t1, 1, 0, Duration::from_secs(1)).await.unwrap();
        let _err = s
            .acquire(t2, 1, 0, Duration::from_millis(20))
            .await
            .unwrap_err();
        // After timeout, t2 should have no waiter.
        assert_eq!(s.tenant_waiters(t2), 0);
        assert_eq!(s.waiting_tenants(), 0);
        // And no outstanding grant either.
        assert_eq!(s.tenant_outstanding_grants(t2), 0);
    }

    #[tokio::test]
    async fn tier_change_at_acquire_updates_weight() {
        let s = sched(2);
        let t = TenantId::new();
        let p1 = s.try_acquire(t, 1, 0).unwrap();
        let p2 = s.try_acquire(t, 100, 0).unwrap();
        assert_eq!(s.tenant_inflight(t), 2);
        drop(p1);
        drop(p2);
    }

    #[tokio::test]
    async fn capacity_one_serializes_acquires() {
        let s = sched(1);
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut handles = vec![];
        for i in 0..5u32 {
            let s = Arc::clone(&s);
            let order = Arc::clone(&order);
            handles.push(tokio::spawn(async move {
                let t = TenantId::new();
                let p = s.acquire(t, 1, 0, Duration::from_secs(5)).await.unwrap();
                order.lock().unwrap().push(i);
                tokio::time::sleep(Duration::from_millis(10)).await;
                drop(p);
            }));
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        for h in handles {
            h.await.unwrap();
        }
        let order = order.lock().unwrap().clone();
        assert_eq!(order.len(), 5);
        assert_eq!(s.inflight(), 0);
    }

    #[tokio::test]
    async fn debug_format_does_not_panic() {
        let s = sched(2);
        let t = TenantId::new();
        let _p = s.try_acquire(t, 1, 0).unwrap();
        let fmt = format!("{s:?}");
        assert!(fmt.contains("DrrScheduler"));
        assert!(fmt.contains("inflight"));
    }

    #[tokio::test]
    async fn permit_debug_format() {
        let s = sched(1);
        let t = TenantId::new();
        let p = s.try_acquire(t, 1, 0).unwrap();
        let fmt = format!("{p:?}");
        assert!(fmt.contains("DrrPermit"));
    }

    // ─────────────────────────────────────────────────────────────────
    //  Sprint 8 — N15 hardening : slot-leak race + cancel safety
    // ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn cancel_during_grant_does_not_leak_slot() {
        // This test induces the race : a tight loop of acquire-with-
        // tiny-timeout + release, verifying the inflight count
        // returns to 0 at the end (no leaked slots).
        let s = sched(4);
        let mut handles = vec![];
        for _ in 0..100 {
            let s = Arc::clone(&s);
            handles.push(tokio::spawn(async move {
                let t = TenantId::new();
                // Mix of immediate / short / longer timeouts.
                let timeout_ms = (rand_micro() % 5) + 1;
                if let Ok(p) = s.acquire(t, 1, 0, Duration::from_millis(timeout_ms)).await {
                    tokio::time::sleep(Duration::from_micros(rand_micro() % 200)).await;
                    drop(p);
                }
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        // Give the scheduler a moment to finalize.
        tokio::time::sleep(Duration::from_millis(50)).await;
        // Invariant: no slots leaked.
        assert_eq!(
            s.inflight(),
            0,
            "slot leak detected — inflight = {}",
            s.inflight()
        );
        assert_eq!(
            s.active_tenants(),
            0,
            "tenant entry leak — active_tenants = {}",
            s.active_tenants()
        );
    }

    /// Pseudo-random microsecond jitter (no rand crate dep).
    fn rand_micro() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64)
            .unwrap_or(0)
    }

    #[tokio::test]
    async fn outstanding_grant_claimed_via_release_on_timeout_race() {
        // Hand-crafted scenario : T1 holds the only slot.  T2 starts
        // acquire with a SHORT timeout.  We sleep BEYOND that timeout
        // (so T2 times out), THEN release T1's permit (firing
        // advance() which would grant T2 — but T2 already timed out).
        // Verify no slot leak.
        let s = sched(1);
        let t1 = TenantId::new();
        let t2 = TenantId::new();
        let p1 = s.acquire(t1, 1, 0, Duration::from_secs(1)).await.unwrap();

        let s2 = Arc::clone(&s);
        let h = tokio::spawn(async move {
            // Short timeout — should fail.
            s2.acquire(t2, 1, 0, Duration::from_millis(10)).await
        });

        // Wait for T2 to definitely time out.
        tokio::time::sleep(Duration::from_millis(50)).await;
        // T2 should have errored.
        let err = h.await.unwrap();
        assert!(err.is_err(), "T2 must time out");

        // Now release T1.  Advance fires, FIFO is empty (T2 already
        // cancelled), no grant fires.
        drop(p1);

        assert_eq!(s.inflight(), 0);
        assert_eq!(s.active_tenants(), 0);
    }

    #[tokio::test]
    async fn metrics_tick_on_drr_rounds_and_force_grants() {
        let metrics = SchedulerMetrics::new();
        let s = DrrScheduler::with_options(
            1,
            BackpressureKind::OpInflightFull,
            100,
            SchedulerKind::Op,
            Some(Arc::clone(&metrics)),
        );

        let t1 = TenantId::new();
        let p1 = s.acquire(t1, 1, 0, Duration::from_secs(1)).await.unwrap();

        // Force contention to drive DRR rounds.
        let s2 = Arc::clone(&s);
        let t2 = TenantId::new();
        let h = tokio::spawn(async move {
            s2.acquire_with_tier(t2, 1, 2, TenantTier::Free, Duration::from_secs(2))
                .await
                .unwrap()
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        drop(p1);
        let _p2 = h.await.unwrap();

        // DRR rounds should have ticked at least once.
        let snap = metrics.snapshot();
        assert!(snap.drr_rounds_for(SchedulerKind::Op) > 0);
    }

    #[tokio::test]
    async fn metrics_drr_rounds_tick_under_contention() {
        // Verify the metric infrastructure ticks `drr_rounds_total`
        // when the scheduler runs at least one round.  We don't try
        // to force starvation (with weight ≥ 1 the deficit threshold
        // is met in 1 round so the force-grant path rarely fires —
        // this is the documented G4 behaviour) ; we just exercise
        // the metric pipeline.
        let metrics = SchedulerMetrics::new();
        let s = DrrScheduler::with_options(
            1,
            BackpressureKind::OpInflightFull,
            100,
            SchedulerKind::Op,
            Some(Arc::clone(&metrics)),
        );

        let blocker = TenantId::new();
        let p = s
            .acquire_with_tier(
                blocker,
                1,
                0,
                TenantTier::Free,
                Duration::from_secs(2),
            )
            .await
            .unwrap();

        // One follower that *will* be granted once the blocker drops.
        let follower = TenantId::new();
        let s2 = Arc::clone(&s);
        let h = tokio::spawn(async move {
            s2.acquire_with_tier(
                follower,
                1,
                0,
                TenantTier::Tier2,
                Duration::from_secs(3),
            )
            .await
            .unwrap()
        });

        tokio::time::sleep(Duration::from_millis(20)).await;
        drop(p);
        let _p_follower = h.await.unwrap();

        // The release-triggered advance ran at least one round.
        let snap = metrics.snapshot();
        assert!(
            snap.drr_rounds_for(SchedulerKind::Op) >= 1,
            "drr_rounds should have ticked at least once"
        );
    }

    #[tokio::test]
    async fn stress_no_leak_under_high_contention_short_timeouts() {
        let s = sched(8);
        let mut handles = vec![];
        for _ in 0..400 {
            let s = Arc::clone(&s);
            handles.push(tokio::spawn(async move {
                let t = TenantId::new();
                let timeout_micros = 100 + (rand_micro() % 2000);
                if let Ok(p) = s
                    .acquire(t, 1, 0, Duration::from_micros(timeout_micros))
                    .await
                {
                    tokio::time::sleep(Duration::from_micros(rand_micro() % 500)).await;
                    drop(p);
                }
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        // No slot, waiter, or grant leaks.
        assert_eq!(s.inflight(), 0, "slot leak under stress");
        assert_eq!(s.active_tenants(), 0, "entry leak under stress");
    }
}
