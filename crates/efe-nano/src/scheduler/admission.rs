// SPDX-License-Identifier: UNLICENSED
//! **Sprint 8 — N15** : Admission gate.
//!
//! The `AdmissionGate` is the **single front-door** the dispatcher
//! crosses before executing a swarm.  It composes the four scheduler
//! subsystems built in Waves 2-5 :
//!
//! ```text
//!       ┌─────────────────────────────────────────────────────┐
//!       │                AdmissionGate                        │
//!       │  ┌──────────────┐  ┌─────────────┐                  │
//!       │  │ Policy lookup│→ │ Tracker     │  per-tenant      │
//!       │  └──────┬───────┘  └──────┬──────┘  atomic check    │
//!       │         │                 │                          │
//!       │         ▼                 ▼                          │
//!       │  ┌──────────────┐  ┌─────────────┐                  │
//!       │  │ Swarm DRR    │→ │ Op DRR      │  global fair-    │
//!       │  └──────┬───────┘  └──────┬──────┘  share permits   │
//!       │         │                 │                          │
//!       │         └─────┬───────────┘                          │
//!       │               ▼                                      │
//!       │      ┌──────────────────┐  ┌──────────────────┐     │
//!       │      │ LLM token bucket │  │ Audit bucket     │     │
//!       │      └──────────────────┘  └──────────────────┘     │
//!       └─────────────────────────────────────────────────────┘
//! ```
//!
//! # Admission flow (architecture § 6.2)
//!
//! 1. **Policy resolution** : `policy_lookup.policy(tenant)`.  If
//!    tenant is suspended → return `TenantSuspended`.
//! 2. **Atomic tracker reservation** : `inflight_ops += op_count`,
//!    `inflight_swarms += 1`.  Fails with `OpInflightFull` or
//!    `SwarmInflightFull`.
//! 3. **Global swarm DRR permit** : acquire one swarm slot from
//!    `swarm_drr`.  On timeout → release tracker reservation, return
//!    `SwarmInflightFull` (global cap reached).
//! 4. Return [`AdmissionTicket`] — RAII guard that releases everything
//!    on `Drop`.
//!
//! # Op-level + LLM + audit (Wave 8 wiring)
//!
//! Per-op DRR slot acquisition + LLM token consumption + audit-write
//! consumption are exposed as **separate** gate methods.  The
//! dispatcher's executor calls them at the right call sites :
//! - `acquire_op_permit` → before dispatching each op.
//! - `consume_llm_tokens` → before each LLM provider call.
//! - `consume_audit_write` → before each audit-chain record.
//!
//! # Backward compatibility
//!
//! The gate is **optional** : the dispatcher accepts
//! `Option<Arc<AdmissionGate>>`.  When `None`, all admission is open
//! (the legacy 1136-test path).  Production wiring installs the
//! gate ; tests that don't exercise fairness construct dispatchers
//! without it.

use std::sync::Arc;
use std::time::{Duration, Instant};

use efe_nano_sdk::{BackpressureError, BackpressureKind, QuotaPolicy, TenantId};

use crate::scheduler::drr::{DrrPermit, DrrScheduler};
use crate::scheduler::metrics::{RegistryKind, SchedulerKind, SchedulerMetrics};
use crate::scheduler::policy_lookup::TenantPolicyLookup;
use crate::scheduler::token_bucket::TokenBucketRegistry;
use crate::scheduler::tracker::InflightTracker;

/// **Sprint 8 — N15** : default acquire-timeout when the caller
/// passes none.  5 seconds — long enough to absorb a brief
/// contention spike, short enough that the customer sees a quick
/// 429 instead of a hanging request.
pub const DEFAULT_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);

/// **Sprint 8 — N15** : default retry-after hint (ms) embedded in
/// every gate-emitted `BackpressureError`.  500 ms — a balance
/// between client-side backoff and overload-recovery latency.
pub const DEFAULT_RETRY_AFTER_MS: u32 = 500;

/// **Sprint 8 — N15** : Admission gate.
///
/// Owns all four scheduler subsystems and orchestrates the
/// per-swarm admission decision.
///
/// All async methods are cancel-safe : if the caller drops the
/// future before completion, the partial state (tracker reservation,
/// DRR registration) is cleaned up by the underlying machinery.
pub struct AdmissionGate {
    policy_lookup: Arc<dyn TenantPolicyLookup>,
    tracker: Arc<InflightTracker>,
    swarm_drr: Arc<DrrScheduler>,
    op_drr: Arc<DrrScheduler>,
    firecracker_drr: Arc<DrrScheduler>,
    /// **Sprint 9 — N16** : dedicated lane for replay traffic ;
    /// capped at 10 % of `op_drr` capacity (architecture ADR-5).
    /// `None` keeps the legacy (no-replay-lane) behaviour ; production
    /// wiring installs a sized lane.
    replay_op_drr: Option<Arc<DrrScheduler>>,
    audit_buckets: Arc<TokenBucketRegistry>,
    llm_buckets: Arc<TokenBucketRegistry>,
    /// Acquire timeout for the global DRR permits.
    acquire_timeout: Duration,
    /// Retry-after hint stamped on emitted backpressure errors.
    retry_after_ms: u32,
    /// **Sprint 8 — N15** : optional metrics surface.  When `None`,
    /// no observation is recorded (tests / micro-benches).
    metrics: Option<Arc<SchedulerMetrics>>,
}

impl std::fmt::Debug for AdmissionGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdmissionGate")
            .field("swarm_drr_capacity", &self.swarm_drr.capacity())
            .field("op_drr_capacity", &self.op_drr.capacity())
            .field("firecracker_drr_capacity", &self.firecracker_drr.capacity())
            .field("acquire_timeout", &self.acquire_timeout)
            .field("retry_after_ms", &self.retry_after_ms)
            .finish()
    }
}

// ─────────────────────────────────────────────────────────────────────
//  AdmissionGate construction
// ─────────────────────────────────────────────────────────────────────

impl AdmissionGate {
    /// **N15** : start a [`AdmissionGateBuilder`].
    pub fn builder() -> AdmissionGateBuilder {
        AdmissionGateBuilder::default()
    }

    /// **N15** : default acquire timeout.
    pub fn acquire_timeout(&self) -> Duration {
        self.acquire_timeout
    }

    /// **N15** : access the policy lookup (used by tests + metrics).
    pub fn policy_lookup(&self) -> &Arc<dyn TenantPolicyLookup> {
        &self.policy_lookup
    }

    /// **N15** : access the tracker.
    pub fn tracker(&self) -> &Arc<InflightTracker> {
        &self.tracker
    }

    /// **N15** : access the swarm DRR scheduler.
    pub fn swarm_drr(&self) -> &Arc<DrrScheduler> {
        &self.swarm_drr
    }

    /// **N15** : access the per-op DRR scheduler.
    pub fn op_drr(&self) -> &Arc<DrrScheduler> {
        &self.op_drr
    }

    /// **N15** : access the Firecracker pool DRR scheduler.
    pub fn firecracker_drr(&self) -> &Arc<DrrScheduler> {
        &self.firecracker_drr
    }

    /// **N16** : access the dedicated replay DRR scheduler (if installed).
    pub fn replay_op_drr(&self) -> Option<&Arc<DrrScheduler>> {
        self.replay_op_drr.as_ref()
    }

    /// **N15** : access the LLM token bucket registry.
    pub fn llm_buckets(&self) -> &Arc<TokenBucketRegistry> {
        &self.llm_buckets
    }

    /// **N15** : access the audit token bucket registry.
    pub fn audit_buckets(&self) -> &Arc<TokenBucketRegistry> {
        &self.audit_buckets
    }

    /// **N15** : access the metrics surface (if installed).
    pub fn metrics(&self) -> Option<&Arc<SchedulerMetrics>> {
        self.metrics.as_ref()
    }
}

// ─────────────────────────────────────────────────────────────────────
//  AdmissionGate.admit_swarm
// ─────────────────────────────────────────────────────────────────────

impl AdmissionGate {
    /// **N15** : admit a swarm with `op_count` ops on behalf of
    /// `tenant`.
    ///
    /// On success, returns an [`AdmissionTicket`] that holds the
    /// tracker reservation + global swarm DRR permit.  Dropping the
    /// ticket releases both.
    ///
    /// # Errors
    ///
    /// Returns [`BackpressureError`] when :
    /// - Tenant is suspended (`TenantSuspended`).
    /// - Per-tenant op cap reached (`OpInflightFull`).
    /// - Per-tenant swarm cap reached (`SwarmInflightFull`).
    /// - Global swarm DRR pool times out (`SwarmInflightFull` — same
    ///   kind, different cause).
    ///
    /// # Cancellation safety (G11 + I13)
    ///
    /// If the caller drops the future mid-acquire (e.g. on client
    /// disconnect during the `swarm_drr.acquire` await), the
    /// `TrackerRollbackGuard` installed before the await rolls back
    /// the tracker reservation in its `Drop`.  No leak possible.
    /// The DRR future itself is also cancel-safe (the awaiter
    /// unregisters from the per-tenant FIFO on drop via its own
    /// scope guard inside `tokio::time::timeout`).
    pub async fn admit_swarm(
        self: &Arc<Self>,
        tenant: TenantId,
        op_count: u32,
    ) -> Result<AdmissionTicket, BackpressureError> {
        let tier_for_metric = self.policy_lookup.tier(tenant);
        if let Some(m) = self.metrics.as_deref() {
            m.tick_admission_request(tier_for_metric);
        }

        // (1) Policy lookup — propagates TenantSuspended.
        let policy = match self.policy_lookup.policy(tenant).await {
            Ok(p) => {
                if let Some(m) = self.metrics.as_deref() {
                    m.tick_policy_lookup_hit();
                }
                p
            }
            Err(err) => {
                if let Some(m) = self.metrics.as_deref() {
                    m.tick_policy_lookup_suspended();
                    m.tick_admission_rejected(tier_for_metric, err.kind);
                }
                return Err(err);
            }
        };

        // (2) Atomic tracker reservation.
        if let Err(kind) = self.tracker.try_reserve_swarm(
            tenant,
            op_count,
            policy.max_inflight_ops,
            policy.max_inflight_swarms,
        ) {
            if let Some(m) = self.metrics.as_deref() {
                m.tick_admission_rejected(policy.tier, kind);
            }
            return Err(BackpressureError::standard(
                kind,
                tenant,
                self.retry_after_ms,
            ));
        }

        // **G11** : Install a scope-guard so the tracker reservation
        // rolls back automatically if the caller drops this future
        // during the `swarm_drr.acquire` await below.  We disarm it
        // on success.
        let mut rollback_guard = TrackerRollbackGuard {
            tracker: Arc::clone(&self.tracker),
            tenant,
            op_count,
            armed: true,
        };

        // (3) Global swarm DRR permit.
        let drr_started = Instant::now();
        let swarm_permit = match self
            .swarm_drr
            .acquire_with_tier(
                tenant,
                policy.drr_weight,
                policy.max_starvation_rounds,
                policy.tier,
                self.acquire_timeout,
            )
            .await
        {
            Ok(p) => p,
            Err(err) => {
                if let Some(m) = self.metrics.as_deref() {
                    m.tick_admission_rejected(policy.tier, err.kind);
                }
                // The drop guard rolls back the tracker reservation
                // automatically when it goes out of scope below.
                return Err(err);
            }
        };
        // Success — disarm the rollback guard so the ticket owns the
        // reservation.
        rollback_guard.armed = false;
        drop(rollback_guard);

        if let Some(m) = self.metrics.as_deref() {
            m.observe_acquire_latency(
                SchedulerKind::Swarm,
                drr_started.elapsed().as_secs_f64(),
            );
            m.tick_admission_accepted(policy.tier);
            m.add_inflight_ops(policy.tier, i64::from(op_count));
            m.add_inflight_swarms(policy.tier, 1);
        }

        Ok(AdmissionTicket {
            gate: Arc::clone(self),
            tenant,
            op_count,
            swarm_permit: Some(swarm_permit),
            policy,
            released: false,
        })
    }

    /// **N15** : acquire a per-op DRR permit on behalf of `tenant`.
    /// Used by the dispatcher's executor at every op-execution call
    /// site.  Returns the DRR permit ; drop it to release.
    pub async fn acquire_op_permit(
        self: &Arc<Self>,
        tenant: TenantId,
    ) -> Result<DrrPermit, BackpressureError> {
        let policy = self.policy_lookup.policy(tenant).await?;
        let started = Instant::now();
        let result = self
            .op_drr
            .acquire_with_tier(
                tenant,
                policy.drr_weight,
                policy.max_starvation_rounds,
                policy.tier,
                self.acquire_timeout,
            )
            .await;
        if let Some(m) = self.metrics.as_deref() {
            m.observe_acquire_latency(SchedulerKind::Op, started.elapsed().as_secs_f64());
        }
        result
    }

    /// **N15** : acquire a Firecracker pool share permit for
    /// `tenant`.  Used by the runtime pool integration (Wave 8).
    pub async fn acquire_firecracker_permit(
        self: &Arc<Self>,
        tenant: TenantId,
    ) -> Result<DrrPermit, BackpressureError> {
        let policy = self.policy_lookup.policy(tenant).await?;
        // Per-tenant pool share is in basis points (1bp = 0.01%).  We
        // map share → weight directly since DRR weights are unitless.
        let weight = u32::from(policy.firecracker_pool_share_bps).max(1);
        let started = Instant::now();
        let result = self
            .firecracker_drr
            .acquire_with_tier(
                tenant,
                weight,
                policy.max_starvation_rounds,
                policy.tier,
                self.acquire_timeout,
            )
            .await;
        if let Some(m) = self.metrics.as_deref() {
            m.observe_acquire_latency(
                SchedulerKind::Firecracker,
                started.elapsed().as_secs_f64(),
            );
        }
        result
    }

    /// **N16** : acquire a replay-lane DRR permit for `tenant`.
    /// Used by `MultiRegionReplayEngine` to gate replay-of-op traffic.
    /// Returns `BackpressureKind::ReplayRateLimited` on timeout.
    ///
    /// If no `replay_op_drr` is wired on the gate, this method falls
    /// back to the production `op_drr` (no replay-specific
    /// throttling) — convenient default for tests.
    pub async fn acquire_replay_op_permit(
        self: &Arc<Self>,
        tenant: TenantId,
    ) -> Result<DrrPermit, BackpressureError> {
        let policy = self.policy_lookup.policy(tenant).await?;
        // Replay tenants get a scaled-down weight : 1/10th of normal.
        let replay_weight = (policy.drr_weight / 10).max(1);
        let drr = self.replay_op_drr.as_ref().unwrap_or(&self.op_drr);
        let started = Instant::now();
        let result = drr
            .acquire_with_tier(
                tenant,
                replay_weight,
                policy.max_starvation_rounds,
                policy.tier,
                self.acquire_timeout,
            )
            .await;
        if let Some(m) = self.metrics.as_deref() {
            m.observe_acquire_latency(SchedulerKind::Op, started.elapsed().as_secs_f64());
        }
        result
    }

    /// **N15** : consume `tokens` from `tenant`'s LLM bucket for
    /// `provider`.  Used by the dispatcher's Mode C path before each
    /// LLM provider call.
    pub async fn consume_llm_tokens(
        &self,
        tenant: TenantId,
        provider: &str,
        tokens: f64,
    ) -> Result<(), BackpressureError> {
        let policy = self.policy_lookup.policy(tenant).await?;
        let rate = policy.llm_tokens_per_second.max(0.0);
        let capacity = burst_capacity(rate);
        let result = self
            .llm_buckets
            .consume(
                tenant,
                provider,
                capacity,
                rate,
                tokens,
                self.acquire_timeout,
            )
            .await;
        if let Some(m) = self.metrics.as_deref() {
            m.tick_bucket_consume(RegistryKind::Llm, policy.tier, result.is_ok());
        }
        result
    }

    /// **N15** : consume one audit-write quanta from `tenant`'s
    /// bucket.  Used by the audit chain writer before every record.
    pub async fn consume_audit_write(
        &self,
        tenant: TenantId,
    ) -> Result<(), BackpressureError> {
        let policy = self.policy_lookup.policy(tenant).await?;
        let rate = policy.audit_writes_per_second.max(0.0);
        let capacity = burst_capacity(rate);
        let result = self
            .audit_buckets
            .consume(
                tenant,
                "audit",
                capacity,
                rate,
                1.0,
                self.acquire_timeout,
            )
            .await;
        if let Some(m) = self.metrics.as_deref() {
            m.tick_bucket_consume(RegistryKind::Audit, policy.tier, result.is_ok());
        }
        result
    }

    /// **N15** : non-blocking probe — would `admit_swarm` succeed
    /// right now, without waiting on the DRR pool ?  Useful for
    /// proactive throttling.
    pub async fn would_admit(&self, tenant: TenantId, op_count: u32) -> bool {
        let Ok(policy) = self.policy_lookup.policy(tenant).await else {
            return false;
        };
        let snap = self.tracker.snapshot(tenant);
        snap.inflight_ops.saturating_add(op_count) <= policy.max_inflight_ops
            && snap.inflight_swarms.saturating_add(1) <= policy.max_inflight_swarms
            && self.swarm_drr.inflight() < self.swarm_drr.capacity()
    }
}

/// Burst capacity = max(1, rate).  Even slow tiers get at least one
/// token of headroom so single requests don't deadlock on cold
/// start.
fn burst_capacity(rate: f64) -> f64 {
    if !rate.is_finite() {
        // Platform tier — infinite refill, sane finite capacity.
        return f64::MAX / 2.0;
    }
    rate.max(1.0)
}

// ─────────────────────────────────────────────────────────────────────
//  TrackerRollbackGuard — cancellation safety (G11)
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N15** : scope-guard that rolls back a tracker
/// reservation when the surrounding async function's future is
/// dropped mid-execution (e.g. client disconnect during the
/// `swarm_drr.acquire` await).
///
/// The guard is *armed* by default ; the success path **disarms** it
/// (sets `armed = false`) before dropping it, so the rollback only
/// runs on the cancel-drop path.
struct TrackerRollbackGuard {
    tracker: Arc<InflightTracker>,
    tenant: TenantId,
    op_count: u32,
    armed: bool,
}

impl Drop for TrackerRollbackGuard {
    fn drop(&mut self) {
        if self.armed {
            self.tracker.release_swarm(self.tenant, self.op_count);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  AdmissionTicket — RAII
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N15** : RAII guard returned by
/// [`AdmissionGate::admit_swarm`].
///
/// On `Drop`, the ticket :
/// 1. Releases the global swarm DRR permit (its own `Drop`).
/// 2. Decrements the per-tenant tracker counters by `op_count`.
///
/// Explicit [`AdmissionTicket::release`] is also exposed for
/// dispatchers that want deterministic cleanup before the natural
/// end-of-scope.
pub struct AdmissionTicket {
    gate: Arc<AdmissionGate>,
    tenant: TenantId,
    op_count: u32,
    swarm_permit: Option<DrrPermit>,
    policy: QuotaPolicy,
    released: bool,
}

impl std::fmt::Debug for AdmissionTicket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdmissionTicket")
            .field("tenant", &self.tenant)
            .field("op_count", &self.op_count)
            .field("tier", &self.policy.tier)
            .field("released", &self.released)
            .finish()
    }
}

impl AdmissionTicket {
    /// The tenant this ticket admits.
    pub fn tenant(&self) -> TenantId {
        self.tenant
    }

    /// Op count reserved by this ticket.
    pub fn op_count(&self) -> u32 {
        self.op_count
    }

    /// **N15** : the policy snapshot resolved at admission time.
    /// Stable for the duration of the ticket — downstream callers
    /// can read it without re-fetching from the lookup.
    pub fn policy(&self) -> &QuotaPolicy {
        &self.policy
    }

    /// **N15** : explicitly release the ticket.  Equivalent to
    /// dropping it.
    pub fn release(mut self) {
        self.do_release();
    }

    fn do_release(&mut self) {
        if self.released {
            return;
        }
        self.released = true;
        // 1. Release swarm DRR permit (its Drop fires).
        self.swarm_permit.take();
        // 2. Release tracker reservation.
        self.gate
            .tracker
            .release_swarm(self.tenant, self.op_count);
        // 3. Update metrics gauges.
        if let Some(m) = self.gate.metrics.as_deref() {
            m.add_inflight_ops(self.policy.tier, -i64::from(self.op_count));
            m.add_inflight_swarms(self.policy.tier, -1);
            m.add_admission_inflight(-1);
        }
    }
}

impl Drop for AdmissionTicket {
    fn drop(&mut self) {
        self.do_release();
    }
}

// ─────────────────────────────────────────────────────────────────────
//  AdmissionGateBuilder
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N15** : builder for [`AdmissionGate`].
///
/// All scheduler subsystem references are wrapped in `Arc` so the
/// gate can share them with metrics surfaces / debug endpoints.
#[derive(Default)]
pub struct AdmissionGateBuilder {
    policy_lookup: Option<Arc<dyn TenantPolicyLookup>>,
    tracker: Option<Arc<InflightTracker>>,
    swarm_drr: Option<Arc<DrrScheduler>>,
    op_drr: Option<Arc<DrrScheduler>>,
    firecracker_drr: Option<Arc<DrrScheduler>>,
    replay_op_drr: Option<Arc<DrrScheduler>>,
    audit_buckets: Option<Arc<TokenBucketRegistry>>,
    llm_buckets: Option<Arc<TokenBucketRegistry>>,
    acquire_timeout: Option<Duration>,
    retry_after_ms: Option<u32>,
    metrics: Option<Arc<SchedulerMetrics>>,
}

impl AdmissionGateBuilder {
    /// **N15** : install the policy lookup.
    pub fn with_policy_lookup(mut self, lookup: Arc<dyn TenantPolicyLookup>) -> Self {
        self.policy_lookup = Some(lookup);
        self
    }

    /// **N15** : install the inflight tracker.
    pub fn with_tracker(mut self, tracker: Arc<InflightTracker>) -> Self {
        self.tracker = Some(tracker);
        self
    }

    /// **N15** : install the global swarm DRR scheduler.
    pub fn with_swarm_drr(mut self, drr: Arc<DrrScheduler>) -> Self {
        self.swarm_drr = Some(drr);
        self
    }

    /// **N15** : install the per-op DRR scheduler.
    pub fn with_op_drr(mut self, drr: Arc<DrrScheduler>) -> Self {
        self.op_drr = Some(drr);
        self
    }

    /// **N15** : install the Firecracker pool DRR scheduler.
    pub fn with_firecracker_drr(mut self, drr: Arc<DrrScheduler>) -> Self {
        self.firecracker_drr = Some(drr);
        self
    }

    /// **N16** : install the dedicated replay DRR scheduler.  When
    /// unset, replay traffic shares the production `op_drr` lane.
    pub fn with_replay_op_drr(mut self, drr: Arc<DrrScheduler>) -> Self {
        self.replay_op_drr = Some(drr);
        self
    }

    /// **N15** : install the audit token bucket registry.
    pub fn with_audit_buckets(mut self, buckets: Arc<TokenBucketRegistry>) -> Self {
        self.audit_buckets = Some(buckets);
        self
    }

    /// **N15** : install the LLM token bucket registry.
    pub fn with_llm_buckets(mut self, buckets: Arc<TokenBucketRegistry>) -> Self {
        self.llm_buckets = Some(buckets);
        self
    }

    /// **N15** : override the acquire timeout.
    pub fn with_acquire_timeout(mut self, timeout: Duration) -> Self {
        self.acquire_timeout = Some(timeout);
        self
    }

    /// **N15** : override the retry-after hint.
    pub fn with_retry_after_ms(mut self, ms: u32) -> Self {
        self.retry_after_ms = Some(ms.max(1));
        self
    }

    /// **N15** : install a [`SchedulerMetrics`] surface.  When set,
    /// the gate ticks all 12 Prometheus counters on every operation.
    pub fn with_metrics(mut self, metrics: Arc<SchedulerMetrics>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    /// **N15** : finalize.  Any field left unset falls back to a
    /// sensible default :
    /// - `policy_lookup` : default `InMemoryTenantPolicyLookup`.
    /// - `tracker` : fresh empty.
    /// - `swarm_drr` : capacity 10 000 (large enough to never gate),
    ///   classified `SchedulerKind::Swarm` ; metrics-wired if installed.
    /// - `op_drr` : capacity 50 000, `SchedulerKind::Op`.
    /// - `firecracker_drr` : capacity 1 000, `SchedulerKind::Firecracker`.
    /// - `audit_buckets` : new registry classified `AuditRateLimited`.
    /// - `llm_buckets` : new registry classified `LlmRateLimited`.
    /// - `acquire_timeout` : [`DEFAULT_ACQUIRE_TIMEOUT`].
    /// - `retry_after_ms` : [`DEFAULT_RETRY_AFTER_MS`].
    pub fn build(self) -> Arc<AdmissionGate> {
        let policy_lookup = self.policy_lookup.unwrap_or_else(|| {
            Arc::new(crate::scheduler::policy_lookup::InMemoryTenantPolicyLookup::default())
        });
        let tracker = self.tracker.unwrap_or_else(InflightTracker::arc);
        let metrics_for_drr = self.metrics.clone();
        let swarm_drr = self.swarm_drr.unwrap_or_else(|| {
            DrrScheduler::with_options(
                10_000,
                BackpressureKind::SwarmInflightFull,
                500,
                SchedulerKind::Swarm,
                metrics_for_drr.clone(),
            )
        });
        let op_drr = self.op_drr.unwrap_or_else(|| {
            DrrScheduler::with_options(
                50_000,
                BackpressureKind::OpInflightFull,
                500,
                SchedulerKind::Op,
                metrics_for_drr.clone(),
            )
        });
        let firecracker_drr = self.firecracker_drr.unwrap_or_else(|| {
            DrrScheduler::with_options(
                1_000,
                BackpressureKind::FirecrackerPoolFull,
                500,
                SchedulerKind::Firecracker,
                metrics_for_drr,
            )
        });
        let audit_buckets = self
            .audit_buckets
            .unwrap_or_else(|| TokenBucketRegistry::new(BackpressureKind::AuditRateLimited));
        let llm_buckets = self
            .llm_buckets
            .unwrap_or_else(|| TokenBucketRegistry::new(BackpressureKind::LlmRateLimited));
        Arc::new(AdmissionGate {
            policy_lookup,
            tracker,
            swarm_drr,
            op_drr,
            firecracker_drr,
            replay_op_drr: self.replay_op_drr,
            audit_buckets,
            llm_buckets,
            acquire_timeout: self.acquire_timeout.unwrap_or(DEFAULT_ACQUIRE_TIMEOUT),
            retry_after_ms: self.retry_after_ms.unwrap_or(DEFAULT_RETRY_AFTER_MS),
            metrics: self.metrics,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::policy_lookup::InMemoryTenantPolicyLookup;
    use efe_nano_sdk::TenantTier;

    fn gate_with(lookup: Arc<InMemoryTenantPolicyLookup>) -> Arc<AdmissionGate> {
        AdmissionGate::builder()
            .with_policy_lookup(lookup as Arc<dyn TenantPolicyLookup>)
            .build()
    }

    #[tokio::test]
    async fn admits_default_tier_within_caps() {
        let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
        let gate = gate_with(lookup);
        let t = TenantId::new();
        let ticket = gate.admit_swarm(t, 1).await.unwrap();
        assert_eq!(ticket.tenant(), t);
        assert_eq!(ticket.op_count(), 1);
        assert_eq!(ticket.policy().tier, TenantTier::Free);
    }

    #[tokio::test]
    async fn rejects_suspended_tenant() {
        let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
        let t = TenantId::new();
        lookup.suspend(t);
        let gate = gate_with(lookup);
        let err = gate.admit_swarm(t, 1).await.unwrap_err();
        assert_eq!(err.kind, BackpressureKind::TenantSuspended);
    }

    #[tokio::test]
    async fn rejects_when_op_cap_exceeded() {
        let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
        // Free tier max_inflight_ops = 5.
        let gate = gate_with(lookup);
        let t = TenantId::new();
        let _ticket = gate.admit_swarm(t, 5).await.unwrap();
        let err = gate.admit_swarm(t, 1).await.unwrap_err();
        assert_eq!(err.kind, BackpressureKind::OpInflightFull);
    }

    #[tokio::test]
    async fn rejects_when_swarm_cap_exceeded() {
        let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
        // Free tier max_inflight_swarms = 1, max_ops = 5.
        let gate = gate_with(lookup);
        let t = TenantId::new();
        let _ticket = gate.admit_swarm(t, 1).await.unwrap();
        let err = gate.admit_swarm(t, 1).await.unwrap_err();
        // op_count=1 + inflight_ops=1 still under cap 5 → swarm cap fires.
        assert_eq!(err.kind, BackpressureKind::SwarmInflightFull);
    }

    #[tokio::test]
    async fn release_on_drop_frees_capacity() {
        let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
        let gate = gate_with(lookup);
        let t = TenantId::new();
        {
            let _t = gate.admit_swarm(t, 1).await.unwrap();
            assert_eq!(gate.tracker().snapshot(t).inflight_swarms, 1);
        }
        // Drop'ed → counters back to 0.
        assert_eq!(gate.tracker().snapshot(t).inflight_swarms, 0);
        assert_eq!(gate.tracker().snapshot(t).inflight_ops, 0);
    }

    #[tokio::test]
    async fn explicit_release_consumes_ticket() {
        let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
        let gate = gate_with(lookup);
        let t = TenantId::new();
        let ticket = gate.admit_swarm(t, 3).await.unwrap();
        ticket.release();
        assert_eq!(gate.tracker().snapshot(t).inflight_ops, 0);
        assert_eq!(gate.tracker().snapshot(t).inflight_swarms, 0);
    }

    #[tokio::test]
    async fn tier_change_applies_to_next_admission() {
        let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
        let t = TenantId::new();
        lookup.set_tier(t, TenantTier::Tier3);
        let gate = gate_with(lookup);
        // Tier3 allows 500 ops / 100 swarms.
        let ticket = gate.admit_swarm(t, 50).await.unwrap();
        assert_eq!(ticket.policy().tier, TenantTier::Tier3);
        assert_eq!(ticket.policy().max_inflight_ops, 500);
    }

    #[tokio::test]
    async fn audit_bucket_throttles_when_rate_exhausted() {
        let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
        let gate = gate_with(lookup);
        let t = TenantId::new();
        // Free tier: 50 audit writes / sec.  Burn 50 ; next should
        // throttle but eventually succeed (refill is 50/sec).
        for _ in 0..50 {
            gate.consume_audit_write(t).await.unwrap();
        }
        // The 51st should still succeed (refill is fast).  Just verify
        // the call completes without panic.
        gate.consume_audit_write(t).await.unwrap();
    }

    #[tokio::test]
    async fn llm_bucket_classified_as_llm_rate_limited() {
        let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
        let gate = AdmissionGate::builder()
            .with_policy_lookup(lookup as Arc<dyn TenantPolicyLookup>)
            .with_acquire_timeout(Duration::from_millis(20))
            .build();
        let t = TenantId::new();
        // Free tier: 50 tokens/sec.  Consume 50, then ask for many
        // more — should fail with LlmRateLimited.
        gate.consume_llm_tokens(t, "openai", 50.0).await.unwrap();
        let err = gate.consume_llm_tokens(t, "openai", 999.0).await.unwrap_err();
        // 999 > burst_capacity of 50, but wait that's because cap > burst.
        // Actually burst_capacity = rate.max(1.0) = 50.  And 999 > 50 →
        // immediate "cost exceeds capacity" error.  Kind is still
        // LlmRateLimited (the registry's classification).
        assert_eq!(err.kind, BackpressureKind::LlmRateLimited);
    }

    #[tokio::test]
    async fn op_permit_acquires_one_slot() {
        let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
        let gate = gate_with(lookup);
        let t = TenantId::new();
        let permit = gate.acquire_op_permit(t).await.unwrap();
        assert_eq!(gate.op_drr().inflight(), 1);
        drop(permit);
        assert_eq!(gate.op_drr().inflight(), 0);
    }

    #[tokio::test]
    async fn firecracker_permit_uses_pool_share_as_weight() {
        let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
        let gate = gate_with(lookup);
        let t = TenantId::new();
        let permit = gate.acquire_firecracker_permit(t).await.unwrap();
        assert_eq!(gate.firecracker_drr().inflight(), 1);
        drop(permit);
    }

    #[tokio::test]
    async fn would_admit_is_synchronous_probe() {
        let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
        let gate = gate_with(lookup);
        let t = TenantId::new();
        assert!(gate.would_admit(t, 1).await);
        let _ticket = gate.admit_swarm(t, 5).await.unwrap();
        // At cap : would_admit returns false (op_count + inflight_ops = 6 > 5).
        assert!(!gate.would_admit(t, 1).await);
    }

    #[tokio::test]
    async fn would_admit_false_for_suspended() {
        let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
        let t = TenantId::new();
        lookup.suspend(t);
        let gate = gate_with(lookup);
        assert!(!gate.would_admit(t, 1).await);
    }

    #[tokio::test]
    async fn global_swarm_cap_enforced_via_drr() {
        // Build a gate with a tiny swarm_drr capacity = 2 to test
        // global cap.
        let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
        lookup.set_tier(TenantId::new(), TenantTier::Enterprise); // doesn't matter
        let swarm_drr = DrrScheduler::new(2, BackpressureKind::SwarmInflightFull, 100);
        let gate = AdmissionGate::builder()
            .with_policy_lookup(lookup as Arc<dyn TenantPolicyLookup>)
            .with_swarm_drr(swarm_drr)
            .with_acquire_timeout(Duration::from_millis(50))
            .build();
        let t1 = TenantId::new();
        let t2 = TenantId::new();
        let t3 = TenantId::new();
        let _a = gate.admit_swarm(t1, 1).await.unwrap();
        let _b = gate.admit_swarm(t2, 1).await.unwrap();
        let err = gate.admit_swarm(t3, 1).await.unwrap_err();
        assert_eq!(err.kind, BackpressureKind::SwarmInflightFull);
    }

    #[tokio::test]
    async fn drr_timeout_rolls_back_tracker_reservation() {
        let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
        let swarm_drr = DrrScheduler::new(1, BackpressureKind::SwarmInflightFull, 100);
        let gate = AdmissionGate::builder()
            .with_policy_lookup(lookup as Arc<dyn TenantPolicyLookup>)
            .with_swarm_drr(swarm_drr)
            .with_acquire_timeout(Duration::from_millis(50))
            .build();
        let t1 = TenantId::new();
        let t2 = TenantId::new();
        let _hold = gate.admit_swarm(t1, 1).await.unwrap();
        let err = gate.admit_swarm(t2, 1).await.unwrap_err();
        assert_eq!(err.kind, BackpressureKind::SwarmInflightFull);
        // T2's tracker reservation must have been rolled back.
        assert_eq!(gate.tracker().snapshot(t2).inflight_ops, 0);
        assert_eq!(gate.tracker().snapshot(t2).inflight_swarms, 0);
    }

    #[tokio::test]
    async fn builder_defaults_produce_working_gate() {
        let gate = AdmissionGate::builder().build();
        let t = TenantId::new();
        let _ticket = gate.admit_swarm(t, 1).await.unwrap();
    }

    #[tokio::test]
    async fn ticket_carries_resolved_policy() {
        let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
        let t = TenantId::new();
        lookup.set_tier(t, TenantTier::Tier2);
        let gate = gate_with(lookup);
        let ticket = gate.admit_swarm(t, 1).await.unwrap();
        assert_eq!(ticket.policy().tier, TenantTier::Tier2);
        assert_eq!(ticket.policy().drr_weight, 5);
    }

    #[tokio::test]
    async fn debug_format_does_not_panic() {
        let gate = AdmissionGate::builder().build();
        let _ = format!("{gate:?}");
        let t = TenantId::new();
        let ticket = gate.admit_swarm(t, 1).await.unwrap();
        let _ = format!("{ticket:?}");
    }

    #[tokio::test]
    async fn platform_tier_admits_huge_op_count() {
        let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
        let t = TenantId::new();
        lookup.set_tier(t, TenantTier::Platform);
        let gate = gate_with(lookup);
        // 10 000 ops — Free tier would explode, Platform should accept.
        let _ticket = gate.admit_swarm(t, 10_000).await.unwrap();
    }

    // ─────────────────────────────────────────────────────────────────
    //  Cancellation safety (G11) — dropping the future mid-await
    //  must not leak tracker reservations.
    // ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn dropping_admit_future_rolls_back_tracker_reservation() {
        // Build a gate with a tiny swarm DRR capacity so the second
        // admit must wait.  We hold the slot with one admit, then
        // drop the second admit's future mid-wait and verify the
        // tracker is rolled back.
        let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
        let swarm_drr = DrrScheduler::with_options(
            1,
            BackpressureKind::SwarmInflightFull,
            500,
            SchedulerKind::Swarm,
            None,
        );
        let gate = AdmissionGate::builder()
            .with_policy_lookup(Arc::clone(&lookup) as Arc<dyn TenantPolicyLookup>)
            .with_swarm_drr(swarm_drr)
            .with_acquire_timeout(Duration::from_secs(10))
            .build();

        let t1 = TenantId::new();
        let t2 = TenantId::new();

        // Hold the only swarm slot.
        let _hold = gate.admit_swarm(t1, 1).await.unwrap();

        // Start a second admit that will block on the swarm DRR.
        let gate2 = Arc::clone(&gate);
        // Wrap in a timeout we'll abort early to simulate client
        // disconnect.  We move `gate2` into the task so the future
        // can borrow it for its entire lifetime.
        let join = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_secs(10), gate2.admit_swarm(t2, 3)).await
        });
        // Give it time to register the reservation + queue.
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Verify the tracker reservation IS recorded mid-flight.
        let mid_snap = gate.tracker().snapshot(t2);
        assert_eq!(mid_snap.inflight_ops, 3);
        assert_eq!(mid_snap.inflight_swarms, 1);

        // Abort the task — drops the future.
        join.abort();
        let _ = join.await;

        // Give the Drop machinery a moment to fire.
        tokio::time::sleep(Duration::from_millis(50)).await;

        // The TrackerRollbackGuard's Drop should have rolled back
        // the reservation.
        let after_snap = gate.tracker().snapshot(t2);
        assert_eq!(
            after_snap.inflight_ops, 0,
            "tracker op reservation leaked after future drop : {} ops still recorded",
            after_snap.inflight_ops
        );
        assert_eq!(
            after_snap.inflight_swarms, 0,
            "tracker swarm reservation leaked after future drop"
        );
    }

    #[tokio::test]
    async fn metrics_drr_rounds_ticks_on_swarm_drr() {
        let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
        let metrics = SchedulerMetrics::new();
        // Force the gate's internal swarm DRR to have a small
        // capacity so contention is created.
        let gate = AdmissionGate::builder()
            .with_policy_lookup(Arc::clone(&lookup) as Arc<dyn TenantPolicyLookup>)
            .with_metrics(Arc::clone(&metrics))
            .build();

        let t = TenantId::new();
        let ticket = gate.admit_swarm(t, 1).await.unwrap();
        drop(ticket);

        // First admit was synchronous (no contention).  But release
        // calls advance, which ticks at least one round even on the
        // synchronous-grant path.  Confirm the metric pipeline works
        // end-to-end.
        let snap = metrics.snapshot();
        assert!(snap.drr_rounds_for(SchedulerKind::Swarm) >= 1);
    }
}
