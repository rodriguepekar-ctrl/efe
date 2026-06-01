//! [`InMemoryCatalog`] — DashMap-backed catalog for testing + dev.
//!
//! Sprint 2 swaps in a Postgres-backed catalog ; this stays as the
//! canonical fixture for unit + integration tests.

use async_trait::async_trait;
use dashmap::DashMap;
use efe_nano_sdk::traits::catalog::{Catalog, CatalogResult, ListFilter, ResolvedAgent};
use efe_nano_sdk::{
    Author, ComparisonStrategy, DeterminismLevel, NanoAgent, NanoAgentId, NanoAgentRef,
    NanoAgentVersion, RolloutPolicy, StabilityTier, TenantId, VersionReq,
};
use std::sync::{Arc, Mutex};

use crate::dispatcher::routing::sticky_sample;

/// In-memory catalog using DashMap.
///
/// Keyed by `(NanoAgentId, NanoAgentVersion)`.  Per-tenant filtering
/// is done at read time.
///
/// **Sprint 8 — N13** : extended with per-(tenant, agent_id) rollout
/// policy storage.  Atomically updatable ; transitions validated.
#[derive(Debug, Default)]
pub struct InMemoryCatalog {
    entries: DashMap<(NanoAgentId, NanoAgentVersion), Arc<NanoAgent>>,
    /// **N13** : `(tenant_id, agent_id) → RolloutPolicy`.  Atomic
    /// updates via Mutex<Policy> ; the slot is fast to lock because
    /// only `set_rollout_policy` writes.
    policies: DashMap<(TenantId, NanoAgentId), Mutex<RolloutPolicy>>,
}

impl InMemoryCatalog {
    /// Create an empty catalog.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the total number of entries (across all tenants).
    /// Useful for tests.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true if the catalog is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Helper : check if `agent` is visible to `tenant`.
    fn is_visible(&self, agent: &NanoAgent, tenant: TenantId) -> bool {
        match &agent.author {
            Author::Efe => true,
            Author::Tenant { tenant_id } => *tenant_id == tenant,
            Author::Community { .. } => true,
            // **Sprint 10 — N18** : publisher-authored agents are
            // platform-global like EFE-shipped + community.  Cross-
            // tenant visibility (the publisher registry is the shared
            // governance layer ; the dispatch trace carries the
            // publisher id for audit).  Sprint 11 wires status-aware
            // filtering (Revoked / Suspended → hidden).
            Author::Publisher { .. } => true,
        }
    }

    /// Find the latest version of `id` matching the version requirement.
    fn find_latest_matching(
        &self,
        r: &NanoAgentRef,
        tenant: TenantId,
    ) -> Option<Arc<NanoAgent>> {
        let mut candidates: Vec<Arc<NanoAgent>> = self
            .entries
            .iter()
            .filter_map(|kv| {
                let (id, version) = kv.key();
                if id != &r.id {
                    return None;
                }
                if !r.version.matches(version) {
                    return None;
                }
                let agent = kv.value();
                if !self.is_visible(agent, tenant) {
                    return None;
                }
                Some(Arc::clone(agent))
            })
            .collect();
        candidates.sort_by(|a, b| b.version.as_semver().cmp(&a.version.as_semver()));
        candidates.into_iter().next()
    }
}

#[async_trait]
impl Catalog for InMemoryCatalog {
    async fn get(
        &self,
        r: &NanoAgentRef,
        tenant: TenantId,
    ) -> CatalogResult<Arc<NanoAgent>> {
        self.find_latest_matching(r, tenant).ok_or_else(|| {
            format!(
                "no version of '{}' matches '{}'",
                r.id.as_str(),
                r.version.as_str()
            )
            .into()
        })
    }

    async fn list(
        &self,
        tenant: TenantId,
        filter: ListFilter,
    ) -> CatalogResult<Vec<Arc<NanoAgent>>> {
        let mut all: Vec<Arc<NanoAgent>> = self
            .entries
            .iter()
            .filter_map(|kv| {
                let agent = kv.value();

                // Visibility check.
                let visible = match &agent.author {
                    Author::Efe => filter.include_efe_shipped,
                    Author::Tenant { tenant_id } => {
                        filter.include_tenant_owned && *tenant_id == tenant
                    }
                    Author::Community { .. } => filter.include_community,
                    // **Sprint 10 — N18** : publisher-authored agents
                    // ride the community visibility flag for Sprint
                    // 10.  Sprint 11 promotes this to a dedicated
                    // `include_publisher` field on `ListFilter`.
                    Author::Publisher { .. } => filter.include_community,
                };
                if !visible {
                    return None;
                }

                // Category filter.
                if let Some(cat) = filter.category {
                    if agent.category != cat {
                        return None;
                    }
                }

                // Stability tier filter (≥ min).
                if let Some(min_tier) = filter.stability_tier_min {
                    if agent.stability_tier < min_tier {
                        return None;
                    }
                }

                Some(Arc::clone(agent))
            })
            .collect();

        // Sort by id, then version desc.
        all.sort_by(|a, b| {
            a.id.as_str()
                .cmp(b.id.as_str())
                .then_with(|| b.version.as_semver().cmp(&a.version.as_semver()))
        });

        // Pagination.
        let offset = filter.offset as usize;
        let limit = filter.limit as usize;
        let result = all.into_iter().skip(offset).take(limit).collect();
        Ok(result)
    }

    async fn search(
        &self,
        tenant: TenantId,
        query: &str,
    ) -> CatalogResult<Vec<Arc<NanoAgent>>> {
        let q = query.to_lowercase();
        let result: Vec<Arc<NanoAgent>> = self
            .entries
            .iter()
            .filter_map(|kv| {
                let agent = kv.value();
                if !self.is_visible(agent, tenant) {
                    return None;
                }
                let id_lower = agent.id.as_str().to_lowercase();
                let desc_lower = agent.description.to_lowercase();
                if id_lower.contains(&q) || desc_lower.contains(&q) {
                    Some(Arc::clone(agent))
                } else {
                    None
                }
            })
            .collect();
        Ok(result)
    }

    async fn publish(
        &self,
        tenant: TenantId,
        agent: NanoAgent,
    ) -> CatalogResult<NanoAgentId> {
        // **Sprint 8 — N14** : Reserve the `swarm_envelope` agent id for
        // the envelope-trace sentinel.  Refuse any publish attempt that
        // uses it — otherwise envelope traces would collide with
        // tenant-published agents.
        if agent.id.as_str() == crate::dispatcher::envelope::SWARM_ENVELOPE_AGENT_ID {
            return Err(format!(
                "agent id '{}' is reserved for the N14 swarm envelope sentinel ; \
                 publishes rejected",
                agent.id.as_str()
            )
            .into());
        }

        // Authorization check : tenant can only publish under their own author.
        if let Author::Tenant { tenant_id } = &agent.author {
            if *tenant_id != tenant {
                return Err(format!(
                    "tenant {tenant} not authorized to publish under tenant {tenant_id}"
                )
                .into());
            }
        }

        let id = agent.id.clone();
        let version_str = agent.version.as_str().to_string();
        let key = (agent.id.clone(), agent.version.clone());

        // Atomic check-and-insert via DashMap::entry — the bucket is locked
        // for the duration of the Entry, preventing TOCTOU races between
        // contains_key + insert.
        match self.entries.entry(key) {
            dashmap::mapref::entry::Entry::Occupied(_) => Err(format!(
                "agent '{}' v{} already exists",
                id.as_str(),
                version_str
            )
            .into()),
            dashmap::mapref::entry::Entry::Vacant(slot) => {
                slot.insert(Arc::new(agent));
                Ok(id)
            }
        }
    }

    async fn unpublish(
        &self,
        tenant: TenantId,
        id: &NanoAgentId,
        version: &NanoAgentVersion,
    ) -> CatalogResult<()> {
        let key = (id.clone(), version.clone());

        // Authorization check (scoped to release the Ref before removal).
        {
            let entry = self.entries.get(&key);
            if let Some(e) = entry {
                match &e.author {
                    Author::Efe => {
                        return Err("cannot unpublish EFE-shipped agent".into());
                    }
                    Author::Tenant { tenant_id } => {
                        if *tenant_id != tenant {
                            return Err(format!(
                                "tenant {} not authorized to unpublish '{}' v{}",
                                tenant,
                                id.as_str(),
                                version.as_str()
                            )
                            .into());
                        }
                    }
                    Author::Community { .. } => {
                        // Community agents can be unpublished by any tenant (admin-only in prod).
                        // Phase 1 : permissive.
                    }
                    // **Sprint 10 — N18** : publisher-authored agents
                    // can only be unpublished by operators (Sprint 11
                    // wires the signed revocation cert + ledger
                    // entry).  Sprint 10 : operator-only.
                    Author::Publisher { .. } => {
                        if tenant != TenantId::system() {
                            return Err(format!(
                                "tenant {tenant} cannot unpublish publisher-authored \
                                 agent '{}' — operator-only (Sprint 11 wires signed \
                                 revocation)",
                                id.as_str()
                            )
                            .into());
                        }
                    }
                }
            }
            // `entry` Ref dropped at end of scope.
        }

        self.entries.remove(&key);
        Ok(())
    }

    async fn promote(
        &self,
        id: &NanoAgentId,
        version: &NanoAgentVersion,
        to: StabilityTier,
    ) -> CatalogResult<()> {
        let key = (id.clone(), version.clone());
        let mut entry = self
            .entries
            .get_mut(&key)
            .ok_or_else(|| format!("agent '{}' v{} not found", id.as_str(), version.as_str()))?;
        // Cannot demote — verify monotonic promotion.
        if to < entry.stability_tier {
            return Err(format!(
                "cannot demote from {:?} to {:?}",
                entry.stability_tier, to
            )
            .into());
        }
        // Replace the Arc with one carrying the new tier.
        let agent = (**entry.value()).clone();
        let mut new_agent = agent;
        new_agent.stability_tier = to;
        *entry = Arc::new(new_agent);
        Ok(())
    }

    // ─────────────────────────────────────────────────────────────────
    //  Sprint 8 — N13 : rollout policy API
    // ─────────────────────────────────────────────────────────────────

    async fn resolve_for_dispatch(
        &self,
        tenant: TenantId,
        agent_id: &NanoAgentId,
        version_req: &VersionReq,
        input_hash: &[u8; 32],
    ) -> CatalogResult<ResolvedAgent> {
        // 1. Version-pin opt-out : if version_req is an exact pin, bypass
        //    policy and resolve directly.  The caller wanted that exact
        //    version, presumably for reproducibility / audit.
        let r = NanoAgentRef {
            id: agent_id.clone(),
            version: version_req.clone(),
        };
        if version_req.is_exact_pin() {
            return Ok(ResolvedAgent::Single(self.get(&r, tenant).await?));
        }

        // 2. Look up the policy.  If none ⇒ Stable(latest).
        let policy = self.get_rollout_policy(tenant, agent_id).await?;

        // 3. **Bug-fix N13** : the policy's prod_version (and any
        //    second_version) MUST satisfy the caller's version_req.  If
        //    not, the caller asked for something incompatible with the
        //    current policy — fall back to the standard latest-match
        //    resolution.  Without this check, a caller asking for
        //    `^2.0` would silently receive v1.0.0 when the policy is
        //    `Stable(1.0.0)` — a correctness violation.
        let prod_in_range = version_req.matches(policy.prod_version());
        let second_in_range = policy
            .second_version()
            .map(|v| version_req.matches(v))
            .unwrap_or(true);
        if !prod_in_range || !second_in_range {
            // Policy version(s) don't satisfy caller's range — fall back
            // to standard latest-match resolution.  Logged so platform
            // engineers can spot policy / caller drift.
            tracing::debug!(
                target: "efe_nano::catalog",
                ?tenant,
                agent_id = %agent_id.as_str(),
                version_req = %version_req.as_str(),
                policy_kind = policy.kind_str(),
                "rollout policy versions don't satisfy version_req ; falling back to standard get()"
            );
            return Ok(ResolvedAgent::Single(self.get(&r, tenant).await?));
        }

        // 4. Branch on policy kind.
        match policy {
            RolloutPolicy::Stable { version } => {
                let agent = self.get_exact_version(agent_id, &version, tenant)?;
                Ok(ResolvedAgent::Single(agent))
            }
            RolloutPolicy::Shadow {
                prod_version,
                shadow_version,
                comparison,
                cost_attribution,
                shadow_budget_cents,
                shadow_on_cache_hit,
                ..
            } => {
                let prod = self.get_exact_version(agent_id, &prod_version, tenant)?;
                let shadow = self.get_exact_version(agent_id, &shadow_version, tenant)?;
                Ok(ResolvedAgent::ShadowPair {
                    prod,
                    shadow,
                    comparison,
                    cost_attribution,
                    shadow_budget_cents,
                    shadow_on_cache_hit,
                })
            }
            RolloutPolicy::Canary {
                prod_version,
                canary_version,
                canary_bps,
                ..
            } => {
                // Sticky sample — same input always lands on the same variant.
                let sample = sticky_sample(tenant, agent_id, input_hash);
                let chosen = if sample < canary_bps {
                    canary_version
                } else {
                    prod_version
                };
                let agent = self.get_exact_version(agent_id, &chosen, tenant)?;
                Ok(ResolvedAgent::Single(agent))
            }
        }
    }

    async fn set_rollout_policy(
        &self,
        tenant: TenantId,
        agent_id: &NanoAgentId,
        policy: RolloutPolicy,
    ) -> CatalogResult<RolloutPolicy> {
        // 1. Validate both referenced versions exist in the catalog and
        //    are visible to `tenant`.
        let prod_v = policy.prod_version();
        let _ = self.get_exact_version(agent_id, prod_v, tenant)?;
        if let Some(other) = policy.second_version() {
            let _ = self.get_exact_version(agent_id, other, tenant)?;
        }

        // 2. **Bug-fix N13 (B7)** : Cross-tenant authorization check.
        //    Architecture § 16.3 : "A tenant can only set policies on
        //    agents it OWNS".  Operators (TenantId::system()) may set
        //    policy on EFE-shipped agents ; tenants may only set policy
        //    on their own tenant-authored agents.
        let agent = self.get_exact_version(agent_id, prod_v, tenant)?;
        match &agent.author {
            Author::Efe => {
                if tenant != TenantId::system() {
                    return Err(format!(
                        "tenant {tenant} cannot set policy on EFE-shipped agent '{}' \
                         — operator-only",
                        agent_id.as_str()
                    )
                    .into());
                }
            }
            Author::Tenant { tenant_id } => {
                if *tenant_id != tenant {
                    return Err(format!(
                        "tenant {tenant} cannot set policy on tenant-{tenant_id}-owned \
                         agent '{}'",
                        agent_id.as_str()
                    )
                    .into());
                }
            }
            Author::Community { .. } => {
                // Community-authored agents : only operator may policy.
                if tenant != TenantId::system() {
                    return Err(format!(
                        "tenant {tenant} cannot set policy on community-authored \
                         agent '{}' — operator-only",
                        agent_id.as_str()
                    )
                    .into());
                }
            }
            // **Sprint 10 — N18** : publisher-authored agents — only
            // operator may set policy (matches the community rule).
            // Sprint 11 will allow the publisher themselves via signed
            // RolloutPolicyProof.
            Author::Publisher { .. } => {
                if tenant != TenantId::system() {
                    return Err(format!(
                        "tenant {tenant} cannot set policy on publisher-authored \
                         agent '{}' — operator-only (Sprint 11 wires publisher proof)",
                        agent_id.as_str()
                    )
                    .into());
                }
            }
        }

        // 3. SideEffect agents may NOT be shadowed.
        if matches!(agent.determinism, DeterminismLevel::SideEffect)
            && matches!(policy, RolloutPolicy::Shadow { .. })
        {
            return Err(format!(
                "cannot shadow agent '{}' : DeterminismLevel::SideEffect (double side-effect risk)",
                agent_id.as_str()
            )
            .into());
        }

        // 4. Semantic comparator cycle-avoidance : the comparator must
        //    not be the same as the agent being shadowed.
        if let RolloutPolicy::Shadow {
            comparison: ComparisonStrategy::Semantic { comparator_agent_id },
            ..
        } = &policy
        {
            if comparator_agent_id == agent_id {
                return Err("comparator agent cannot be the agent being shadowed".into());
            }
        }

        // 5. Read prior policy under lock + validate transition.
        let key = (tenant, agent_id.clone());
        let prior_policy = match self.policies.get(&key) {
            Some(slot) => slot.lock().expect("policy mutex poisoned").clone(),
            None => RolloutPolicy::Stable {
                version: agent.version.clone(),
            },
        };
        validate_transition(&prior_policy, &policy)?;

        // 5. Atomic upsert.
        self.policies
            .entry(key)
            .and_modify(|slot| {
                // Inner Mutex is on an exclusive ref from `and_modify`,
                // but clippy expects us to use `get_mut` ; we still need
                // the lock for symmetry with concurrent readers via
                // `.get(&key)`.  Suppress per-call.
                #[allow(clippy::mut_mutex_lock)]
                let mut guard = slot.lock().expect("policy mutex poisoned");
                *guard = policy.clone();
            })
            .or_insert_with(|| Mutex::new(policy.clone()));

        Ok(prior_policy)
    }

    async fn get_rollout_policy(
        &self,
        tenant: TenantId,
        agent_id: &NanoAgentId,
    ) -> CatalogResult<RolloutPolicy> {
        let key = (tenant, agent_id.clone());
        if let Some(slot) = self.policies.get(&key) {
            let guard = slot.lock().expect("policy mutex poisoned");
            return Ok(guard.clone());
        }
        // Default : Stable(latest visible).
        let r = NanoAgentRef::new(agent_id.clone());
        let agent = self.get(&r, tenant).await?;
        Ok(RolloutPolicy::Stable {
            version: agent.version.clone(),
        })
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Sprint 8 N13 helpers — exact-version lookup + transition validator
// ─────────────────────────────────────────────────────────────────────

impl InMemoryCatalog {
    /// Look up an exact (agent_id, version) within `tenant`'s visibility.
    /// Returns a `VersionNotFound`-shaped error if absent.
    pub(crate) fn get_exact_version(
        &self,
        agent_id: &NanoAgentId,
        version: &NanoAgentVersion,
        tenant: TenantId,
    ) -> CatalogResult<Arc<NanoAgent>> {
        let key = (agent_id.clone(), version.clone());
        let entry = self
            .entries
            .get(&key)
            .ok_or_else(|| format!("agent '{}' v{} not found", agent_id.as_str(), version.as_str()))?;
        if !self.is_visible(&entry, tenant) {
            return Err(format!(
                "agent '{}' v{} not visible to tenant {}",
                agent_id.as_str(),
                version.as_str(),
                tenant
            )
            .into());
        }
        Ok(Arc::clone(&entry))
    }

    /// Snapshot of all live rollout policies — for ops / metrics.
    pub fn rollout_policy_count(&self) -> usize {
        self.policies.len()
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Transition validator
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N13 § 5.2** : Enforce the legal-transition matrix.
///
/// Stable → anything  : always allowed.
/// Shadow(p, s) → Stable(p)  : "drop shadow" path.
/// Shadow(p, s) → Canary(p, s, _)  : "promote shadow to canary".
/// Canary(p, c, _) → Stable(p)  : "rollback".
/// Canary(p, c, _) → Stable(c)  : "promote".
/// Canary(p, c, X%) → Canary(p, c, Y%)  : "ramp" (Y > X or Y < X both OK).
/// Anything else : rejected with `IllegalTransition`.
pub(crate) fn validate_transition(
    prev: &RolloutPolicy,
    next: &RolloutPolicy,
) -> CatalogResult<()> {
    use RolloutPolicy::*;
    match (prev, next) {
        // From Stable, anything is allowed.
        (Stable { .. }, _) => Ok(()),

        // Shadow → Stable(prod_version) = drop shadow.
        (
            Shadow { prod_version, .. },
            Stable { version: v },
        ) if v == prod_version => Ok(()),

        // Shadow → Canary(same prod, same shadow) = promote to canary.
        (
            Shadow { prod_version, shadow_version, .. },
            Canary { prod_version: pp, canary_version: cv, .. },
        ) if pp == prod_version && cv == shadow_version => Ok(()),

        // Canary → Stable(prod) = rollback ; Canary → Stable(canary) = promote.
        (
            Canary { prod_version, canary_version, .. },
            Stable { version: v },
        ) if v == prod_version || v == canary_version => Ok(()),

        // Canary → Canary(same versions, different bps) = ramping.
        (
            Canary { prod_version, canary_version, .. },
            Canary { prod_version: pp, canary_version: cv, .. },
        ) if pp == prod_version && cv == canary_version => Ok(()),

        // Anything else is rejected.
        (a, b) => Err(format!(
            "illegal rollout transition : {} → {}",
            a.kind_str(),
            b.kind_str()
        )
        .into()),
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use efe_nano_sdk::{
        AuditPolicy, Category, ComplianceTags, DeterminismLevel, ExecutionMode,
        IdempotencyPolicy, ResourceLimits, VersionReq,
    };

    fn make_agent(id: &str, version: &str, author: Author) -> NanoAgent {
        NanoAgent {
            id: NanoAgentId::new(id).unwrap(),
            version: NanoAgentVersion::new(version).unwrap(),
            description: format!("{id} test agent"),
            author,
            category: Category::Validation,
            stability_tier: StabilityTier::Tier1,
            execution_mode: ExecutionMode::PureRust {
                entry: format!("{id}::run"),
            },
            determinism: DeterminismLevel::FullDeterministic,
            idempotency: IdempotencyPolicy::auto_24h(),
            input_schema: serde_json::json!({"type": "object"}),
            output_schema: serde_json::json!({"type": "object"}),
            error_schema: None,
            limits: ResourceLimits::default_mode_a(),
            cost_estimate_cents: 0.01,
            latency_p99_ms_target: 50,
            audit: AuditPolicy::default(),
            compliance: ComplianceTags::default(),
            mcp_tool_metadata: None,
            signature: None,
            co_publishing: None,
            gold_set_ref: None,
            created_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn empty_catalog() {
        let c = InMemoryCatalog::new();
        assert!(c.is_empty());
        assert_eq!(c.len(), 0);
    }

    #[tokio::test]
    async fn publish_and_get() {
        let c = InMemoryCatalog::new();
        let tenant = TenantId::system();
        c.publish(tenant, make_agent("foo", "1.0.0", Author::Efe))
            .await
            .unwrap();
        let r = NanoAgentRef::new(NanoAgentId::new("foo").unwrap());
        let agent = c.get(&r, tenant).await.unwrap();
        assert_eq!(agent.id.as_str(), "foo");
        assert_eq!(agent.version.as_str(), "1.0.0");
    }

    #[tokio::test]
    async fn version_resolution_picks_latest() {
        let c = InMemoryCatalog::new();
        let tenant = TenantId::system();
        c.publish(tenant, make_agent("foo", "1.0.0", Author::Efe))
            .await
            .unwrap();
        c.publish(tenant, make_agent("foo", "1.2.3", Author::Efe))
            .await
            .unwrap();
        c.publish(tenant, make_agent("foo", "2.0.0", Author::Efe))
            .await
            .unwrap();
        // "^1.0" should pick 1.2.3 (latest 1.x)
        let r = NanoAgentRef::with_version(
            NanoAgentId::new("foo").unwrap(),
            VersionReq::new("^1.0").unwrap(),
        );
        let agent = c.get(&r, tenant).await.unwrap();
        assert_eq!(agent.version.as_str(), "1.2.3");

        // "*" should pick 2.0.0 (latest overall)
        let r = NanoAgentRef::new(NanoAgentId::new("foo").unwrap());
        let agent = c.get(&r, tenant).await.unwrap();
        assert_eq!(agent.version.as_str(), "2.0.0");
    }

    #[tokio::test]
    async fn get_returns_error_when_no_match() {
        let c = InMemoryCatalog::new();
        let tenant = TenantId::system();
        let r = NanoAgentRef::new(NanoAgentId::new("foo").unwrap());
        let err = c.get(&r, tenant).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn duplicate_publish_rejected() {
        let c = InMemoryCatalog::new();
        let tenant = TenantId::system();
        c.publish(tenant, make_agent("foo", "1.0.0", Author::Efe))
            .await
            .unwrap();
        let err = c
            .publish(tenant, make_agent("foo", "1.0.0", Author::Efe))
            .await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn concurrent_publish_of_same_version_yields_exactly_one_success() {
        // Concurrency stress-test : 32 tasks all racing to publish the same
        // (id, version).  Without the TOCTOU fix (entry-based atomic
        // check-and-insert), several tasks could pass the contains_key check
        // before any inserts and we'd end up with multiple successes.
        let c = Arc::new(InMemoryCatalog::new());
        let tenant = TenantId::system();

        let mut handles = Vec::new();
        for _ in 0..32 {
            let c = Arc::clone(&c);
            let agent = make_agent("racer", "1.0.0", Author::Efe);
            handles.push(tokio::spawn(async move { c.publish(tenant, agent).await }));
        }

        let mut successes = 0;
        let mut failures = 0;
        for h in handles {
            match h.await.unwrap() {
                Ok(_) => successes += 1,
                Err(_) => failures += 1,
            }
        }
        assert_eq!(
            successes, 1,
            "concurrent publish of same (id, version) must yield exactly 1 success"
        );
        assert_eq!(failures, 31);

        // Only one entry must exist in the catalog.
        assert_eq!(c.len(), 1);
    }

    #[tokio::test]
    async fn tenant_isolation_in_get() {
        let c = InMemoryCatalog::new();
        let tenant_a = TenantId::new();
        let tenant_b = TenantId::new();
        c.publish(
            tenant_a,
            make_agent("foo", "1.0.0", Author::Tenant { tenant_id: tenant_a }),
        )
        .await
        .unwrap();

        // tenant_a can see their agent
        let r = NanoAgentRef::new(NanoAgentId::new("foo").unwrap());
        assert!(c.get(&r, tenant_a).await.is_ok());
        // tenant_b cannot
        assert!(c.get(&r, tenant_b).await.is_err());
    }

    #[tokio::test]
    async fn list_filters_by_category() {
        let c = InMemoryCatalog::new();
        let tenant = TenantId::system();
        c.publish(tenant, make_agent("a", "1.0.0", Author::Efe))
            .await
            .unwrap();
        let r = c
            .list(tenant, ListFilter::by_category(Category::Validation))
            .await
            .unwrap();
        assert_eq!(r.len(), 1);

        let r = c
            .list(tenant, ListFilter::by_category(Category::Extraction))
            .await
            .unwrap();
        assert_eq!(r.len(), 0);
    }

    #[tokio::test]
    async fn list_pagination() {
        let c = InMemoryCatalog::new();
        let tenant = TenantId::system();
        for i in 0..10 {
            c.publish(tenant, make_agent(&format!("a{i}"), "1.0.0", Author::Efe))
                .await
                .unwrap();
        }
        let filter = ListFilter::include_all().with_pagination(3, 0);
        let r = c.list(tenant, filter).await.unwrap();
        assert_eq!(r.len(), 3);

        let filter = ListFilter::include_all().with_pagination(3, 9);
        let r = c.list(tenant, filter).await.unwrap();
        assert_eq!(r.len(), 1);  // only 10 entries total
    }

    #[tokio::test]
    async fn search_finds_by_description() {
        let c = InMemoryCatalog::new();
        let tenant = TenantId::system();
        c.publish(tenant, make_agent("foo_bar", "1.0.0", Author::Efe))
            .await
            .unwrap();

        let r = c.search(tenant, "foo").await.unwrap();
        assert_eq!(r.len(), 1);

        let r = c.search(tenant, "FOO").await.unwrap();  // case-insensitive
        assert_eq!(r.len(), 1);

        let r = c.search(tenant, "nonexistent").await.unwrap();
        assert_eq!(r.len(), 0);
    }

    #[tokio::test]
    async fn unpublish_tenant_owned() {
        let c = InMemoryCatalog::new();
        let tenant = TenantId::new();
        c.publish(
            tenant,
            make_agent("foo", "1.0.0", Author::Tenant { tenant_id: tenant }),
        )
        .await
        .unwrap();
        let id = NanoAgentId::new("foo").unwrap();
        let v = NanoAgentVersion::new("1.0.0").unwrap();
        c.unpublish(tenant, &id, &v).await.unwrap();
        // Should be gone
        let r = NanoAgentRef::new(id);
        assert!(c.get(&r, tenant).await.is_err());
    }

    #[tokio::test]
    async fn cannot_unpublish_efe_shipped() {
        let c = InMemoryCatalog::new();
        let tenant = TenantId::system();
        c.publish(tenant, make_agent("foo", "1.0.0", Author::Efe))
            .await
            .unwrap();
        let id = NanoAgentId::new("foo").unwrap();
        let v = NanoAgentVersion::new("1.0.0").unwrap();
        let err = c.unpublish(tenant, &id, &v).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn promote_monotonic() {
        let c = InMemoryCatalog::new();
        let tenant = TenantId::system();
        c.publish(tenant, make_agent("foo", "1.0.0", Author::Efe))
            .await
            .unwrap();
        let id = NanoAgentId::new("foo").unwrap();
        let v = NanoAgentVersion::new("1.0.0").unwrap();

        // Tier1 → Tier2 OK
        c.promote(&id, &v, StabilityTier::Tier2).await.unwrap();
        let r = NanoAgentRef::new(id.clone());
        let agent = c.get(&r, tenant).await.unwrap();
        assert_eq!(agent.stability_tier, StabilityTier::Tier2);

        // Tier2 → Tier1 (demotion) rejected
        let err = c.promote(&id, &v, StabilityTier::Tier1).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn list_include_efe_filter_off() {
        let c = InMemoryCatalog::new();
        let tenant = TenantId::system();
        c.publish(tenant, make_agent("foo", "1.0.0", Author::Efe))
            .await
            .unwrap();
        let mut filter = ListFilter::include_all();
        filter.include_efe_shipped = false;
        let r = c.list(tenant, filter).await.unwrap();
        assert_eq!(r.len(), 0);
    }
}
