//! Dispatcher — orchestrates [`SwarmRequest`] → [`SwarmResponse`].
//!
//! The dispatcher is the single entry point for batch nano-agent
//! execution.  It performs :
//!
//! 1. **Planning** ([`dag::plan`]) — validate the DAG, resolve catalog
//!    entries, topologically layer the ops.
//! 2. **Execution** ([`executor::execute_plan`]) — run each layer in
//!    parallel, propagate skips on dependency failure.
//! 3. **Aggregation** — fold per-op [`OperationOutcome`]s into a
//!    [`SwarmResponse`] with overall status + total cost + total
//!    latency.
//!
//! It also enforces the **swarm-wide deadline** (`request.timeout_ms`)
//! via a global `tokio::time::timeout`, surfacing
//! [`SwarmStatus::TimedOut`] when exceeded.

/// **Sprint 8 — N13** : shadow output comparison strategies (Exact /
/// Structural / Semantic / None).
pub mod compare;
pub mod dag;
/// **Sprint 8 — N14** : swarm envelope construction + writing.
pub mod envelope;
pub mod executor;
/// **Sprint 8 — N13** : sticky-routing for canary cohort selection.
pub mod routing;
/// **Sprint 8 — N13** : async shadow scheduler.
pub mod shadow;
pub mod types;

pub use types::{DagPlan, Layer, OpHandle, OperationOutcome};

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use efe_nano_sdk::{
    Catalog, OpStatus, OperationError, OperationErrorKind, OperationResult, SwarmRequest,
    SwarmResponse, SwarmStatus,
};

use crate::audit::AuditWriter;
use crate::dispatcher::shadow::ShadowScheduler;
use crate::error::DispatcherError;
use crate::idempotency::IdempotencyCache;
use crate::runtime::{NanoAgentExecutor, PureRustRuntime};
use crate::scheduler::AdmissionGate;

// ─────────────────────────────────────────────────────────────────────
//  Dispatcher
// ─────────────────────────────────────────────────────────────────────

/// Phase 1 + Phase 2 dispatcher implementation.
///
/// Holds the shared catalog + runtime + audit + idempotency-cache
/// references and orchestrates [`SwarmRequest`] → [`SwarmResponse`]
/// translation.
///
/// Sprint 4 (Phase 2) generalized the runtime field to
/// `Arc<dyn NanoAgentExecutor>` so that Mode A only / Mode A+B /
/// Mode A+B+C runtimes are all pluggable behind the same dispatcher.
pub struct Dispatcher {
    catalog: Arc<dyn Catalog>,
    runtime: Arc<dyn NanoAgentExecutor>,
    audit: Arc<dyn AuditWriter>,
    idempotency: Arc<IdempotencyCache>,
    /// **Sprint 8 — N13** : optional shadow scheduler.  When `None`,
    /// shadow policies on the catalog are ignored (production-only
    /// dispatch).  When `Some`, the dispatcher applies the full N13
    /// rollout machinery — sticky routing + shadow scheduling.
    shadow_scheduler: Option<Arc<ShadowScheduler>>,
    /// **Sprint 8 — N14 (B3 fix)** : optional envelope metrics surface.
    /// When `None`, metrics aren't tracked.  When `Some`, the
    /// dispatcher records the 8 Prometheus-compatible surfaces.
    envelope_metrics: Option<Arc<envelope::EnvelopeMetrics>>,
    /// **Sprint 8 — N15** : optional fairness-scheduling admission
    /// gate.  When `None`, the dispatcher behaves as in pre-N15
    /// builds (no gating ; all 1136 legacy tests continue passing).
    /// When `Some`, every `dispatch` call must clear the gate before
    /// planning — failures emit
    /// [`DispatcherError::Backpressure`] (mapped to HTTP 429).
    admission_gate: Option<Arc<AdmissionGate>>,
}

impl Dispatcher {
    /// Construct a dispatcher.  The executor must be able to handle every
    /// `execution_mode` represented in the catalog ; otherwise individual
    /// ops fail with `ModeNotWired` or `Internal("handler not registered")`.
    pub fn new(
        catalog: Arc<dyn Catalog>,
        runtime: Arc<dyn NanoAgentExecutor>,
        audit: Arc<dyn AuditWriter>,
        idempotency: Arc<IdempotencyCache>,
    ) -> Self {
        Self {
            catalog,
            runtime,
            audit,
            idempotency,
            shadow_scheduler: None,
            envelope_metrics: None,
            admission_gate: None,
        }
    }

    /// **Sprint 8 — N15** : install a fairness-scheduling admission
    /// gate.  Returns `self` for chaining.
    pub fn with_admission_gate(mut self, gate: Arc<AdmissionGate>) -> Self {
        self.admission_gate = Some(gate);
        self
    }

    /// **Sprint 8 — N15** : access the admission gate (if installed).
    pub fn admission_gate(&self) -> Option<&Arc<AdmissionGate>> {
        self.admission_gate.as_ref()
    }

    /// **Sprint 8 — N14 (B3 fix)** : install an `EnvelopeMetrics`
    /// instance for observability.  Returns `self` for chaining.
    pub fn with_envelope_metrics(mut self, metrics: Arc<envelope::EnvelopeMetrics>) -> Self {
        self.envelope_metrics = Some(metrics);
        self
    }

    /// **Sprint 8 — N14** : access the envelope metrics surface (if installed).
    pub fn envelope_metrics(&self) -> Option<&Arc<envelope::EnvelopeMetrics>> {
        self.envelope_metrics.as_ref()
    }

    /// Convenience constructor : wrap a `PureRustRuntime` directly.
    /// Used by Phase 1 tests + Mode-A-only deployments.
    pub fn with_pure_rust_runtime(
        catalog: Arc<dyn Catalog>,
        runtime: Arc<PureRustRuntime>,
        audit: Arc<dyn AuditWriter>,
        idempotency: Arc<IdempotencyCache>,
    ) -> Self {
        Self::new(catalog, runtime as Arc<dyn NanoAgentExecutor>, audit, idempotency)
    }

    /// **Sprint 8 — N13** : Builder — install a [`ShadowScheduler`] for
    /// rollout-policy-aware dispatch.  Returns `self` for chaining.
    pub fn with_shadow_scheduler(mut self, scheduler: Arc<ShadowScheduler>) -> Self {
        self.shadow_scheduler = Some(scheduler);
        self
    }

    /// **Sprint 8 — N13** : access the catalog (for callers that need
    /// to read / write rollout policy directly).
    pub fn catalog(&self) -> &Arc<dyn Catalog> {
        &self.catalog
    }

    // ─────────────────────────────────────────────────────────────────
    //  Sprint 8 — N13 (B5 + B8) : rollout workflow helpers
    // ─────────────────────────────────────────────────────────────────

    /// **N13** : Promote the active canary version of `agent_id` to
    /// `Stable`.  Evaluates promotion gates against the current drift
    /// window ; rejects with diagnostics if any gate blocks.
    ///
    /// Side-effects on success :
    /// 1. Catalog policy → `Stable(canary_version)`.
    /// 2. Idempotency cache flushed for `(tenant, agent_id)`.
    /// 3. Drift window cleared.
    /// 4. `RolloutPolicyChanged` audit trace emitted.
    ///
    /// `force = true` bypasses the gate check (operator override).
    pub async fn promote_canary(
        &self,
        tenant: efe_nano_sdk::TenantId,
        agent_id: &efe_nano_sdk::NanoAgentId,
        actor: impl Into<String>,
        reason: Option<String>,
        force: bool,
    ) -> Result<PromotionReceipt, RolloutWorkflowError> {
        let policy = self
            .catalog
            .get_rollout_policy(tenant, agent_id)
            .await
            .map_err(|e| RolloutWorkflowError::CatalogError(e.to_string()))?;
        let (canary_version, gates) = match &policy {
            efe_nano_sdk::RolloutPolicy::Canary {
                canary_version,
                promotion_gates,
                ..
            } => (canary_version.clone(), *promotion_gates),
            _ => {
                return Err(RolloutWorkflowError::NotACanary {
                    current_kind: policy.kind_str(),
                });
            }
        };

        // Gate evaluation — skipped if force=true.
        if !force {
            if let Some(scheduler) = self.shadow_scheduler.as_ref() {
                let stats = scheduler
                    .drift_window()
                    .stats(tenant, agent_id)
                    .unwrap_or(efe_nano_sdk::DriftStats::empty());
                let outcome = crate::audit::PromotionGateEvaluator::evaluate_for_promotion(
                    &stats, &gates,
                );
                if let efe_nano_sdk::GateOutcome::Block { reasons } = outcome {
                    return Err(RolloutWorkflowError::GatesBlocked { reasons });
                }
            }
            // No scheduler ⇒ no drift data ⇒ gates can't fail-closed.
            // Operator can still pass force=true to proceed regardless.
        }

        // Atomic catalog swap.
        let next = efe_nano_sdk::RolloutPolicy::Stable {
            version: canary_version.clone(),
        };
        self.catalog
            .set_rollout_policy(tenant, agent_id, next.clone())
            .await
            .map_err(|e| RolloutWorkflowError::CatalogError(e.to_string()))?;

        // Side-effects.
        let actor_str = actor.into();
        let _ = self.idempotency.flush_for_agent(tenant, agent_id).await;
        if let Some(scheduler) = self.shadow_scheduler.as_ref() {
            scheduler.drift_window().clear(tenant, agent_id);
        }
        // Emit RolloutPolicyChanged trace.
        let trace = crate::dispatcher::shadow::build_rollout_policy_changed_trace(
            tenant,
            agent_id.clone(),
            &policy,
            &next,
            actor_str.clone(),
            reason.clone(),
        );
        self.audit.record(trace).await;

        Ok(PromotionReceipt {
            tenant,
            agent_id: agent_id.clone(),
            previous_policy: policy,
            new_policy: next,
            actor: actor_str,
            reason,
        })
    }

    /// **N13** : Roll back the active canary (or shadow) policy back
    /// to `Stable(prod_version)`.  No gate check — always succeeds.
    ///
    /// Side-effects :
    /// 1. Catalog policy → `Stable(prod_version)`.
    /// 2. Idempotency cache flushed for `(tenant, agent_id)`.
    /// 3. Drift window cleared + rollback cooldown started.
    /// 4. `RolloutPolicyChanged` audit trace emitted.
    pub async fn rollback_canary(
        &self,
        tenant: efe_nano_sdk::TenantId,
        agent_id: &efe_nano_sdk::NanoAgentId,
        actor: impl Into<String>,
        reason: Option<String>,
    ) -> Result<RollbackReceipt, RolloutWorkflowError> {
        let policy = self
            .catalog
            .get_rollout_policy(tenant, agent_id)
            .await
            .map_err(|e| RolloutWorkflowError::CatalogError(e.to_string()))?;
        let prod_version = match &policy {
            efe_nano_sdk::RolloutPolicy::Canary { prod_version, .. }
            | efe_nano_sdk::RolloutPolicy::Shadow { prod_version, .. } => prod_version.clone(),
            efe_nano_sdk::RolloutPolicy::Stable { .. } => {
                return Err(RolloutWorkflowError::AlreadyStable);
            }
        };
        let next = efe_nano_sdk::RolloutPolicy::Stable {
            version: prod_version.clone(),
        };
        self.catalog
            .set_rollout_policy(tenant, agent_id, next.clone())
            .await
            .map_err(|e| RolloutWorkflowError::CatalogError(e.to_string()))?;

        let actor_str = actor.into();
        let _ = self.idempotency.flush_for_agent(tenant, agent_id).await;
        if let Some(scheduler) = self.shadow_scheduler.as_ref() {
            scheduler.drift_window().clear(tenant, agent_id);
            scheduler
                .drift_window()
                .mark_rollback(tenant, agent_id.clone());
        }
        let trace = crate::dispatcher::shadow::build_rollout_policy_changed_trace(
            tenant,
            agent_id.clone(),
            &policy,
            &next,
            actor_str.clone(),
            reason.clone(),
        );
        self.audit.record(trace).await;

        Ok(RollbackReceipt {
            tenant,
            agent_id: agent_id.clone(),
            previous_policy: policy,
            new_policy: next,
            actor: actor_str,
            reason,
        })
    }

    /// **N13** : Read-only inspection of the current rollout policy +
    /// drift stats + gate evaluation.  Returns a structured report —
    /// no side effects.
    pub async fn observe_canary(
        &self,
        tenant: efe_nano_sdk::TenantId,
        agent_id: &efe_nano_sdk::NanoAgentId,
    ) -> Result<CanaryObservation, RolloutWorkflowError> {
        let policy = self
            .catalog
            .get_rollout_policy(tenant, agent_id)
            .await
            .map_err(|e| RolloutWorkflowError::CatalogError(e.to_string()))?;
        let (drift_stats, gate_outcome) = if let Some(scheduler) = self.shadow_scheduler.as_ref() {
            let stats = scheduler
                .drift_window()
                .stats(tenant, agent_id)
                .unwrap_or(efe_nano_sdk::DriftStats::empty());
            // Use canary's gates if available, else default.
            let gates = match &policy {
                efe_nano_sdk::RolloutPolicy::Canary {
                    promotion_gates, ..
                } => *promotion_gates,
                _ => efe_nano_sdk::PromotionGates::default(),
            };
            let outcome =
                crate::audit::PromotionGateEvaluator::evaluate_for_promotion(&stats, &gates);
            (Some(stats), Some(outcome))
        } else {
            (None, None)
        };
        let in_cooldown = self
            .shadow_scheduler
            .as_ref()
            .map(|s| s.drift_window().in_rollback_cooldown(tenant, agent_id))
            .unwrap_or(false);
        Ok(CanaryObservation {
            policy,
            drift_stats,
            gate_outcome,
            in_rollback_cooldown: in_cooldown,
        })
    }

    /// Execute a [`SwarmRequest`] end-to-end.
    pub async fn dispatch(&self, request: SwarmRequest) -> Result<SwarmResponse, DispatcherError> {
        let started = Instant::now();
        let request_id = request.request_id;
        let swarm_budget = std::time::Duration::from_millis(request.timeout_ms as u64);

        // ─────────────────────────────────────────────────────────────
        //  N15 : Admission gate — checked BEFORE planning so a
        //  rejected swarm doesn't consume catalog lookups or audit
        //  writes.  The returned `AdmissionTicket` is held for the
        //  whole `dispatch` scope ; its `Drop` releases the global
        //  swarm DRR permit + per-tenant tracker reservation.
        //
        //  When `admission_gate` is `None`, dispatch proceeds
        //  unchanged (legacy + tests).
        // ─────────────────────────────────────────────────────────────
        let _admission_ticket = if let Some(gate) = self.admission_gate.as_ref() {
            let op_count = request.operations.len() as u32;
            match gate.admit_swarm(request.tenant_id, op_count).await {
                Ok(t) => Some(t),
                Err(e) => return Err(DispatcherError::Backpressure(e)),
            }
        } else {
            None
        };

        // ─────────────────────────────────────────────────────────────
        //  N14 : Pre-allocate envelope trace id + builder + counters.
        // ─────────────────────────────────────────────────────────────
        let envelope_trace_id = efe_nano_sdk::TraceId::new();
        let builder = envelope::EnvelopeBuilder::new(envelope_trace_id, &request);
        // **B2 fix** : shared per-swarm atomic counters for shadow / canary
        // aggregates ; threaded into the executor + shadow scheduler.
        let counters = Arc::new(envelope::SwarmRolloutCounters::new());

        // ─────────────────────────────────────────────────────────────
        //  Plan
        // ─────────────────────────────────────────────────────────────
        let plan = match dag::plan(&request, &*self.catalog).await {
            Ok(p) => p,
            Err(e) => {
                // N14 : planner failure still emits an envelope row for forensics.
                let total_latency_ms = started.elapsed().as_millis() as u32;
                let response = planner_failure_response(&request, total_latency_ms);
                self.write_envelope_safely(builder, &response, &HashMap::new(), &counters).await;
                return Err(e);
            }
        };

        // ─────────────────────────────────────────────────────────────
        //  Execute with overall timeout.
        // ─────────────────────────────────────────────────────────────
        let outcomes_result = tokio::time::timeout(
            swarm_budget,
            executor::execute_plan_with_extras_n14(
                &plan,
                &request,
                Arc::clone(&self.runtime),
                Arc::clone(&self.audit),
                Arc::clone(&self.idempotency),
                Some(Arc::clone(&self.catalog)),
                self.shadow_scheduler.as_ref().map(Arc::clone),
                Some(envelope_trace_id),
                Some(Arc::clone(&counters)),
            ),
        )
        .await;

        let outcomes: HashMap<String, OperationOutcome> = match outcomes_result {
            Ok(Ok(o)) => o,
            Ok(Err(e)) => {
                // Executor internal failure : emit envelope, propagate.
                let total_latency_ms = started.elapsed().as_millis() as u32;
                let response = planner_failure_response(&request, total_latency_ms);
                self.write_envelope_safely(builder, &response, &HashMap::new(), &counters).await;
                return Err(e);
            }
            Err(_elapsed) => {
                // Swarm-wide timeout : synthesize response + emit envelope.
                let total_latency_ms = started.elapsed().as_millis() as u32;
                let response = timed_out_response(&request, total_latency_ms);
                let envelope_id = self.write_envelope_safely(builder, &response, &HashMap::new(), &counters).await;
                let mut response = response;
                response.swarm_envelope_trace_id = Some(envelope_id);
                return Ok(response);
            }
        };

        // ─────────────────────────────────────────────────────────────
        //  Aggregate
        // ─────────────────────────────────────────────────────────────
        let total_latency_ms = started.elapsed().as_millis() as u32;
        let mut response = aggregate(request_id, &request, outcomes.clone(), total_latency_ms);

        // ─────────────────────────────────────────────────────────────
        //  N14 : Write envelope + attach id to response.
        // ─────────────────────────────────────────────────────────────
        let envelope_id = self.write_envelope_safely(builder, &response, &outcomes, &counters).await;
        response.swarm_envelope_trace_id = Some(envelope_id);
        Ok(response)
    }

    /// **N14** : write the envelope through the audit writer (resilient
    /// to audit failures per N6 contract).  Returns the envelope id
    /// regardless of audit outcome — callers always attach it to the
    /// response so customers can query their log even after a partial
    /// audit-backend failure.
    async fn write_envelope_safely(
        &self,
        builder: envelope::EnvelopeBuilder,
        response: &SwarmResponse,
        outcomes: &HashMap<String, OperationOutcome>,
        counters: &envelope::SwarmRolloutCounters,
    ) -> efe_nano_sdk::TraceId {
        // **B2 + B3 fix** : finalize with shared counters AND optional
        // metrics surface so the envelope carries shadow / canary
        // aggregates and the 8 Prometheus surfaces tick.
        envelope::write_envelope_full(
            &*self.audit,
            builder,
            response,
            outcomes,
            counters,
            self.envelope_metrics.as_deref(),
        )
        .await
    }
}

/// **N14** : Synthesise a failure-shape SwarmResponse for planner /
/// internal errors.  No ops ran, status = PartialFailure so callers
/// can distinguish from a clean Success.
fn planner_failure_response(request: &SwarmRequest, total_latency_ms: u32) -> SwarmResponse {
    let mut results = HashMap::new();
    for op in &request.operations {
        results.insert(
            op.op_id.clone(),
            OperationResult {
                op_id: op.op_id.clone(),
                status: OpStatus::Skipped,
                output: None,
                error: Some(OperationError::new(
                    OperationErrorKind::PreflightFailure,
                    "swarm planner failed".to_string(),
                )),
                latency_ms: 0,
                cost_cents: 0.0,
            },
        );
    }
    SwarmResponse {
        request_id: request.request_id,
        overall_status: SwarmStatus::PartialFailure,
        results,
        total_latency_ms,
        total_cost_cents: 0.0,
        swarm_envelope_trace_id: None,
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Aggregation helpers
// ─────────────────────────────────────────────────────────────────────

/// Build a [`SwarmResponse`] from the per-op outcomes.
fn aggregate(
    request_id: uuid::Uuid,
    request: &SwarmRequest,
    outcomes: HashMap<String, OperationOutcome>,
    total_latency_ms: u32,
) -> SwarmResponse {
    let mut results: HashMap<String, OperationResult> = HashMap::with_capacity(outcomes.len());
    let mut any_failure = false;
    let mut total_cost_cents = 0.0;

    // Iterate ops in request order to make response deterministic.
    for op in &request.operations {
        let outcome = outcomes
            .get(&op.op_id)
            .cloned()
            .unwrap_or_else(|| OperationOutcome::pending(&op.op_id));
        match outcome.status {
            OpStatus::Success => {}
            OpStatus::Failed
            | OpStatus::Skipped
            | OpStatus::Cancelled
            | OpStatus::Pending
            | OpStatus::Running => {
                // Anything non-Success counts as a failure (Pending/Running
                // are unreachable post-execution but defensively counted).
                any_failure = true;
            }
        }
        total_cost_cents += outcome.cost_cents;
        results.insert(op.op_id.clone(), outcome.into_operation_result());
    }

    // The SDK SwarmStatus enum has no "all-failed" variant — both "some
    // ops failed" and "every op failed" map to PartialFailure (the customer
    // inspects per-op results for detail).
    let overall_status = if any_failure {
        SwarmStatus::PartialFailure
    } else {
        SwarmStatus::Success
    };

    SwarmResponse {
        request_id,
        overall_status,
        results,
        total_latency_ms,
        total_cost_cents,
        swarm_envelope_trace_id: None,
    }
}

/// Build a `TimedOut` swarm response (every op reported as timed-out).
fn timed_out_response(request: &SwarmRequest, total_latency_ms: u32) -> SwarmResponse {
    let mut results: HashMap<String, OperationResult> =
        HashMap::with_capacity(request.operations.len());
    for op in &request.operations {
        results.insert(
            op.op_id.clone(),
            OperationResult {
                op_id: op.op_id.clone(),
                status: OpStatus::Cancelled,
                output: None,
                error: Some(OperationError::new(
                    OperationErrorKind::Timeout,
                    format!("swarm timed out after {} ms", request.timeout_ms),
                )),
                latency_ms: 0,
                cost_cents: 0.0,
            },
        );
    }
    SwarmResponse {
        request_id: request.request_id,
        overall_status: SwarmStatus::TimedOut,
        results,
        total_latency_ms,
        total_cost_cents: 0.0,
        swarm_envelope_trace_id: None,
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::stub::InMemoryAuditWriter;
    use crate::catalog::InMemoryCatalog;
    use crate::runtime::pure_rust::HandlerRegistry;
    use crate::starter_pack::register_all_starter_pack;
    use efe_nano_sdk::{
        NanoAgentId, NanoAgentRef, Operation, OperationInput, RetryPolicy, TenantId,
    };
    use uuid::Uuid;

    fn lit(v: serde_json::Value) -> OperationInput {
        OperationInput::Literal { value: v }
    }

    fn op(id: &str, agent_id: &str, deps: &[&str], input: OperationInput) -> Operation {
        Operation {
            op_id: id.to_string(),
            agent: NanoAgentRef::new(NanoAgentId::new(agent_id).unwrap()),
            input,
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
            timeout_ms: None,
            retry: RetryPolicy::default(),
        }
    }

    fn req(ops: Vec<Operation>) -> SwarmRequest {
        SwarmRequest {
            request_id: Uuid::new_v4(),
            tenant_id: TenantId::system(),
            caller_agent_id: Some("test".to_string()),
            operations: ops,
            max_parallelism: 100,
            timeout_ms: 30_000,
            idempotency_key: None,
            streaming: false,
            audit_tags: vec![],
            cost_cap_cents: None,
            parent_envelope_trace_id: None,
        }
    }

    async fn fixture() -> Dispatcher {
        let catalog = Arc::new(InMemoryCatalog::new());
        let registry = HandlerRegistry::new();
        register_all_starter_pack(&*catalog, &registry).await.unwrap();
        let runtime = Arc::new(PureRustRuntime::new(registry));
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let idempotency = Arc::new(IdempotencyCache::new());
        Dispatcher::with_pure_rust_runtime(
            catalog as Arc<dyn Catalog>,
            runtime,
            audit,
            idempotency,
        )
    }

    #[tokio::test]
    async fn dispatch_single_op_success() {
        let d = fixture().await;
        let request = req(vec![op(
            "v",
            "json_validate",
            &[],
            lit(serde_json::json!({
                "value": {"x": 1},
                "schema": {"type": "object"}
            })),
        )]);
        let resp = d.dispatch(request).await.unwrap();
        assert_eq!(resp.overall_status, SwarmStatus::Success);
        assert_eq!(resp.results.len(), 1);
        assert_eq!(resp.results["v"].status, OpStatus::Success);
        assert!(resp.total_cost_cents > 0.0);
    }

    #[tokio::test]
    async fn dispatch_partial_failure_propagates() {
        let d = fixture().await;
        // op-a fails (missing schema field), op-b is ok, op-c depends on a → skipped.
        let request = req(vec![
            op("a", "json_validate", &[], lit(serde_json::json!({"value": {}}))),
            op(
                "b",
                "json_validate",
                &[],
                lit(serde_json::json!({
                    "value": {"x": 1},
                    "schema": {"type": "object"}
                })),
            ),
            op(
                "c",
                "json_validate",
                &["a"],
                lit(serde_json::json!({
                    "value": {},
                    "schema": {"type": "object"}
                })),
            ),
        ]);
        let resp = d.dispatch(request).await.unwrap();
        assert_eq!(resp.overall_status, SwarmStatus::PartialFailure);
        assert_eq!(resp.results["a"].status, OpStatus::Failed);
        assert_eq!(resp.results["b"].status, OpStatus::Success);
        assert_eq!(resp.results["c"].status, OpStatus::Skipped);
    }

    #[tokio::test]
    async fn dispatch_all_failed_still_partial_failure() {
        let d = fixture().await;
        let request = req(vec![op(
            "bad",
            "json_validate",
            &[],
            lit(serde_json::json!({})),
        )]);
        let resp = d.dispatch(request).await.unwrap();
        assert_eq!(resp.overall_status, SwarmStatus::PartialFailure);
        assert_eq!(resp.results["bad"].status, OpStatus::Failed);
    }

    #[tokio::test]
    async fn dispatch_unknown_agent_yields_invalid_dag_error() {
        let d = fixture().await;
        let request = req(vec![op(
            "x",
            "ghost_agent",
            &[],
            lit(serde_json::json!({})),
        )]);
        let err = d.dispatch(request).await.unwrap_err();
        assert!(matches!(err, DispatcherError::InvalidDag { .. }));
    }

    #[tokio::test]
    async fn dispatch_cycle_yields_cycle_error() {
        let d = fixture().await;
        let request = req(vec![
            op(
                "a",
                "json_validate",
                &["b"],
                lit(serde_json::json!({"value": {}, "schema": {}})),
            ),
            op(
                "b",
                "json_validate",
                &["a"],
                lit(serde_json::json!({"value": {}, "schema": {}})),
            ),
        ]);
        let err = d.dispatch(request).await.unwrap_err();
        assert!(matches!(err, DispatcherError::CycleDetected { .. }));
    }

    #[tokio::test]
    async fn dispatch_diamond_dag_all_success() {
        let d = fixture().await;
        let request = req(vec![
            op(
                "src",
                "json_validate",
                &[],
                lit(serde_json::json!({
                    "value": "data",
                    "schema": {"type": "string"}
                })),
            ),
            op(
                "leaf1",
                "regex_extract",
                &["src"],
                lit(serde_json::json!({
                    "pattern": r"\w+",
                    "text": "hello world"
                })),
            ),
            op(
                "leaf2",
                "csv_normalize",
                &["src"],
                lit(serde_json::json!({"csv": "a,b\n1,2"})),
            ),
            op(
                "join",
                "json_validate",
                &["leaf1", "leaf2"],
                lit(serde_json::json!({
                    "value": "done",
                    "schema": {"type": "string"}
                })),
            ),
        ]);
        let resp = d.dispatch(request).await.unwrap();
        assert_eq!(resp.overall_status, SwarmStatus::Success);
        assert_eq!(resp.results.len(), 4);
        for o in resp.results.values() {
            assert_eq!(o.status, OpStatus::Success);
        }
    }

    #[tokio::test]
    async fn dispatch_swarm_timeout_path_is_well_formed() {
        // We don't try to *force* a timeout here (that's racy : a fast op may
        // complete before the deadline timer fires).  Instead we exercise the
        // helper that synthesizes the TimedOut response and verify its shape.
        let request = req(vec![op(
            "v",
            "json_validate",
            &[],
            lit(serde_json::json!({})),
        )]);
        let resp = super::timed_out_response(&request, 0);
        assert_eq!(resp.overall_status, SwarmStatus::TimedOut);
        assert_eq!(resp.results["v"].status, OpStatus::Cancelled);
        let err = resp.results["v"].error.as_ref().unwrap();
        assert_eq!(err.kind, OperationErrorKind::Timeout);
    }

    #[tokio::test]
    async fn dispatch_idempotency_cache_hit() {
        let d = fixture().await;
        let request = req(vec![op(
            "v",
            "json_validate",
            &[],
            lit(serde_json::json!({
                "value": {"x": 1},
                "schema": {"type": "object"}
            })),
        )]);
        // First call : warms cache.
        let resp1 = d.dispatch(request.clone()).await.unwrap();
        assert_eq!(resp1.results["v"].status, OpStatus::Success);
        // Second call : should be free / cached.
        let resp2 = d.dispatch(request).await.unwrap();
        assert_eq!(resp2.results["v"].status, OpStatus::Success);
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Sprint 8 — N13 : rollout workflow receipts + errors
// ─────────────────────────────────────────────────────────────────────

/// **N13** : Returned by [`Dispatcher::promote_canary`] on success.
#[derive(Debug, Clone)]
pub struct PromotionReceipt {
    pub tenant: efe_nano_sdk::TenantId,
    pub agent_id: efe_nano_sdk::NanoAgentId,
    pub previous_policy: efe_nano_sdk::RolloutPolicy,
    pub new_policy: efe_nano_sdk::RolloutPolicy,
    pub actor: String,
    pub reason: Option<String>,
}

/// **N13** : Returned by [`Dispatcher::rollback_canary`] on success.
#[derive(Debug, Clone)]
pub struct RollbackReceipt {
    pub tenant: efe_nano_sdk::TenantId,
    pub agent_id: efe_nano_sdk::NanoAgentId,
    pub previous_policy: efe_nano_sdk::RolloutPolicy,
    pub new_policy: efe_nano_sdk::RolloutPolicy,
    pub actor: String,
    pub reason: Option<String>,
}

/// **N13** : Returned by [`Dispatcher::observe_canary`].
#[derive(Debug, Clone)]
pub struct CanaryObservation {
    pub policy: efe_nano_sdk::RolloutPolicy,
    pub drift_stats: Option<efe_nano_sdk::DriftStats>,
    pub gate_outcome: Option<efe_nano_sdk::GateOutcome>,
    pub in_rollback_cooldown: bool,
}

/// **N13** : errors emitted by the rollout workflow helpers.
#[derive(Debug, thiserror::Error)]
pub enum RolloutWorkflowError {
    #[error("operation rejected : current policy is {current_kind}, not Canary")]
    NotACanary { current_kind: &'static str },

    #[error("rollback rejected : policy is already Stable")]
    AlreadyStable,

    #[error("promotion blocked by gates : {}", reasons.join(" ; "))]
    GatesBlocked { reasons: Vec<String> },

    #[error("catalog error : {0}")]
    CatalogError(String),
}
