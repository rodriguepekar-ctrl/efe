//! [`Catalog`] trait — registry of all nano agents.

use async_trait::async_trait;
use std::sync::Arc;

use crate::types::{
    Category, ComparisonStrategy, CostAttribution, NanoAgent, NanoAgentId, NanoAgentRef,
    NanoAgentVersion, RolloutPolicy, StabilityTier, TenantId, VersionReq,
};

/// Result type for catalog operations.
///
/// We use a richer error in the runtime crate, but at the SDK level we
/// expose a generic `Box<dyn Error>` to keep deps minimal.
pub type CatalogResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Filter for the [`Catalog::list`] operation.
#[derive(Debug, Clone, Default)]
pub struct ListFilter {
    /// Filter by category.
    pub category: Option<Category>,

    /// Minimum stability tier (inclusive).
    pub stability_tier_min: Option<StabilityTier>,

    /// Include EFE-shipped agents.
    pub include_efe_shipped: bool,

    /// Include tenant-owned agents (private to the tenant).
    pub include_tenant_owned: bool,

    /// Include community agents.
    pub include_community: bool,

    /// Pagination limit (default 100).
    pub limit: u32,

    /// Pagination offset.
    pub offset: u32,
}

impl ListFilter {
    /// Default : include everything, paginate 100 at a time.
    pub fn include_all() -> Self {
        Self {
            category: None,
            stability_tier_min: None,
            include_efe_shipped: true,
            include_tenant_owned: true,
            include_community: true,
            limit: 100,
            offset: 0,
        }
    }

    /// Filter for EFE-shipped agents only.
    pub fn efe_only() -> Self {
        Self {
            category: None,
            stability_tier_min: None,
            include_efe_shipped: true,
            include_tenant_owned: false,
            include_community: false,
            limit: 100,
            offset: 0,
        }
    }

    /// Filter for a specific category.
    pub fn by_category(category: Category) -> Self {
        Self {
            category: Some(category),
            ..Self::include_all()
        }
    }

    /// Builder : set pagination.
    pub fn with_pagination(mut self, limit: u32, offset: u32) -> Self {
        self.limit = limit;
        self.offset = offset;
        self
    }
}

/// Registry of all nano agents.
///
/// The catalog is the source of truth for what nano agents exist,
/// their versions, and their metadata.  Implementations :
///
/// - `InMemoryCatalog` — for testing and dev (in `efe-nano` crate).
/// - `PostgresCatalog` — production backend (in `efe-nano` crate, Sprint 2).
#[async_trait]
pub trait Catalog: Send + Sync + 'static {
    /// Get a specific nano agent by reference.
    ///
    /// Resolves the version constraint (if any) and returns the
    /// most recent matching version.
    ///
    /// Returns an error if no version matches.
    async fn get(&self, r: &NanoAgentRef, tenant: TenantId) -> CatalogResult<Arc<NanoAgent>>;

    /// List nano agents visible to the given tenant, with filters.
    async fn list(
        &self,
        tenant: TenantId,
        filter: ListFilter,
    ) -> CatalogResult<Vec<Arc<NanoAgent>>>;

    /// Search by free-text query (description matching).
    async fn search(
        &self,
        tenant: TenantId,
        query: &str,
    ) -> CatalogResult<Vec<Arc<NanoAgent>>>;

    /// Publish a new nano agent version (customer-owned tenant only).
    ///
    /// The agent's author must be `Author::Tenant { tenant_id: tenant }`
    /// or the operation is rejected.
    async fn publish(
        &self,
        tenant: TenantId,
        agent: NanoAgent,
    ) -> CatalogResult<NanoAgentId>;

    /// Soft-delete a version (customer's own agents).
    async fn unpublish(
        &self,
        tenant: TenantId,
        id: &NanoAgentId,
        version: &NanoAgentVersion,
    ) -> CatalogResult<()>;

    /// Promote stability tier (admin-only operation).
    async fn promote(
        &self,
        id: &NanoAgentId,
        version: &NanoAgentVersion,
        to: StabilityTier,
    ) -> CatalogResult<()>;

    // ─────────────────────────────────────────────────────────────────
    //  Sprint 8 — N13 : shadow / canary rollout API
    // ─────────────────────────────────────────────────────────────────

    /// **Sprint 8 — N13** : resolve an agent for dispatch, applying any
    /// active rollout policy.
    ///
    /// Returns [`ResolvedAgent::Single`] for stable / canary-resolved /
    /// version-pin opt-out paths ; returns [`ResolvedAgent::ShadowPair`]
    /// when the policy is `Shadow`.
    ///
    /// The default impl ignores rollout policies and delegates to
    /// [`Self::get`] — preserves pre-N13 behaviour for catalog impls
    /// that don't yet support rollout.
    async fn resolve_for_dispatch(
        &self,
        tenant: TenantId,
        agent_id: &NanoAgentId,
        version_req: &VersionReq,
        _input_hash: &[u8; 32],
    ) -> CatalogResult<ResolvedAgent> {
        let r = NanoAgentRef {
            id: agent_id.clone(),
            version: version_req.clone(),
        };
        let agent = self.get(&r, tenant).await?;
        Ok(ResolvedAgent::Single(agent))
    }

    /// **Sprint 8 — N13** : set or update the rollout policy for an
    /// agent.  Atomically replaces any prior policy.  Returns the
    /// previous policy (or the default `Stable(latest)` if none existed).
    ///
    /// The default impl returns an error : catalogs without rollout
    /// support shouldn't accept policy writes.
    async fn set_rollout_policy(
        &self,
        _tenant: TenantId,
        _agent_id: &NanoAgentId,
        _policy: RolloutPolicy,
    ) -> CatalogResult<RolloutPolicy> {
        Err("set_rollout_policy not supported by this catalog impl".into())
    }

    /// **Sprint 8 — N13** : get the current rollout policy.
    ///
    /// The default impl returns `Stable { version: <latest visible> }`
    /// so callers that don't yet support rollout work transparently.
    async fn get_rollout_policy(
        &self,
        tenant: TenantId,
        agent_id: &NanoAgentId,
    ) -> CatalogResult<RolloutPolicy> {
        let r = NanoAgentRef::new(agent_id.clone());
        let agent = self.get(&r, tenant).await?;
        Ok(RolloutPolicy::Stable {
            version: agent.version.clone(),
        })
    }
}

// ─────────────────────────────────────────────────────────────────────
//  ResolvedAgent
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N13** : What [`Catalog::resolve_for_dispatch`] returns.
#[derive(Debug, Clone)]
pub enum ResolvedAgent {
    /// Single-agent dispatch — Stable / Canary-resolved / version-pin
    /// opt-out.
    Single(Arc<NanoAgent>),

    /// Production + shadow pair.  Dispatcher runs `prod` synchronously,
    /// schedules `shadow` async via `ShadowScheduler`.
    ShadowPair {
        prod: Arc<NanoAgent>,
        shadow: Arc<NanoAgent>,
        comparison: ComparisonStrategy,
        cost_attribution: CostAttribution,
        shadow_budget_cents: Option<f64>,
        shadow_on_cache_hit: bool,
    },
}

impl ResolvedAgent {
    /// The "main" agent — the one that produces the caller-visible
    /// output.  For `Single`, that's the only agent ; for `ShadowPair`,
    /// it's `prod`.
    pub fn prod_agent(&self) -> &Arc<NanoAgent> {
        match self {
            Self::Single(a) => a,
            Self::ShadowPair { prod, .. } => prod,
        }
    }

    /// Whether this dispatch carries a shadow side-effect.
    pub fn has_shadow(&self) -> bool {
        matches!(self, Self::ShadowPair { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_filter_include_all_defaults() {
        let f = ListFilter::include_all();
        assert!(f.include_efe_shipped);
        assert!(f.include_tenant_owned);
        assert!(f.include_community);
        assert_eq!(f.limit, 100);
        assert_eq!(f.offset, 0);
    }

    #[test]
    fn list_filter_efe_only() {
        let f = ListFilter::efe_only();
        assert!(f.include_efe_shipped);
        assert!(!f.include_tenant_owned);
        assert!(!f.include_community);
    }

    #[test]
    fn list_filter_by_category() {
        let f = ListFilter::by_category(Category::Validation);
        assert_eq!(f.category, Some(Category::Validation));
        assert!(f.include_efe_shipped);  // include_all default
    }

    #[test]
    fn list_filter_with_pagination() {
        let f = ListFilter::include_all().with_pagination(50, 100);
        assert_eq!(f.limit, 50);
        assert_eq!(f.offset, 100);
    }
}
