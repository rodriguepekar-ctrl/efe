// SPDX-License-Identifier: UNLICENSED
//! **Sprint 8 — N15** : Tenant policy lookup.
//!
//! The `TenantPolicyLookup` trait resolves a [`TenantId`] → either a
//! [`QuotaPolicy`] (allowed) or a `TenantSuspended` backpressure error
//! (denied at the front door).  It is the single source of truth for
//! "what quota does this tenant have?" — every other scheduler module
//! (DRR, token bucket, tracker, admission gate) reads from it.
//!
//! # Resolution order (architecture § 6.1)
//!
//! 1. If the tenant is in the suspension set → return
//!    `BackpressureError { kind: TenantSuspended, .. }`.
//! 2. If a per-tenant **policy override** is registered → return it.
//!    (Override is fully formed ; the tier inside it is honored.)
//! 3. If a per-tenant **tier override** is registered → return
//!    `QuotaPolicy::for_tier(tier)`.
//! 4. Otherwise → return `QuotaPolicy::for_tier(default_tier)`.
//!
//! # Concurrency
//!
//! The in-memory implementation uses [`DashMap`] for lock-free reads
//! on the hot path (every admission request touches `policy(tenant)`).
//! Writes (suspend, set_tier, set_override) take a per-shard lock but
//! are admin-side and rare.
//!
//! # Suspension semantics
//!
//! Suspension is **operator-initiated** and orthogonal to tier :
//! - `suspend(tenant)` flags the tenant — every subsequent lookup
//!   returns the `TenantSuspended` error.
//! - `unsuspend(tenant)` removes the flag.
//! - Suspension does NOT clear the in-flight counters — existing
//!   permits drain naturally ; only **new** admissions are denied.
//!
//! Sprint 9 wires this trait to a Postgres-backed implementation that
//! reads from the tenant catalog (`m50.tenants` table) so per-tenant
//! overrides survive process restarts.

use std::sync::Arc;

use async_trait::async_trait;
use dashmap::{DashMap, DashSet};
use efe_nano_sdk::{BackpressureError, BackpressureKind, QuotaPolicy, TenantId, TenantTier};

// ─────────────────────────────────────────────────────────────────────
//  TenantPolicyLookup trait
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N15** : look up the [`QuotaPolicy`] for a tenant.
///
/// Returns `Err(BackpressureError { kind: TenantSuspended, .. })` if
/// the tenant is suspended.  All other lookups always succeed (the
/// trait never returns `Err` for "unknown tenant" — unknowns get the
/// configured default tier per `Free` tier safety policy).
#[async_trait]
pub trait TenantPolicyLookup: Send + Sync + 'static {
    /// Look up the active [`QuotaPolicy`] for `tenant`.
    ///
    /// Returns `Err(BackpressureError)` ONLY when the tenant is
    /// suspended.  Unknown tenants resolve to the default tier ; the
    /// trait NEVER returns `Err` for routine lookups.
    async fn policy(&self, tenant: TenantId) -> Result<QuotaPolicy, BackpressureError>;

    /// Cheap synchronous probe — true if the tenant is suspended.
    ///
    /// Provided so the audit-trail path can record the suspension
    /// reason without re-entering the async path.
    fn is_suspended(&self, tenant: TenantId) -> bool;

    /// Tier of `tenant`.  Convenience for metric labels.  Defaults to
    /// the trait's default tier when no tier is registered.
    fn tier(&self, tenant: TenantId) -> TenantTier;
}

// ─────────────────────────────────────────────────────────────────────
//  InMemoryTenantPolicyLookup
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N15** : in-memory implementation of
/// [`TenantPolicyLookup`].
///
/// Wraps three concurrent maps :
/// - `tiers` — tenant → tier override.
/// - `overrides` — tenant → full policy override (highest precedence).
/// - `suspended` — set of suspended tenants.
///
/// All three are lock-free reads via [`DashMap`] / [`DashSet`].
///
/// # Defaults
///
/// New instances default to [`TenantTier::Free`] — the safest
/// starting position for an empty registry.  Operators register
/// known tenants explicitly via [`Self::set_tier`] /
/// [`Self::set_policy_override`].
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use efe_nano::scheduler::policy_lookup::*;
/// use efe_nano_sdk::{TenantId, TenantTier};
/// # async fn demo() {
/// let lookup = InMemoryTenantPolicyLookup::with_default(TenantTier::Tier1);
/// let tenant = TenantId::new();
///
/// // Default lookup : Tier1 preset.
/// let p = lookup.policy(tenant).await.unwrap();
/// assert_eq!(p.tier, TenantTier::Tier1);
///
/// // Promote to Enterprise.
/// lookup.set_tier(tenant, TenantTier::Enterprise);
/// let p = lookup.policy(tenant).await.unwrap();
/// assert_eq!(p.tier, TenantTier::Enterprise);
///
/// // Suspend.
/// lookup.suspend(tenant);
/// assert!(lookup.policy(tenant).await.is_err());
/// # }
/// ```
#[derive(Debug)]
pub struct InMemoryTenantPolicyLookup {
    /// Tier used when a tenant has no explicit registration.
    default_tier: TenantTier,
    /// Tenant → tier override.
    tiers: DashMap<TenantId, TenantTier>,
    /// Tenant → full policy override (overrides tier resolution).
    overrides: DashMap<TenantId, QuotaPolicy>,
    /// Tenants flagged as suspended by an operator.
    suspended: DashSet<TenantId>,
}

impl Default for InMemoryTenantPolicyLookup {
    fn default() -> Self {
        Self::with_default(TenantTier::Free)
    }
}

impl InMemoryTenantPolicyLookup {
    /// **N15** : construct a new lookup with `default_tier` as the
    /// fallback for unregistered tenants.
    pub fn with_default(default_tier: TenantTier) -> Self {
        Self {
            default_tier,
            tiers: DashMap::new(),
            overrides: DashMap::new(),
            suspended: DashSet::new(),
        }
    }

    /// **N15** : wrap in an `Arc` for trait-object use.
    pub fn into_arc(self) -> Arc<Self> {
        Arc::new(self)
    }

    /// **N15** : register a tier override for `tenant`.  Returns the
    /// previous tier override, if any.
    pub fn set_tier(&self, tenant: TenantId, tier: TenantTier) -> Option<TenantTier> {
        self.tiers.insert(tenant, tier)
    }

    /// **N15** : remove a tier override for `tenant`.  Returns the
    /// previously registered tier, if any.
    pub fn clear_tier(&self, tenant: TenantId) -> Option<TenantTier> {
        self.tiers.remove(&tenant).map(|(_, t)| t)
    }

    /// **N15** : register a full policy override for `tenant`.  This
    /// has the highest precedence — it overrides both tier resolution
    /// and the default tier.  Returns the previous override, if any.
    ///
    /// Sprint 8 ship uses tier-only resolution ; this surface exists
    /// for Sprint 9 per-tenant negotiated SLAs.
    pub fn set_policy_override(
        &self,
        tenant: TenantId,
        policy: QuotaPolicy,
    ) -> Option<QuotaPolicy> {
        self.overrides.insert(tenant, policy)
    }

    /// **N15** : remove a per-tenant policy override.  Returns the
    /// previously registered policy, if any.
    pub fn clear_policy_override(&self, tenant: TenantId) -> Option<QuotaPolicy> {
        self.overrides.remove(&tenant).map(|(_, p)| p)
    }

    /// **N15** : suspend a tenant.  Subsequent `policy(tenant)` calls
    /// return `BackpressureError { kind: TenantSuspended, .. }`.  Has
    /// no effect on already-running ops — only new admissions deny.
    /// Returns true if the suspension flag was added (false if the
    /// tenant was already suspended).
    pub fn suspend(&self, tenant: TenantId) -> bool {
        self.suspended.insert(tenant)
    }

    /// **N15** : lift suspension on a tenant.  Returns true if the
    /// flag was actually present.
    pub fn unsuspend(&self, tenant: TenantId) -> bool {
        self.suspended.remove(&tenant).is_some()
    }

    /// **N15** : default tier (used for unregistered tenants).
    pub fn default_tier(&self) -> TenantTier {
        self.default_tier
    }

    /// **N15** : number of tenants with a tier override.
    pub fn registered_tiers_len(&self) -> usize {
        self.tiers.len()
    }

    /// **N15** : number of tenants with a policy override.
    pub fn registered_overrides_len(&self) -> usize {
        self.overrides.len()
    }

    /// **N15** : number of suspended tenants.
    pub fn suspended_len(&self) -> usize {
        self.suspended.len()
    }
}

#[async_trait]
impl TenantPolicyLookup for InMemoryTenantPolicyLookup {
    async fn policy(&self, tenant: TenantId) -> Result<QuotaPolicy, BackpressureError> {
        // (1) Suspension is checked first.
        if self.suspended.contains(&tenant) {
            return Err(BackpressureError::standard(
                BackpressureKind::TenantSuspended,
                tenant,
                // Default retry-after = 60 s — operator decides when
                // to lift suspension ; the client SHOULD wait at least
                // a minute before reattempting.
                60_000,
            ));
        }
        // (2) Per-tenant policy override has highest precedence.
        if let Some(p) = self.overrides.get(&tenant) {
            return Ok(p.value().clone());
        }
        // (3) Per-tenant tier override.
        if let Some(t) = self.tiers.get(&tenant) {
            return Ok(QuotaPolicy::for_tier(*t.value()));
        }
        // (4) Fallback to default tier.
        Ok(QuotaPolicy::for_tier(self.default_tier))
    }

    fn is_suspended(&self, tenant: TenantId) -> bool {
        self.suspended.contains(&tenant)
    }

    fn tier(&self, tenant: TenantId) -> TenantTier {
        if let Some(p) = self.overrides.get(&tenant) {
            return p.value().tier;
        }
        if let Some(t) = self.tiers.get(&tenant) {
            return *t.value();
        }
        self.default_tier
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> (InMemoryTenantPolicyLookup, TenantId) {
        (InMemoryTenantPolicyLookup::default(), TenantId::new())
    }

    #[tokio::test]
    async fn unknown_tenant_yields_default_tier_policy() {
        let (lookup, tenant) = fresh();
        let p = lookup.policy(tenant).await.unwrap();
        assert_eq!(p.tier, TenantTier::Free);
        assert_eq!(p.drr_weight, 1);
    }

    #[tokio::test]
    async fn with_default_tier_changes_unknown_response() {
        let lookup = InMemoryTenantPolicyLookup::with_default(TenantTier::Tier2);
        let tenant = TenantId::new();
        let p = lookup.policy(tenant).await.unwrap();
        assert_eq!(p.tier, TenantTier::Tier2);
        assert_eq!(p.drr_weight, 5);
    }

    #[tokio::test]
    async fn set_tier_promotes_existing_tenant() {
        let (lookup, tenant) = fresh();
        assert!(lookup.set_tier(tenant, TenantTier::Enterprise).is_none());
        let p = lookup.policy(tenant).await.unwrap();
        assert_eq!(p.tier, TenantTier::Enterprise);
        assert_eq!(p.drr_weight, 100);
    }

    #[tokio::test]
    async fn set_tier_returns_previous_value_on_replace() {
        let (lookup, tenant) = fresh();
        lookup.set_tier(tenant, TenantTier::Tier1);
        let prev = lookup.set_tier(tenant, TenantTier::Tier3);
        assert_eq!(prev, Some(TenantTier::Tier1));
    }

    #[tokio::test]
    async fn clear_tier_falls_back_to_default() {
        let (lookup, tenant) = fresh();
        lookup.set_tier(tenant, TenantTier::Tier2);
        assert_eq!(lookup.tier(tenant), TenantTier::Tier2);
        let prev = lookup.clear_tier(tenant);
        assert_eq!(prev, Some(TenantTier::Tier2));
        assert_eq!(lookup.tier(tenant), TenantTier::Free);
    }

    #[tokio::test]
    async fn policy_override_beats_tier_override() {
        let (lookup, tenant) = fresh();
        lookup.set_tier(tenant, TenantTier::Tier1);
        // Custom Enterprise policy with bumped audit QPS.
        let mut custom = QuotaPolicy::enterprise();
        custom.audit_writes_per_second = 99_999.0;
        lookup.set_policy_override(tenant, custom.clone());
        let p = lookup.policy(tenant).await.unwrap();
        // Override wins → Enterprise tier + bumped audit QPS.
        assert_eq!(p.tier, TenantTier::Enterprise);
        assert!((p.audit_writes_per_second - 99_999.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn clear_override_falls_back_to_tier_resolution() {
        let (lookup, tenant) = fresh();
        lookup.set_tier(tenant, TenantTier::Tier2);
        lookup.set_policy_override(tenant, QuotaPolicy::enterprise());
        assert_eq!(lookup.policy(tenant).await.unwrap().tier, TenantTier::Enterprise);
        let prev = lookup.clear_policy_override(tenant);
        assert!(prev.is_some());
        // Falls back to the tier override (Tier2), not the default.
        assert_eq!(lookup.policy(tenant).await.unwrap().tier, TenantTier::Tier2);
    }

    #[tokio::test]
    async fn suspended_tenant_yields_backpressure_error() {
        let (lookup, tenant) = fresh();
        assert!(lookup.suspend(tenant));
        let err = lookup.policy(tenant).await.unwrap_err();
        assert_eq!(err.kind, BackpressureKind::TenantSuspended);
        assert!(err.retry_after_ms > 0);
        assert!(err.advice.contains("suspended") || !err.advice.is_empty());
    }

    #[tokio::test]
    async fn suspend_twice_is_idempotent_returning_false_second_time() {
        let (lookup, tenant) = fresh();
        assert!(lookup.suspend(tenant));
        // Second suspend returns false — already in the set.
        assert!(!lookup.suspend(tenant));
        assert_eq!(lookup.suspended_len(), 1);
    }

    #[tokio::test]
    async fn unsuspend_lifts_the_block() {
        let (lookup, tenant) = fresh();
        lookup.suspend(tenant);
        assert!(lookup.policy(tenant).await.is_err());
        assert!(lookup.unsuspend(tenant));
        let p = lookup.policy(tenant).await.unwrap();
        assert_eq!(p.tier, TenantTier::Free);
    }

    #[tokio::test]
    async fn unsuspend_unknown_tenant_returns_false() {
        let lookup = InMemoryTenantPolicyLookup::default();
        assert!(!lookup.unsuspend(TenantId::new()));
    }

    #[tokio::test]
    async fn suspension_outranks_overrides() {
        let (lookup, tenant) = fresh();
        lookup.set_policy_override(tenant, QuotaPolicy::enterprise());
        lookup.suspend(tenant);
        let err = lookup.policy(tenant).await.unwrap_err();
        assert_eq!(err.kind, BackpressureKind::TenantSuspended);
    }

    #[tokio::test]
    async fn is_suspended_synchronous_probe_matches_async_lookup() {
        let (lookup, tenant) = fresh();
        assert!(!lookup.is_suspended(tenant));
        lookup.suspend(tenant);
        assert!(lookup.is_suspended(tenant));
        lookup.unsuspend(tenant);
        assert!(!lookup.is_suspended(tenant));
    }

    #[tokio::test]
    async fn tier_probe_reflects_overrides() {
        let (lookup, tenant) = fresh();
        assert_eq!(lookup.tier(tenant), TenantTier::Free);

        lookup.set_tier(tenant, TenantTier::Tier3);
        assert_eq!(lookup.tier(tenant), TenantTier::Tier3);

        let mut custom = QuotaPolicy::enterprise();
        custom.tier = TenantTier::Enterprise;
        lookup.set_policy_override(tenant, custom);
        assert_eq!(lookup.tier(tenant), TenantTier::Enterprise);
    }

    #[tokio::test]
    async fn registered_lengths_reflect_state() {
        let lookup = InMemoryTenantPolicyLookup::default();
        let a = TenantId::new();
        let b = TenantId::new();
        let c = TenantId::new();
        lookup.set_tier(a, TenantTier::Tier1);
        lookup.set_policy_override(b, QuotaPolicy::tier3());
        lookup.suspend(c);
        assert_eq!(lookup.registered_tiers_len(), 1);
        assert_eq!(lookup.registered_overrides_len(), 1);
        assert_eq!(lookup.suspended_len(), 1);
    }

    #[tokio::test]
    async fn concurrent_writers_do_not_panic() {
        // Smoke : 16 tasks racing to register tier overrides for the
        // same set of tenants must not corrupt state.
        let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
        let tenants: Vec<_> = (0..32).map(|_| TenantId::new()).collect();
        let mut handles = vec![];
        for worker in 0..16 {
            let lookup = Arc::clone(&lookup);
            let tenants = tenants.clone();
            handles.push(tokio::spawn(async move {
                for t in &tenants {
                    let tier = match worker % 5 {
                        0 => TenantTier::Free,
                        1 => TenantTier::Tier1,
                        2 => TenantTier::Tier2,
                        3 => TenantTier::Tier3,
                        _ => TenantTier::Enterprise,
                    };
                    lookup.set_tier(*t, tier);
                    let _ = lookup.policy(*t).await;
                }
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        // Every tenant should have *some* tier set after the race.
        assert_eq!(lookup.registered_tiers_len(), tenants.len());
    }

    #[tokio::test]
    async fn backpressure_error_carries_tenant_id() {
        let (lookup, tenant) = fresh();
        lookup.suspend(tenant);
        let err = lookup.policy(tenant).await.unwrap_err();
        assert_eq!(err.tenant_id, tenant);
    }

    #[tokio::test]
    async fn platform_tier_resolves_to_unbounded_policy() {
        let (lookup, tenant) = fresh();
        lookup.set_tier(tenant, TenantTier::Platform);
        let p = lookup.policy(tenant).await.unwrap();
        assert!(p.is_unbounded());
        assert_eq!(p.max_inflight_ops, u32::MAX);
    }
}
