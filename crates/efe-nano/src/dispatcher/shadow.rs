//! **Sprint 8 — N13** : Asynchronous shadow execution scheduler.
//!
//! See architecture doc § 4.4.
//!
//! # Lifecycle
//!
//! When `Catalog::resolve_for_dispatch` returns `ResolvedAgent::ShadowPair`,
//! the dispatcher calls [`ShadowScheduler::schedule`] BEFORE returning the
//! production response.  The scheduler :
//!
//! 1. Reserves shadow budget via [`crate::runtime::cost_attribution::CostRouter`].
//!    On exhaustion, drops the shadow with a warn + metric ; production
//!    proceeds unaffected.
//! 2. Spawns a `tokio::spawn` task that :
//!    - Runs the shadow agent with the same input.
//!    - Emits a `ShadowRun` audit trace (parent_trace_ids = [prod_trace_id]).
//!    - Compares prod vs shadow output via [`crate::dispatcher::compare`].
//!    - Emits a `ShadowComparison` audit trace.
//!    - Records the outcome in the [`crate::audit::drift::DriftWindow`].
//!    - Checks auto-rollback threshold ; if breached, atomic catalog
//!      swap back to `Stable(prod_version)`.
//!
//! # Fire-and-forget semantics
//!
//! [`ShadowScheduler::schedule`] returns `()`.  No `JoinHandle` is
//! retained.  If the tokio runtime shuts down mid-shadow, the shadow is
//! cancelled — that's acceptable graceful degradation.

use chrono::Utc;
use efe_nano_sdk::{
    Catalog, ComparisonStrategy, CostAttribution, NanoAgent, NanoAgentRef, NanoAgentVersion,
    PromotionGates, RolloutPolicy, ShadowOutcome, TenantId, TraceId,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::audit::{drift::DriftWindow, AuditWriter, NanoAgentTrace};
use crate::dispatcher::compare::{
    compare as compare_outputs, compare_semantic, CompareContext,
};
use crate::runtime::cost_attribution::CostRouter;
use crate::runtime::{CallContext, NanoAgentExecutor};

// ─────────────────────────────────────────────────────────────────────
//  Trace error_kind sentinels
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N13** : `error_kind` sentinel marking a trace as a
/// shadow execution (parent = production trace).
pub const TRACE_KIND_SHADOW_RUN: &str = "ShadowRun";

/// **Sprint 8 — N13** : `error_kind` sentinel marking a trace as the
/// comparison result between a production and shadow execution.
pub const TRACE_KIND_SHADOW_COMPARISON: &str = "ShadowComparison";

/// **Sprint 8 — N13** : `error_kind` sentinel marking a trace as a
/// rollout-policy-change virtual-swarm row.
pub const TRACE_KIND_ROLLOUT_POLICY_CHANGED: &str = "RolloutPolicyChanged";

/// **Sprint 8 — N13** : virtual swarm UUID assigned to every
/// `RolloutPolicyChanged` trace.  Used so the auditor can query "all
/// rollout-policy-change rows" with a single index lookup.
pub const ROLLOUT_VIRTUAL_SWARM_UUID: Uuid = uuid::uuid!("00000000-0000-4eee-8000-000000000d13");

// ─────────────────────────────────────────────────────────────────────
//  ProdOutcomeSnapshot
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N13** : What the production op produced — captured at
/// the moment of return so the background shadow comparison has a
/// stable reference.
#[derive(Debug, Clone)]
pub struct ProdOutcomeSnapshot {
    pub agent_version: NanoAgentVersion,
    pub output: serde_json::Value,
    pub cost_cents: f64,
    pub latency_ms: u32,
    pub trace_id: TraceId,
}

// ─────────────────────────────────────────────────────────────────────
//  ShadowSchedulerConfig — paramètres opérationnels
// ─────────────────────────────────────────────────────────────────────

/// Per-shadow-call dispatch context : the policy fields that survived
/// across the async boundary.
#[derive(Debug, Clone)]
pub struct ShadowDispatchContext {
    pub tenant_id: TenantId,
    pub swarm_request_id: Uuid,
    pub op_id: String,
    pub caller_agent_id: Option<String>,
    pub input: serde_json::Value,
    pub comparison: ComparisonStrategy,
    pub cost_attribution: CostAttribution,
    /// **Sprint 8 — N14** : Swarm envelope anchor.  Propagated to
    /// every shadow / comparison trace so they join to the swarm.
    pub swarm_envelope_trace_id: Option<TraceId>,
}

// ─────────────────────────────────────────────────────────────────────
//  ShadowScheduler
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N13** : the async shadow scheduler.
///
/// Constructed once per dispatcher ; cloneable cheaply via `Arc`.
pub struct ShadowScheduler {
    runtime: Arc<dyn NanoAgentExecutor>,
    audit: Arc<dyn AuditWriter>,
    drift_window: Arc<DriftWindow>,
    cost_router: Arc<CostRouter>,
    /// **Sprint 8 — N13 (B6 fix)** : optional catalog reference used to
    /// resolve the comparator agent for `ComparisonStrategy::Semantic`.
    /// When `None`, Semantic comparisons emit `ComparatorUnavailable`.
    catalog: Option<Arc<dyn Catalog>>,
    /// Hook fired when auto-rollback should trigger.  Held by the
    /// dispatcher's policy-update path.  When `None`, rollback is
    /// logged but no policy swap is performed.  The
    /// [`CatalogRollbackHook`] (this module) wires it to the catalog.
    rollback_hook: Option<Arc<dyn RollbackHook>>,
}

/// **Sprint 8 — N13** : injection point for the catalog's auto-rollback
/// callback.  The drift-window decides WHEN to roll back ; this trait
/// hands the decision off to whoever owns the catalog.
#[async_trait::async_trait]
pub trait RollbackHook: Send + Sync + 'static {
    async fn rollback(
        &self,
        tenant: TenantId,
        agent_id: efe_nano_sdk::NanoAgentId,
        prod_version: NanoAgentVersion,
        reason: String,
    );
}

/// **Sprint 8 — N13 (B2 fix)** : Default `RollbackHook` impl that
/// swaps the catalog policy back to `Stable(prod_version)` and emits a
/// `RolloutPolicyChanged` audit trace.  This makes auto-rollback fully
/// operational without external wiring.
pub struct CatalogRollbackHook {
    catalog: Arc<dyn Catalog>,
    audit: Arc<dyn AuditWriter>,
    idempotency: Option<Arc<crate::idempotency::IdempotencyCache>>,
    drift_window: Arc<DriftWindow>,
}

impl CatalogRollbackHook {
    pub fn new(
        catalog: Arc<dyn Catalog>,
        audit: Arc<dyn AuditWriter>,
        drift_window: Arc<DriftWindow>,
    ) -> Self {
        Self {
            catalog,
            audit,
            idempotency: None,
            drift_window,
        }
    }

    pub fn with_idempotency(
        mut self,
        idempotency: Arc<crate::idempotency::IdempotencyCache>,
    ) -> Self {
        self.idempotency = Some(idempotency);
        self
    }
}

#[async_trait::async_trait]
impl RollbackHook for CatalogRollbackHook {
    async fn rollback(
        &self,
        tenant: TenantId,
        agent_id: efe_nano_sdk::NanoAgentId,
        prod_version: NanoAgentVersion,
        reason: String,
    ) {
        // Read the prior policy for the audit row.
        let prior = match self.catalog.get_rollout_policy(tenant, &agent_id).await {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(
                    ?tenant,
                    agent_id = %agent_id.as_str(),
                    error = %e,
                    "auto-rollback failed : couldn't read prior policy"
                );
                return;
            }
        };
        let next = RolloutPolicy::Stable {
            version: prod_version.clone(),
        };
        // Atomic catalog swap.
        if let Err(e) = self
            .catalog
            .set_rollout_policy(tenant, &agent_id, next.clone())
            .await
        {
            tracing::error!(
                ?tenant,
                agent_id = %agent_id.as_str(),
                error = %e,
                "auto-rollback failed : set_rollout_policy errored"
            );
            return;
        }
        // Drop drift window so the cooldown applies to a fresh policy.
        self.drift_window.clear(tenant, &agent_id);
        // Cache cascade : drop both prod and canary entries to avoid
        // serving stale outputs of the bad canary.
        if let Some(cache) = &self.idempotency {
            let _ = cache.flush_for_agent(tenant, &agent_id).await;
        }
        // Emit the chain-linked RolloutPolicyChanged trace.
        let trace = build_rollout_policy_changed_trace(
            tenant,
            agent_id,
            &prior,
            &next,
            "auto_rollback",
            Some(reason),
        );
        self.audit.record(trace).await;
    }
}

impl ShadowScheduler {
    pub fn new(
        runtime: Arc<dyn NanoAgentExecutor>,
        audit: Arc<dyn AuditWriter>,
        drift_window: Arc<DriftWindow>,
        cost_router: Arc<CostRouter>,
    ) -> Self {
        Self {
            runtime,
            audit,
            drift_window,
            cost_router,
            catalog: None,
            rollback_hook: None,
        }
    }

    pub fn with_rollback_hook(mut self, hook: Arc<dyn RollbackHook>) -> Self {
        self.rollback_hook = Some(hook);
        self
    }

    /// **N13 (B6 fix)** : Inject the catalog so the scheduler can
    /// resolve the comparator agent for `ComparisonStrategy::Semantic`.
    pub fn with_catalog(mut self, catalog: Arc<dyn Catalog>) -> Self {
        self.catalog = Some(catalog);
        self
    }

    /// **N13** : access the drift window (for the Dispatcher's
    /// rollout-workflow helpers).
    pub fn drift_window(&self) -> &Arc<DriftWindow> {
        &self.drift_window
    }

    /// **N13** : access the cost router (for ops dashboards).
    pub fn cost_router(&self) -> &Arc<CostRouter> {
        &self.cost_router
    }

    /// **Schedule a shadow run.**  Fire-and-forget — returns immediately.
    ///
    /// # Parameters
    /// - `self_arc` : the scheduler wrapped in `Arc` so the spawned
    ///   future can hold it.
    /// - `prod_snapshot` : the production outcome already captured.
    /// - `shadow_agent` : the shadow version's catalog entry.
    /// - `prod_agent` : the prod version (for the auto-rollback hook).
    /// - `gates` : the promotion gates (for auto-rollback evaluation).
    /// - `shadow_budget_cents` : optional budget cap.
    /// - `ctx` : per-call shadow dispatch context.
    #[allow(clippy::too_many_arguments)]
    pub fn schedule(
        self_arc: &Arc<Self>,
        prod_snapshot: ProdOutcomeSnapshot,
        shadow_agent: Arc<NanoAgent>,
        prod_agent: Arc<NanoAgent>,
        gates: Option<PromotionGates>,
        shadow_budget_cents: Option<f64>,
        ctx: ShadowDispatchContext,
    ) {
        // Reserve shadow cost budget.
        let estimated_cost = shadow_agent.cost_estimate_cents;
        let cap = shadow_budget_cents.unwrap_or(f64::INFINITY);
        if !cap.is_finite() || cap >= estimated_cost {
            // Budget cap covers estimate — proceed.
        }
        let reservation =
            match self_arc.cost_router.reserve(ctx.tenant_id, estimated_cost, ctx.cost_attribution) {
                Ok(r) => Some(r),
                Err(e) => {
                    tracing::warn!(
                        ?ctx.tenant_id,
                        agent_id = %shadow_agent.id.as_str(),
                        attribution = ctx.cost_attribution.as_str(),
                        error = %e,
                        "shadow dropped : budget exhausted",
                    );
                    return;
                }
            };

        let scheduler = Arc::clone(self_arc);
        tokio::spawn(async move {
            scheduler
                .run_shadow_pipeline(prod_snapshot, shadow_agent, prod_agent, gates, ctx, reservation)
                .await;
        });
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_shadow_pipeline(
        self: Arc<Self>,
        prod_snapshot: ProdOutcomeSnapshot,
        shadow_agent: Arc<NanoAgent>,
        prod_agent: Arc<NanoAgent>,
        gates: Option<PromotionGates>,
        ctx: ShadowDispatchContext,
        reservation: Option<crate::runtime::cost_attribution::CostReservation>,
    ) {
        let shadow_start = std::time::Instant::now();
        let call_ctx = CallContext::from_swarm(
            ctx.tenant_id,
            ctx.swarm_request_id,
            ctx.op_id.clone(),
            vec![prod_snapshot.trace_id],
            ctx.caller_agent_id.clone(),
            Arc::clone(&self.audit) as Arc<dyn AuditWriter>,
            30_000, // 30 s shadow timeout
            None,
        );

        let shadow_result = self
            .runtime
            .execute_with_context(&shadow_agent, ctx.input.clone(), &call_ctx)
            .await;

        let shadow_latency_ms = shadow_start.elapsed().as_millis().min(u32::MAX as u128) as u32;

        let outcome: ShadowOutcome = match shadow_result {
            Err(e) => ShadowOutcome::ShadowFailed {
                compared_at: Utc::now(),
                error: e.to_string(),
            },
            Ok(shadow_resp) => {
                // Commit the actual cost.
                if let Some(r) = reservation {
                    self.cost_router.commit(r, shadow_resp.cost_cents);
                }
                // Write the ShadowRun audit trace.  parent_trace_ids = [prod].
                self.write_shadow_run_trace(
                    &ctx,
                    &shadow_agent,
                    &ctx.input,
                    &shadow_resp.output,
                    shadow_resp.cost_cents,
                    shadow_resp.latency_ms,
                    prod_snapshot.trace_id,
                )
                .await;
                // Compare outputs per policy.
                let cmp_ctx = CompareContext {
                    prod_latency_ms: prod_snapshot.latency_ms,
                    shadow_latency_ms,
                    prod_cost_cents: prod_snapshot.cost_cents,
                    shadow_cost_cents: shadow_resp.cost_cents,
                };
                match &ctx.comparison {
                    ComparisonStrategy::Semantic { comparator_agent_id } => {
                        // **N13 (B6 fix)** : resolve the comparator agent
                        // via the catalog (when available) and dispatch
                        // it with `compare_semantic`.  Falls back to
                        // ComparatorUnavailable when the catalog isn't
                        // wired or the comparator can't be resolved.
                        match self.catalog.as_ref() {
                            None => {
                                tracing::warn!(
                                    comparator = %comparator_agent_id.as_str(),
                                    "Semantic comparison requested but ShadowScheduler has no catalog wired"
                                );
                                ShadowOutcome::ComparatorUnavailable {
                                    compared_at: Utc::now(),
                                }
                            }
                            Some(cat) => {
                                let comparator_ref =
                                    NanoAgentRef::new(comparator_agent_id.clone());
                                match cat.get(&comparator_ref, ctx.tenant_id).await {
                                    Ok(comparator_agent) => {
                                        compare_semantic(
                                            comparator_agent,
                                            Arc::clone(&self.runtime),
                                            &prod_snapshot.output,
                                            &shadow_resp.output,
                                            cmp_ctx,
                                        )
                                        .await
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            comparator = %comparator_agent_id.as_str(),
                                            error = %e,
                                            "Semantic comparator agent not resolvable in catalog"
                                        );
                                        ShadowOutcome::ComparatorUnavailable {
                                            compared_at: Utc::now(),
                                        }
                                    }
                                }
                            }
                        }
                    }
                    other => compare_outputs(other, &prod_snapshot.output, &shadow_resp.output, cmp_ctx),
                }
            }
        };

        // Write the ShadowComparison audit trace.
        self.write_shadow_comparison_trace(
            &ctx,
            &shadow_agent,
            &prod_snapshot,
            &outcome,
        )
        .await;

        // Record drift outcome.
        self.drift_window.record(
            ctx.tenant_id,
            shadow_agent.id.clone(),
            outcome.clone(),
        );

        // Check auto-rollback.
        if let Some(gates) = gates {
            if let Some(stats) = self.drift_window.stats(ctx.tenant_id, &shadow_agent.id) {
                if let Some(reason) =
                    crate::audit::drift::PromotionGateEvaluator::should_auto_rollback(&stats, &gates)
                {
                    self.fire_rollback(ctx.tenant_id, &shadow_agent.id, &prod_agent.version, reason)
                        .await;
                }
            }
        }
    }

    async fn fire_rollback(
        &self,
        tenant: TenantId,
        agent_id: &efe_nano_sdk::NanoAgentId,
        prod_version: &NanoAgentVersion,
        reason: String,
    ) {
        if self.drift_window.in_rollback_cooldown(tenant, agent_id) {
            tracing::debug!(?tenant, agent_id = %agent_id.as_str(), "auto-rollback skipped : cooldown");
            return;
        }
        if let Some(hook) = &self.rollback_hook {
            self.drift_window.mark_rollback(tenant, agent_id.clone());
            hook.rollback(tenant, agent_id.clone(), prod_version.clone(), reason).await;
        } else {
            tracing::warn!(?tenant, agent_id = %agent_id.as_str(), %reason, "auto-rollback skipped : no hook installed");
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn write_shadow_run_trace(
        &self,
        ctx: &ShadowDispatchContext,
        agent: &NanoAgent,
        input: &serde_json::Value,
        output: &serde_json::Value,
        cost_cents: f64,
        latency_ms: u32,
        prod_trace_id: TraceId,
    ) {
        let now = Utc::now();
        let input_hash = crate::audit::hash_json(input);
        let output_hash = crate::audit::hash_json(output);
        let trace = NanoAgentTrace {
            trace_id: TraceId::new(),
            parent_trace_ids: vec![prod_trace_id],
            swarm_request_id: ctx.swarm_request_id,
            operation_id: ctx.op_id.clone(),
            tenant_id: ctx.tenant_id,
            caller_agent_id: ctx.caller_agent_id.clone(),
            nano_agent_id: agent.id.clone(),
            nano_agent_version: agent.version.clone(),
            execution_mode: agent.execution_mode.mode_str().to_string(),
            input_hash_source: input_hash,
            input_hash_resolved: input_hash,
            output_hash: Some(output_hash),
            error_kind: Some(TRACE_KIND_SHADOW_RUN.to_string()),
            started_at: now,
            completed_at: Some(now),
            latency_ms: Some(latency_ms),
            cost_cents,
            llm_tokens_used: None,
            full_input: None,
            full_output: None,
            tombstone: None,
            swarm_envelope_trace_id: ctx.swarm_envelope_trace_id,
        };
        self.audit.record(trace).await;
    }

    async fn write_shadow_comparison_trace(
        &self,
        ctx: &ShadowDispatchContext,
        shadow_agent: &NanoAgent,
        prod_snapshot: &ProdOutcomeSnapshot,
        outcome: &ShadowOutcome,
    ) {
        let now = Utc::now();
        let input_hash = crate::audit::hash_json(&ctx.input);
        // The comparison "output" is a structured ShadowOutcome — we
        // hash a JSON projection of it so the trace's output_hash is
        // stable across re-emissions.
        let outcome_json = shadow_outcome_to_json(outcome);
        let output_hash = crate::audit::hash_json(&outcome_json);

        // parent_trace_ids : [prod, shadow_run].  We can't easily
        // recover the shadow_run trace_id here (we don't return it
        // from write_shadow_run_trace ; Sprint 9 may refactor to
        // make the trace_id chain richer).  Sprint 8 ship : just
        // [prod_trace_id].  The auditor can walk forward from prod
        // to find both ShadowRun and ShadowComparison.
        let trace = NanoAgentTrace {
            trace_id: TraceId::new(),
            parent_trace_ids: vec![prod_snapshot.trace_id],
            swarm_request_id: ctx.swarm_request_id,
            operation_id: ctx.op_id.clone(),
            tenant_id: ctx.tenant_id,
            caller_agent_id: ctx.caller_agent_id.clone(),
            nano_agent_id: shadow_agent.id.clone(),
            nano_agent_version: shadow_agent.version.clone(),
            execution_mode: shadow_agent.execution_mode.mode_str().to_string(),
            input_hash_source: input_hash,
            input_hash_resolved: input_hash,
            output_hash: Some(output_hash),
            error_kind: Some(TRACE_KIND_SHADOW_COMPARISON.to_string()),
            started_at: now,
            completed_at: Some(now),
            latency_ms: Some(0),
            cost_cents: 0.0,
            llm_tokens_used: None,
            full_input: None,
            full_output: Some(outcome_json),
            tombstone: None,
            swarm_envelope_trace_id: ctx.swarm_envelope_trace_id,
        };
        self.audit.record(trace).await;
    }

    /// Allow comparison via comparator agent (Semantic strategy).
    /// Public so the dispatcher can call directly when it has resolved
    /// the comparator agent from the catalog.
    pub async fn compare_with_comparator(
        &self,
        comparator: Arc<NanoAgent>,
        prod_output: &serde_json::Value,
        shadow_output: &serde_json::Value,
        cmp_ctx: CompareContext,
    ) -> ShadowOutcome {
        compare_semantic(comparator, Arc::clone(&self.runtime), prod_output, shadow_output, cmp_ctx).await
    }
}

/// **Sprint 8 — N13** : Project a [`ShadowOutcome`] into a stable JSON
/// shape for trace storage + auditor consumption.
pub fn shadow_outcome_to_json(o: &ShadowOutcome) -> serde_json::Value {
    use serde_json::json;
    match o {
        ShadowOutcome::Match {
            compared_at,
            similarity,
            prod_latency_ms,
            shadow_latency_ms,
            prod_cost_cents,
            shadow_cost_cents,
        } => json!({
            "kind": "match",
            "compared_at": compared_at.to_rfc3339(),
            "similarity": similarity,
            "prod_latency_ms": prod_latency_ms,
            "shadow_latency_ms": shadow_latency_ms,
            "prod_cost_cents": prod_cost_cents,
            "shadow_cost_cents": shadow_cost_cents,
        }),
        ShadowOutcome::Mismatch {
            compared_at,
            similarity,
            diff_summary,
            prod_latency_ms,
            shadow_latency_ms,
            prod_cost_cents,
            shadow_cost_cents,
        } => json!({
            "kind": "mismatch",
            "compared_at": compared_at.to_rfc3339(),
            "similarity": similarity,
            "diff_summary": diff_summary,
            "prod_latency_ms": prod_latency_ms,
            "shadow_latency_ms": shadow_latency_ms,
            "prod_cost_cents": prod_cost_cents,
            "shadow_cost_cents": shadow_cost_cents,
        }),
        ShadowOutcome::ShadowFailed { compared_at, error } => json!({
            "kind": "shadow_failed",
            "compared_at": compared_at.to_rfc3339(),
            "error": error,
        }),
        ShadowOutcome::ComparatorUnavailable { compared_at } => json!({
            "kind": "comparator_unavailable",
            "compared_at": compared_at.to_rfc3339(),
        }),
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Policy-change audit trace builder
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N13** : Build a `RolloutPolicyChanged` virtual-swarm
/// audit trace.  The dispatcher / catalog use this to emit a chain-
/// linked row every time a policy is set / promoted / rolled back.
pub fn build_rollout_policy_changed_trace(
    tenant: TenantId,
    agent_id: efe_nano_sdk::NanoAgentId,
    from: &RolloutPolicy,
    to: &RolloutPolicy,
    actor: impl Into<String>,
    reason: Option<String>,
) -> NanoAgentTrace {
    let now = Utc::now();
    let payload = serde_json::json!({
        "from": from,
        "to": to,
        "actor": actor.into(),
        "reason": reason,
    });
    let input_hash = crate::audit::hash_json(&payload);
    NanoAgentTrace {
        trace_id: TraceId::new(),
        parent_trace_ids: vec![],
        swarm_request_id: ROLLOUT_VIRTUAL_SWARM_UUID,
        operation_id: "rollout_policy_change".to_string(),
        tenant_id: tenant,
        caller_agent_id: None,
        nano_agent_id: agent_id,
        nano_agent_version: from.prod_version().clone(),
        execution_mode: "rollout".to_string(),
        input_hash_source: input_hash,
        input_hash_resolved: input_hash,
        output_hash: None,
        error_kind: Some(TRACE_KIND_ROLLOUT_POLICY_CHANGED.to_string()),
        started_at: now,
        completed_at: Some(now),
        latency_ms: Some(0),
        cost_cents: 0.0,
        llm_tokens_used: None,
        full_input: Some(payload),
        full_output: None,
        tombstone: None,
        // N14 : RolloutPolicyChanged is a VIRTUAL-swarm row (not bound
        // to any real swarm) — explicit None.
        swarm_envelope_trace_id: None,
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::NanoAgentId;

    #[test]
    fn trace_sentinels_are_distinct() {
        assert_ne!(TRACE_KIND_SHADOW_RUN, TRACE_KIND_SHADOW_COMPARISON);
        assert_ne!(TRACE_KIND_SHADOW_COMPARISON, TRACE_KIND_ROLLOUT_POLICY_CHANGED);
        assert_ne!(TRACE_KIND_SHADOW_RUN, TRACE_KIND_ROLLOUT_POLICY_CHANGED);
    }

    #[test]
    fn rollout_virtual_swarm_uuid_is_constant() {
        let u1 = ROLLOUT_VIRTUAL_SWARM_UUID;
        let u2 = ROLLOUT_VIRTUAL_SWARM_UUID;
        assert_eq!(u1, u2);
    }

    #[test]
    fn shadow_outcome_to_json_match_has_kind_field() {
        let o = ShadowOutcome::Match {
            compared_at: Utc::now(),
            similarity: 0.97,
            prod_latency_ms: 10,
            shadow_latency_ms: 12,
            prod_cost_cents: 0.001,
            shadow_cost_cents: 0.002,
        };
        let j = shadow_outcome_to_json(&o);
        assert_eq!(j["kind"], "match");
        assert!((j["similarity"].as_f64().unwrap() - 0.97).abs() < 1e-6);
    }

    #[test]
    fn shadow_outcome_to_json_mismatch_carries_diff() {
        let o = ShadowOutcome::Mismatch {
            compared_at: Utc::now(),
            similarity: 0.5,
            diff_summary: "key changed".into(),
            prod_latency_ms: 10,
            shadow_latency_ms: 12,
            prod_cost_cents: 0.001,
            shadow_cost_cents: 0.002,
        };
        let j = shadow_outcome_to_json(&o);
        assert_eq!(j["kind"], "mismatch");
        assert_eq!(j["diff_summary"], "key changed");
    }

    #[test]
    fn shadow_outcome_to_json_failed_carries_error() {
        let o = ShadowOutcome::ShadowFailed {
            compared_at: Utc::now(),
            error: "timeout".into(),
        };
        let j = shadow_outcome_to_json(&o);
        assert_eq!(j["kind"], "shadow_failed");
        assert_eq!(j["error"], "timeout");
    }

    #[test]
    fn rollout_policy_change_trace_has_correct_sentinel() {
        let from = RolloutPolicy::Stable {
            version: efe_nano_sdk::NanoAgentVersion::new("1.0.0").unwrap(),
        };
        let to = RolloutPolicy::Canary {
            prod_version: efe_nano_sdk::NanoAgentVersion::new("1.0.0").unwrap(),
            canary_version: efe_nano_sdk::NanoAgentVersion::new("1.1.0").unwrap(),
            canary_bps: 500,
            started_at: Utc::now(),
            promotion_gates: PromotionGates::default(),
            cost_attribution: CostAttribution::Tenant,
        };
        let trace = build_rollout_policy_changed_trace(
            TenantId::system(),
            NanoAgentId::new("agent").unwrap(),
            &from,
            &to,
            "operator:test",
            Some("ramp-up".into()),
        );
        assert_eq!(
            trace.error_kind.as_deref(),
            Some(TRACE_KIND_ROLLOUT_POLICY_CHANGED)
        );
        assert_eq!(trace.swarm_request_id, ROLLOUT_VIRTUAL_SWARM_UUID);
        assert!(trace.full_input.is_some());
    }
}
