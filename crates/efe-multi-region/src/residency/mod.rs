// SPDX-License-Identifier: UNLICENSED
//! **Sprint 9 — N16** : Per-tenant data residency policy lookup.
//!
//! Architecture § 7.6.  The coordinator validates this policy BEFORE
//! any chain bytes leave the source region (G1 + I15).

use async_trait::async_trait;
use dashmap::DashMap;
use thiserror::Error;

use efe_nano_sdk::{RegionCode, ResidencyPolicy, ResidencyViolation, TenantId};

/// **Sprint 9 — N16** : closed taxonomy of residency-lookup errors.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ResidencyError {
    /// Policy backend I/O fault.
    #[error("residency backend error : {0}")]
    Backend(String),
}

/// **Sprint 9 — N16** : look up a tenant's residency policy.
#[async_trait]
pub trait ResidencyPolicyLookup: Send + Sync + 'static {
    /// **N16** : Look up the active policy for `tenant`.  Tenants
    /// with no explicit policy fall back to the lookup's configured
    /// default.
    async fn policy(&self, tenant: TenantId) -> Result<ResidencyPolicy, ResidencyError>;

    /// **N16** : Validate that `target_region` is permitted for
    /// `tenant`.  Returns `Err(ResidencyViolation)` on denial.
    async fn validate(
        &self,
        tenant: TenantId,
        target_region: RegionCode,
    ) -> Result<(), ResidencyError> {
        let policy = self.policy(tenant).await?;
        policy
            .validate(target_region)
            .map_err(|_| ResidencyError::Backend("residency violation".into()))
    }
}

/// **Sprint 9 — N16** : in-memory implementation backed by a
/// `DashMap`.  Used for tests + the Sprint 9 demo binary.  Sprint 10
/// will ship a Postgres-backed version that reads from the tenant
/// catalog (`m50.tenants` table).
pub struct InMemoryResidencyPolicyLookup {
    default_policy: ResidencyPolicy,
    policies: DashMap<TenantId, ResidencyPolicy>,
}

impl std::fmt::Debug for InMemoryResidencyPolicyLookup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryResidencyPolicyLookup")
            .field("default_policy", &self.default_policy)
            .field("registered_count", &self.policies.len())
            .finish()
    }
}

impl Default for InMemoryResidencyPolicyLookup {
    fn default() -> Self {
        Self::with_default(ResidencyPolicy::Global)
    }
}

impl InMemoryResidencyPolicyLookup {
    /// **N16** : Construct with the given default policy.
    pub fn with_default(default_policy: ResidencyPolicy) -> Self {
        Self {
            default_policy,
            policies: DashMap::new(),
        }
    }

    /// **N16** : Register an explicit policy for `tenant`.  Returns
    /// the previous policy if any.
    pub fn set_policy(
        &self,
        tenant: TenantId,
        policy: ResidencyPolicy,
    ) -> Option<ResidencyPolicy> {
        self.policies.insert(tenant, policy)
    }

    /// **N16** : Clear a tenant's policy override (falls back to
    /// default).
    pub fn clear_policy(&self, tenant: TenantId) -> Option<ResidencyPolicy> {
        self.policies.remove(&tenant).map(|(_, p)| p)
    }

    /// **N16** : Number of tenants with explicit overrides.
    pub fn registered_count(&self) -> usize {
        self.policies.len()
    }

    /// **N16** : Convenience — validate without going through the
    /// async trait method.  Returns the typed `ResidencyViolation`
    /// rather than `ResidencyError`.
    pub fn validate_sync(
        &self,
        tenant: TenantId,
        target_region: RegionCode,
    ) -> Result<(), ResidencyViolation> {
        let policy = self
            .policies
            .get(&tenant)
            .map(|p| p.value().clone())
            .unwrap_or_else(|| self.default_policy.clone());
        policy.validate(target_region)
    }
}

#[async_trait]
impl ResidencyPolicyLookup for InMemoryResidencyPolicyLookup {
    async fn policy(&self, tenant: TenantId) -> Result<ResidencyPolicy, ResidencyError> {
        Ok(self
            .policies
            .get(&tenant)
            .map(|p| p.value().clone())
            .unwrap_or_else(|| self.default_policy.clone()))
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unknown_tenant_returns_default_policy() {
        let lookup = InMemoryResidencyPolicyLookup::default();
        let p = lookup.policy(TenantId::new()).await.unwrap();
        assert!(matches!(p, ResidencyPolicy::Global));
    }

    #[tokio::test]
    async fn explicit_policy_overrides_default() {
        let lookup = InMemoryResidencyPolicyLookup::default();
        let t = TenantId::new();
        lookup.set_policy(t, ResidencyPolicy::EuOnly);
        let p = lookup.policy(t).await.unwrap();
        assert!(matches!(p, ResidencyPolicy::EuOnly));
    }

    #[tokio::test]
    async fn validate_eu_only_blocks_us_target() {
        let lookup = InMemoryResidencyPolicyLookup::default();
        let t = TenantId::new();
        lookup.set_policy(t, ResidencyPolicy::EuOnly);
        let err = lookup.validate(t, RegionCode::UsEast1).await.unwrap_err();
        // Surfaces as Backend wrap of ResidencyViolation per trait
        // contract.  Caller (coordinator) does the typed violation
        // detection via validate_sync.
        assert!(matches!(err, ResidencyError::Backend(_)));
    }

    #[tokio::test]
    async fn validate_eu_only_accepts_eu_target() {
        let lookup = InMemoryResidencyPolicyLookup::default();
        let t = TenantId::new();
        lookup.set_policy(t, ResidencyPolicy::EuOnly);
        let ok = lookup.validate(t, RegionCode::EuWest1).await;
        assert!(ok.is_ok());
    }

    #[test]
    fn validate_sync_returns_typed_violation() {
        let lookup = InMemoryResidencyPolicyLookup::default();
        let t = TenantId::new();
        lookup.set_policy(t, ResidencyPolicy::EuOnly);
        let err = lookup.validate_sync(t, RegionCode::UsWest2).unwrap_err();
        assert_eq!(err.target_region, RegionCode::UsWest2);
    }

    #[test]
    fn clear_policy_falls_back_to_default() {
        let lookup = InMemoryResidencyPolicyLookup::with_default(ResidencyPolicy::UsOnly);
        let t = TenantId::new();
        lookup.set_policy(t, ResidencyPolicy::EuOnly);
        assert!(lookup.validate_sync(t, RegionCode::EuWest1).is_ok());
        let prev = lookup.clear_policy(t);
        assert!(matches!(prev, Some(ResidencyPolicy::EuOnly)));
        // Now falls back to UsOnly → EuWest1 forbidden.
        assert!(lookup.validate_sync(t, RegionCode::EuWest1).is_err());
        assert!(lookup.validate_sync(t, RegionCode::UsEast1).is_ok());
    }

    #[test]
    fn registered_count_reflects_state() {
        let lookup = InMemoryResidencyPolicyLookup::default();
        assert_eq!(lookup.registered_count(), 0);
        lookup.set_policy(TenantId::new(), ResidencyPolicy::EuOnly);
        lookup.set_policy(TenantId::new(), ResidencyPolicy::UsOnly);
        assert_eq!(lookup.registered_count(), 2);
    }

    #[test]
    fn debug_format_does_not_panic() {
        let lookup = InMemoryResidencyPolicyLookup::default();
        let _ = format!("{lookup:?}");
    }
}
