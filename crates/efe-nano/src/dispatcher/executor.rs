//! Parallel layer executor — Sprint 7 redesign.
//!
//! Given a [`DagPlan`], execute every layer in order :
//! - Within a layer : run all ops in parallel via `tokio::JoinSet`,
//!   subject to the swarm's `max_parallelism` cap.
//! - Between layers : block until every op in the current layer is
//!   terminal (Success / Failed / Skipped / Cancelled).
//! - Failed dependencies cause downstream ops to be marked `Skipped`
//!   rather than executed.
//!
//! The executor is `Runtime`-mode-agnostic : it takes a
//! `Arc<dyn NanoAgentExecutor>` and dispatches per-op based on the
//! agent's `execution_mode`.
//!
//! # Sprint 7 — what changed
//!
//! - **N1** : every op pre-allocates its `trace_id` so downstream
//!   `Ref` / `Merge` ops can record the full `parent_trace_ids` vec
//!   in their `NanoAgentTrace`.  Pre-Sprint-7 this was always empty.
//! - **N2** : the `caller_agent_id` from `SwarmRequest` is threaded
//!   into every per-op `CallContext` and ends up in each trace.
//! - **N4** : preflight failures (input resolve error, deadline expired
//!   pre-dispatch, swarm cost cap already breached) emit a "negative
//!   trace" to the audit chain so the chain stays contiguous.  Without
//!   this, an attacker who causes a preflight-class failure could
//!   remove evidence of an attempted op.
//! - **N6** : `input_hash_source` is computed from the **raw**
//!   `OperationInput` AST (Literal / Ref / Merge tree) and passed to
//!   the runtime separately from the resolved input.  The runtime then
//!   overrides the trace's `input_hash_source` to match.  For Literal
//!   inputs source == resolved ; for Ref / Merge they differ.
//! - **N7** : the swarm-wide `cost_cap_cents` is enforced via a shared
//!   `SwarmCostTracker`.  Between layers, the executor checks the
//!   cap ; if breached, every op in subsequent layers is short-circuited
//!   with `OperationErrorKind::CostCapReached` (and audited as N4
//!   negative trace).

use chrono::Utc;
use efe_nano_sdk::{
    Operation, OperationError, OperationErrorKind, OperationInput, OpStatus, SwarmRequest, TraceId,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::audit::{AuditWriter, NanoAgentTrace};
use crate::dispatcher::shadow::{
    ProdOutcomeSnapshot, ShadowDispatchContext, ShadowScheduler,
};
use crate::dispatcher::types::{DagPlan, OpHandle, OperationOutcome};
use crate::error::DispatcherError;
use crate::idempotency::{CacheEntry, IdempotencyCache, LlmCacheMetadata};
use crate::runtime::context::SwarmCostTracker;
use crate::runtime::{CallContext, NanoAgentExecutor};
use efe_nano_sdk::{Catalog, ResolvedAgent};

/// Execute a planned DAG and return the per-op outcomes keyed by op_id.
///
/// The semaphore enforces `request.max_parallelism` across the entire
/// swarm — not just within one layer.  This matches customer
/// expectations : "do not run more than N nano agents at once for me".
///
/// **Sprint 7** : `caller_agent_id` and `cost_cap_cents` from the
/// `SwarmRequest` are now threaded through every per-op call.
pub async fn execute_plan(
    plan: &DagPlan,
    request: &SwarmRequest,
    runtime: Arc<dyn NanoAgentExecutor>,
    audit: Arc<dyn AuditWriter>,
    idempotency: Arc<IdempotencyCache>,
) -> Result<HashMap<String, OperationOutcome>, DispatcherError> {
    execute_plan_with_extras(plan, request, runtime, audit, idempotency, None, None).await
}

/// **Sprint 8 — N13** : Extended dispatch with rollout-policy
/// resolution and shadow scheduling.  When `catalog` is `Some`, the
/// planner's resolved `handle.agent` is re-resolved per op against the
/// rollout policy.  When `shadow_scheduler` is `Some`, ShadowPair
/// targets spawn shadow runs.
#[allow(clippy::too_many_arguments)]
pub async fn execute_plan_with_extras(
    plan: &DagPlan,
    request: &SwarmRequest,
    runtime: Arc<dyn NanoAgentExecutor>,
    audit: Arc<dyn AuditWriter>,
    idempotency: Arc<IdempotencyCache>,
    catalog: Option<Arc<dyn Catalog>>,
    shadow_scheduler: Option<Arc<ShadowScheduler>>,
) -> Result<HashMap<String, OperationOutcome>, DispatcherError> {
    execute_plan_with_extras_n14(
        plan,
        request,
        runtime,
        audit,
        idempotency,
        catalog,
        shadow_scheduler,
        None,
        None,
    )
    .await
}

/// **Sprint 8 — N14** : Extended dispatch with rollout + envelope id
/// threading.  When `envelope_trace_id` is `Some`, every op trace
/// (incl. cache-hit / shadow / comparison) emitted by this plan
/// carries it via `CallContext.swarm_envelope_trace_id`.
///
/// **B2 fix** : `rollout_counters` accumulates shadow / canary events
/// for the envelope's aggregate fields.
#[allow(clippy::too_many_arguments)]
pub async fn execute_plan_with_extras_n14(
    plan: &DagPlan,
    request: &SwarmRequest,
    runtime: Arc<dyn NanoAgentExecutor>,
    audit: Arc<dyn AuditWriter>,
    idempotency: Arc<IdempotencyCache>,
    catalog: Option<Arc<dyn Catalog>>,
    shadow_scheduler: Option<Arc<ShadowScheduler>>,
    envelope_trace_id: Option<TraceId>,
    rollout_counters: Option<Arc<crate::dispatcher::envelope::SwarmRolloutCounters>>,
) -> Result<HashMap<String, OperationOutcome>, DispatcherError> {
    let started = Instant::now();
    let max_parallel = (request.max_parallelism as usize).max(1);
    let semaphore = Arc::new(Semaphore::new(max_parallel));

    // Sprint 7 — N7 : per-swarm cost tracker.
    let cost_tracker = Arc::new(SwarmCostTracker::new(request.cost_cap_cents));

    // Sprint 7 — N2 : caller agent id propagated to every op.
    let caller_agent_id = request.caller_agent_id.clone();

    let mut outcomes: HashMap<String, OperationOutcome> = HashMap::with_capacity(plan.total_ops);
    // Sprint 7 — N1 : op_id → trace_id map, populated as ops are
    // pre-allocated.  Layer K+1 consults this to compute
    // parent_trace_ids when resolving Ref / Merge.
    let mut op_id_to_trace_id: HashMap<String, TraceId> = HashMap::with_capacity(plan.total_ops);

    // Seed every op as Pending so dependents can find them.
    for h in plan.iter_ops() {
        outcomes.insert(h.op_id().to_string(), OperationOutcome::pending(h.op_id()));
    }

    // ─────────────────────────────────────────────────────────────────
    //  Walk layer-by-layer
    // ─────────────────────────────────────────────────────────────────
    for layer in &plan.layers {
        // Sprint 7 — N7 : if the swarm cost cap was breached at the end
        // of the previous layer, every remaining op is short-circuited
        // and audited as a "preflight CostCapReached" negative trace.
        if cost_tracker.cap_reached() {
            short_circuit_remaining(
                &layer.ops,
                request,
                &caller_agent_id,
                &mut outcomes,
                &audit,
                OperationErrorKind::CostCapReached,
                &format!(
                    "swarm cost cap {:.4} cents reached (current total {:.4})",
                    cost_tracker.cap_cents().unwrap_or(f64::INFINITY),
                    cost_tracker.total_cents(),
                ),
            )
            .await;
            continue;
        }

        // First : mark any op as Skipped if any of its upstream deps
        // are already in a non-Success terminal state.
        let mut runnable: Vec<&OpHandle> = Vec::with_capacity(layer.ops.len());
        for handle in &layer.ops {
            let failed_dep = first_failed_dep(&handle.op, &outcomes);
            if let Some(dep) = failed_dep {
                let outcome = OperationOutcome::skipped_due_to_dependency(handle.op_id(), &dep);
                outcomes.insert(handle.op_id().to_string(), outcome);
            } else {
                runnable.push(handle);
            }
        }

        if runnable.is_empty() {
            continue;
        }

        // Sprint 7 — N6 : compute input_hash_source from the raw
        // OperationInput AST BEFORE resolving Ref / Merge.  This is the
        // hash that goes into the trace's `input_hash_source` field.
        // For Literal inputs this hash equals the resolved hash.
        let mut per_op_metadata: HashMap<String, OpPreflightMeta> =
            HashMap::with_capacity(runnable.len());
        for handle in &runnable {
            let trace_id = TraceId::new();
            op_id_to_trace_id.insert(handle.op_id().to_string(), trace_id);
            per_op_metadata.insert(
                handle.op_id().to_string(),
                OpPreflightMeta {
                    trace_id,
                    input_hash_source: hash_operation_input(&handle.op.input),
                    parent_trace_ids: collect_parent_trace_ids(&handle.op, &op_id_to_trace_id),
                },
            );
        }

        // Build the per-op inputs by resolving Refs / Merges against
        // already-completed outcomes.  If resolution fails for an op,
        // we surface a preflight negative trace (N4).
        let mut resolved_inputs: HashMap<String, serde_json::Value> =
            HashMap::with_capacity(runnable.len());
        let mut preflight_failures: Vec<(String, String)> = Vec::new();

        for handle in &runnable {
            match resolve_input(&handle.op.input, &outcomes) {
                Ok(v) => {
                    resolved_inputs.insert(handle.op_id().to_string(), v);
                }
                Err(e) => {
                    preflight_failures
                        .push((handle.op_id().to_string(), format!("input resolution : {e}")));
                }
            }
        }

        // Sprint 7 — N4 : emit preflight negative traces for resolution
        // failures BEFORE spawning anything else.
        for (op_id, reason) in &preflight_failures {
            let handle = runnable.iter().find(|h| h.op_id() == op_id).expect("present");
            let meta = per_op_metadata.get(op_id).expect("present");
            emit_preflight_negative_trace(
                handle,
                meta,
                request,
                caller_agent_id.as_deref(),
                OperationErrorKind::PreflightFailure,
                reason,
                Arc::clone(&audit),
            )
            .await;
            let mut out = OperationOutcome::pending(op_id);
            out.status = OpStatus::Failed;
            out.trace_id = Some(meta.trace_id);
            out.error = Some(OperationError::new(
                OperationErrorKind::PreflightFailure,
                reason.clone(),
            ));
            outcomes.insert(op_id.clone(), out);
        }

        // ─────────────────────────────────────────────────────────────
        //  Run this layer in parallel.
        // ─────────────────────────────────────────────────────────────
        let mut joinset: tokio::task::JoinSet<OperationOutcome> = tokio::task::JoinSet::new();
        for handle in &runnable {
            let op_id = handle.op_id().to_string();
            if preflight_failures.iter().any(|(id, _)| id == &op_id) {
                continue;
            }
            let Some(input) = resolved_inputs.get(&op_id).cloned() else {
                continue;
            };
            let meta = per_op_metadata
                .get(&op_id)
                .expect("metadata present for runnable op")
                .clone();

            let handle_clone: OpHandle = (*handle).clone();
            let semaphore = Arc::clone(&semaphore);
            let runtime = Arc::clone(&runtime);
            let audit = Arc::clone(&audit);
            let idempotency = Arc::clone(&idempotency);
            let swarm_request_id = request.request_id;
            let tenant_id = request.tenant_id;
            let caller_agent_id_owned = caller_agent_id.clone();
            let cost_tracker_owned = Arc::clone(&cost_tracker);
            // **Sprint 8 — N13** : per-task clones of the rollout extras.
            let catalog_for_task = catalog.as_ref().map(Arc::clone);
            let shadow_for_task = shadow_scheduler.as_ref().map(Arc::clone);
            // **Sprint 8 — N14** : envelope id threaded to each op task.
            let envelope_id_for_task = envelope_trace_id;
            // **B2 fix** : per-task clone of the shared rollout counters.
            let counters_for_task = rollout_counters.as_ref().map(Arc::clone);

            joinset.spawn(async move {
                // Acquire global parallelism slot.
                let _permit = semaphore
                    .acquire_owned()
                    .await
                    .expect("semaphore is never closed");
                run_one_op(
                    handle_clone,
                    input,
                    runtime,
                    audit,
                    idempotency,
                    swarm_request_id,
                    tenant_id,
                    meta,
                    caller_agent_id_owned,
                    cost_tracker_owned,
                    catalog_for_task,
                    shadow_for_task,
                    envelope_id_for_task,
                    counters_for_task,
                )
                .await
            });
        }

        // Drain the JoinSet, collecting outcomes.
        while let Some(joined) = joinset.join_next().await {
            match joined {
                Ok(outcome) => {
                    outcomes.insert(outcome.op_id.clone(), outcome);
                }
                Err(join_err) => {
                    // Task panic — record an internal error against an unknown op.
                    tracing::error!(
                        target: "efe_nano::dispatcher",
                        error = %join_err,
                        "JoinSet task failed unexpectedly"
                    );
                }
            }
        }
    }

    tracing::debug!(
        target: "efe_nano::dispatcher",
        elapsed_ms = started.elapsed().as_millis() as u64,
        layers = plan.depth(),
        total_ops = plan.total_ops,
        swarm_cost_cents = cost_tracker.total_cents(),
        "swarm execution completed"
    );

    Ok(outcomes)
}

// ─────────────────────────────────────────────────────────────────────
//  Sprint 7 helpers
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 7 — N6** : per-op preflight metadata computed once before
/// the layer fires.  Carried into `run_one_op` so the runtime trace
/// can be patched with the correct source-hash + parents.
#[derive(Clone)]
struct OpPreflightMeta {
    trace_id: TraceId,
    input_hash_source: [u8; 32],
    parent_trace_ids: Vec<TraceId>,
}

/// **Sprint 7 — N6** : SHA-256 of the canonical-serialized
/// `OperationInput` AST.  This is the "what the caller sent" hash —
/// captured BEFORE Ref / Merge resolution.  For `Literal { value }`
/// inputs, this equals the resolved-value hash ; for `Ref` / `Merge`
/// trees the two diverge.
fn hash_operation_input(input: &OperationInput) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    // Serialize the AST as canonical JSON via serde.  The variant tag
    // (Literal / Ref / Merge) is part of the serde representation since
    // OperationInput uses `#[serde(tag = "kind", rename_all = "snake_case")]`,
    // so different kinds with same payload bytes hash differently.
    let value = serde_json::to_value(input).unwrap_or(serde_json::Value::Null);
    let canonical = crate::idempotency::canonicalize_json(&value);
    let digest = Sha256::digest(canonical.as_bytes());
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

/// **Sprint 7 — N1** : walk the `OperationInput` AST and collect every
/// upstream `Ref { op_id }` (transitively through `Merge` sources),
/// mapping each to its pre-allocated `trace_id`.  Returns a deduped,
/// sorted-by-op-id vec (sorted for canonical-row-body stability).
fn collect_parent_trace_ids(
    op: &Operation,
    op_id_to_trace_id: &HashMap<String, TraceId>,
) -> Vec<TraceId> {
    let mut acc: Vec<(String, TraceId)> = Vec::new();
    walk_refs(&op.input, op_id_to_trace_id, &mut acc);
    // Also include declared depends_on edges that don't appear as Refs
    // (allowed by the SDK : an op can depend on another for sequencing
    // without consuming its output).
    for dep in &op.depends_on {
        if let Some(tid) = op_id_to_trace_id.get(dep) {
            if !acc.iter().any(|(id, _)| id == dep) {
                acc.push((dep.clone(), *tid));
            }
        }
    }
    acc.sort_by(|a, b| a.0.cmp(&b.0));
    acc.into_iter().map(|(_, t)| t).collect()
}

fn walk_refs(
    input: &OperationInput,
    op_id_to_trace_id: &HashMap<String, TraceId>,
    acc: &mut Vec<(String, TraceId)>,
) {
    match input {
        OperationInput::Literal { .. } => {}
        OperationInput::Ref { op_id, .. } => {
            if let Some(tid) = op_id_to_trace_id.get(op_id) {
                if !acc.iter().any(|(id, _)| id == op_id) {
                    acc.push((op_id.clone(), *tid));
                }
            }
        }
        OperationInput::Merge { sources } => {
            for src in sources {
                walk_refs(src, op_id_to_trace_id, acc);
            }
        }
    }
}

/// **Sprint 7 — N4** : emit a negative trace to the audit chain for
/// an op that failed before reaching the runtime.  The chain stays
/// contiguous even when attackers try to suppress evidence.
#[allow(clippy::too_many_arguments)]
async fn emit_preflight_negative_trace(
    handle: &OpHandle,
    meta: &OpPreflightMeta,
    request: &SwarmRequest,
    caller_agent_id: Option<&str>,
    error_kind: OperationErrorKind,
    reason: &str,
    audit: Arc<dyn AuditWriter>,
) {
    let trace = NanoAgentTrace {
        trace_id: meta.trace_id,
        parent_trace_ids: meta.parent_trace_ids.clone(),
        swarm_request_id: request.request_id,
        operation_id: handle.op_id().to_string(),
        tenant_id: request.tenant_id,
        caller_agent_id: caller_agent_id.map(String::from),
        nano_agent_id: handle.agent.id.clone(),
        nano_agent_version: handle.agent.version.clone(),
        execution_mode: handle.agent.execution_mode.mode_str().to_string(),
        // Source AST is known ; resolved value is empty / not produced.
        input_hash_source: meta.input_hash_source,
        // Sprint 7 — N4 : there is NO resolved input for a preflight
        // failure (resolution failed or didn't run).  We commit to
        // all-zero bytes which is a valid SHA-256 codomain element AND
        // serves as a sentinel for the auditor ("input never resolved").
        input_hash_resolved: [0u8; 32],
        output_hash: None,
        error_kind: Some(format!("{error_kind:?}: {reason}")),
        started_at: Utc::now(),
        completed_at: Some(Utc::now()),
        latency_ms: Some(0),
        cost_cents: 0.0,
        llm_tokens_used: None,
        full_input: None,
        full_output: None,
        tombstone: None,
        swarm_envelope_trace_id: None,
    };
    audit.record(trace).await;
}

/// **Sprint 7 — N7** : when the swarm cost cap is breached, every op
/// in subsequent layers gets a preflight-CostCapReached trace plus a
/// Skipped outcome.
#[allow(clippy::too_many_arguments)]
async fn short_circuit_remaining(
    layer_ops: &[OpHandle],
    request: &SwarmRequest,
    caller_agent_id: &Option<String>,
    outcomes: &mut HashMap<String, OperationOutcome>,
    audit: &Arc<dyn AuditWriter>,
    error_kind: OperationErrorKind,
    reason: &str,
) {
    for handle in layer_ops {
        let trace_id = TraceId::new();
        let meta = OpPreflightMeta {
            trace_id,
            input_hash_source: hash_operation_input(&handle.op.input),
            parent_trace_ids: vec![],
        };
        emit_preflight_negative_trace(
            handle,
            &meta,
            request,
            caller_agent_id.as_deref(),
            error_kind,
            reason,
            Arc::clone(audit),
        )
        .await;
        let mut out = OperationOutcome::pending(handle.op_id());
        out.status = OpStatus::Skipped;
        out.trace_id = Some(trace_id);
        out.error = Some(OperationError::new(error_kind, reason.to_string()));
        outcomes.insert(handle.op_id().to_string(), out);
    }
}

/// Find the first dependency of `op` whose outcome is non-Success.
fn first_failed_dep(
    op: &Operation,
    outcomes: &HashMap<String, OperationOutcome>,
) -> Option<String> {
    for dep_id in &op.depends_on {
        if let Some(o) = outcomes.get(dep_id) {
            if !o.status.is_success() {
                return Some(dep_id.clone());
            }
        } else {
            // Should never happen post-validation, but defensive.
            return Some(dep_id.clone());
        }
    }
    None
}

/// Resolve an [`OperationInput`] into a concrete `serde_json::Value`
/// against already-completed op outcomes.
fn resolve_input(
    input: &OperationInput,
    outcomes: &HashMap<String, OperationOutcome>,
) -> Result<serde_json::Value, String> {
    match input {
        OperationInput::Literal { value } => Ok(value.clone()),

        OperationInput::Ref { op_id, json_path } => {
            let source = outcomes
                .get(op_id)
                .ok_or_else(|| format!("ref to unknown op '{op_id}'"))?;
            let output = source
                .output
                .as_ref()
                .ok_or_else(|| format!("op '{op_id}' has no output yet"))?;
            json_path_lookup(output, json_path)
        }

        OperationInput::Merge { sources } => {
            // Shallow-merge resolved sources.  Objects merge key-wise (later
            // wins on conflict); arrays concatenate; primitives = last wins.
            let mut acc: Option<serde_json::Value> = None;
            for src in sources {
                let v = resolve_input(src, outcomes)?;
                acc = Some(match (acc.take(), v) {
                    (None, v) => v,
                    (Some(serde_json::Value::Object(mut a)), serde_json::Value::Object(b)) => {
                        for (k, v) in b {
                            a.insert(k, v);
                        }
                        serde_json::Value::Object(a)
                    }
                    (Some(serde_json::Value::Array(mut a)), serde_json::Value::Array(b)) => {
                        a.extend(b);
                        serde_json::Value::Array(a)
                    }
                    (Some(_), v) => v,
                });
            }
            acc.ok_or_else(|| "merge with 0 sources".to_string())
        }
    }
}

/// Tiny JSONPath lookup supporting `$`, `$.key`, `$.a.b`, and `$.a[0]`.
/// Anything more complex returns an error in Phase 1.
fn json_path_lookup(value: &serde_json::Value, path: &str) -> Result<serde_json::Value, String> {
    let p = path.trim_start_matches('$');
    if p.is_empty() || p == "." {
        return Ok(value.clone());
    }
    let mut cursor = value;
    let mut remaining = p.trim_start_matches('.');
    while !remaining.is_empty() {
        // Index access?
        if let Some(rest) = remaining.strip_prefix('[') {
            let close = rest
                .find(']')
                .ok_or_else(|| format!("unterminated bracket in path '{path}'"))?;
            let idx_str = &rest[..close];
            let idx: usize = idx_str
                .parse()
                .map_err(|_| format!("invalid array index '{idx_str}' in path '{path}'"))?;
            let arr = cursor.as_array().ok_or_else(|| {
                format!("expected array at '{path}' but got {}", cursor.kind_str())
            })?;
            cursor = arr
                .get(idx)
                .ok_or_else(|| format!("index {idx} out of bounds in '{path}'"))?;
            remaining = &rest[close + 1..];
            if remaining.starts_with('.') {
                remaining = &remaining[1..];
            }
            continue;
        }
        // Key access — find the next `.` or `[` to split on.
        let split = remaining.find(['.', '[']).unwrap_or(remaining.len());
        let key = &remaining[..split];
        let obj = cursor.as_object().ok_or_else(|| {
            format!("expected object at '{path}' but got {}", cursor.kind_str())
        })?;
        cursor = obj
            .get(key)
            .ok_or_else(|| format!("key '{key}' not found in path '{path}'"))?;
        remaining = &remaining[split..];
        if let Some(r) = remaining.strip_prefix('.') {
            remaining = r;
        }
    }
    Ok(cursor.clone())
}

/// Helper trait to render a Value's kind for error messages.
trait ValueKindStr {
    fn kind_str(&self) -> &'static str;
}

impl ValueKindStr for serde_json::Value {
    fn kind_str(&self) -> &'static str {
        match self {
            serde_json::Value::Null => "null",
            serde_json::Value::Bool(_) => "bool",
            serde_json::Value::Number(_) => "number",
            serde_json::Value::String(_) => "string",
            serde_json::Value::Array(_) => "array",
            serde_json::Value::Object(_) => "object",
        }
    }
}

/// Run one op end-to-end : check idempotency cache, run through the
/// runtime, populate the outcome.  Handles retries per the op's
/// [`efe_nano_sdk::RetryPolicy`].
///
/// **Sprint 7** : signature extended with `OpPreflightMeta`,
/// `caller_agent_id`, and `cost_tracker`.
#[allow(clippy::too_many_arguments)]
async fn run_one_op(
    handle: OpHandle,
    input: serde_json::Value,
    runtime: Arc<dyn NanoAgentExecutor>,
    audit: Arc<dyn AuditWriter>,
    idempotency: Arc<IdempotencyCache>,
    swarm_request_id: Uuid,
    tenant_id: efe_nano_sdk::TenantId,
    meta: OpPreflightMeta,
    caller_agent_id: Option<String>,
    cost_tracker: Arc<SwarmCostTracker>,
    catalog: Option<Arc<dyn Catalog>>,
    shadow_scheduler: Option<Arc<ShadowScheduler>>,
    envelope_trace_id: Option<TraceId>,
    rollout_counters: Option<Arc<crate::dispatcher::envelope::SwarmRolloutCounters>>,
) -> OperationOutcome {
    let mut outcome = OperationOutcome::pending(handle.op_id());
    outcome.trace_id = Some(meta.trace_id);
    let max_attempts = handle.op.retry.max_attempts.max(1);
    let backoff_ms = handle.op.retry.backoff_ms;

    // ─────────────────────────────────────────────────────────────────
    //  Sprint 8 — N13 : resolve the rollout policy for this op + this
    //  input.  When `catalog` is None, fall back to the planner-resolved
    //  `handle.agent` (pre-N13 behaviour).
    // ─────────────────────────────────────────────────────────────────
    let input_hash_resolved = crate::audit::hash_json(&input);
    let resolved = if let Some(cat) = catalog.as_ref() {
        match cat
            .resolve_for_dispatch(
                tenant_id,
                &handle.op.agent.id,
                &handle.op.agent.version,
                &input_hash_resolved,
            )
            .await
        {
            Ok(r) => r,
            Err(e) => {
                // **N13** : `resolve_for_dispatch` failure is rare —
                // typically a Postgres outage or a transient lock
                // contention.  We log loudly + fall back to the
                // planner-resolved `handle.agent` so production keeps
                // serving with the policy that was active at planning
                // time.  Subsequent ops re-attempt resolution.
                tracing::warn!(
                    target: "efe_nano::dispatcher",
                    ?tenant_id,
                    agent_id = %handle.op.agent.id.as_str(),
                    error = %e,
                    "catalog.resolve_for_dispatch failed ; falling back to planner-resolved agent"
                );
                ResolvedAgent::Single(Arc::clone(&handle.agent))
            }
        }
    } else {
        ResolvedAgent::Single(Arc::clone(&handle.agent))
    };

    // Extract the prod agent + optional shadow pair info.
    let (prod_agent, shadow_target) = match resolved {
        ResolvedAgent::Single(a) => (a, None),
        ResolvedAgent::ShadowPair {
            prod,
            shadow,
            comparison,
            cost_attribution,
            shadow_budget_cents,
            shadow_on_cache_hit,
        } => (
            prod,
            Some((shadow, comparison, cost_attribution, shadow_budget_cents, shadow_on_cache_hit)),
        ),
    };

    // **N14 (B2 fix)** : canary routing accounting.  If the catalog
    // had a Canary policy AND the resolved version DIFFERS from the
    // policy's prod_version, increment the canary_routed counter.
    if let (Some(cat), Some(c)) = (catalog.as_ref(), rollout_counters.as_ref()) {
        if let Ok(efe_nano_sdk::RolloutPolicy::Canary { prod_version, .. }) = cat
            .get_rollout_policy(tenant_id, &handle.op.agent.id)
            .await
        {
            if prod_agent.version != prod_version {
                c.inc_canary_routed();
            }
        }
    }

    // ─────────────────────────────────────────────────────────────────
    //  Idempotency cache : check first.
    //
    //  Sprint 8 — N12 : `derive_key` now takes the full `&NanoAgent` so
    //  the v2 9-component discriminator (execution_mode, Mode C
    //  provider/model/template, pii_strategy, ...) is materialized.
    //  Pre-N12 calls that passed only `(id, version)` are silently
    //  collision-prone — see `EFE_M50_N12_CACHE_KEY_HARDENING_ARCHITECTURE.md`.
    // ─────────────────────────────────────────────────────────────────
    let cache_key = idempotency.derive_key(tenant_id, &prod_agent, &input);
    if let Some(cached) = idempotency.get(&cache_key).await {
        outcome.status = OpStatus::Success;
        outcome.output = Some(cached.output.clone());
        outcome.cost_cents = cached.cost_cents;
        outcome.cache_hit = true;
        outcome.latency_ms = 0;
        cost_tracker.add(cached.cost_cents);
        // Sprint 8 — N12 G5 : write the cache-hit trace.  N14 :
        // propagate envelope_trace_id so the audit row joins to its swarm.
        write_cache_hit_trace(
            audit.as_ref(),
            tenant_id,
            swarm_request_id,
            &handle.op.op_id,
            caller_agent_id.clone(),
            &prod_agent,
            &input,
            &cached,
            envelope_trace_id,
        )
        .await;
        // Sprint 8 — N13 : if Shadow policy with shadow_on_cache_hit=true,
        // still schedule shadow even on cache hit (to keep shadow window
        // full during the quality-gate window).
        if let (Some(scheduler), Some((shadow_agent, comparison, cost_attribution, shadow_budget, true))) =
            (shadow_scheduler.as_ref(), shadow_target.as_ref())
        {
            // **N14 (B2 fix)** : count the scheduled shadow.
            if let Some(c) = rollout_counters.as_ref() {
                c.inc_shadow_run();
            }
            ShadowScheduler::schedule(
                scheduler,
                ProdOutcomeSnapshot {
                    agent_version: prod_agent.version.clone(),
                    output: cached.output.clone(),
                    cost_cents: cached.cost_cents,
                    latency_ms: 0,
                    trace_id: meta.trace_id,
                },
                Arc::clone(shadow_agent),
                Arc::clone(&prod_agent),
                // **N13 (B2 fix)** : Shadow policies use default gates
                // for auto-rollback evaluation.  This arms the safety
                // net during the quality-gate window.
                Some(efe_nano_sdk::PromotionGates::default()),
                *shadow_budget,
                ShadowDispatchContext {
                    tenant_id,
                    swarm_request_id,
                    op_id: handle.op.op_id.clone(),
                    caller_agent_id: caller_agent_id.clone(),
                    input: input.clone(),
                    comparison: comparison.clone(),
                    cost_attribution: *cost_attribution,
                    swarm_envelope_trace_id: envelope_trace_id,
                },
            );
        }
        return outcome;
    }

    // ─────────────────────────────────────────────────────────────────
    //  Run with retries.
    // ─────────────────────────────────────────────────────────────────
    let mut last_err: Option<OperationError> = None;
    let agent = &*prod_agent;
    for attempt in 1..=max_attempts {
        outcome.attempts = attempt;
        // Sprint 7 — N1 / N2 / N7 : threaded context.  Note we re-use
        // the pre-allocated `meta.trace_id` so the runtime's trace
        // write uses the SAME id that downstream ops referenced.
        let mut ctx = CallContext::from_swarm(
            tenant_id,
            swarm_request_id,
            handle.op.op_id.clone(),
            meta.parent_trace_ids.clone(),
            caller_agent_id.clone(),
            Arc::clone(&audit),
            handle.effective_timeout_ms,
            Some(Arc::clone(&cost_tracker)),
        );
        // Override the auto-generated trace_id with our pre-allocated
        // one so the audit write matches what downstream ops linked to.
        ctx.trace_id = meta.trace_id;
        // Sprint 7 — N6 : tell the runtime to use the AST-derived
        // source-hash instead of recomputing it from the resolved value.
        ctx.input_hash_source_override = Some(meta.input_hash_source);
        // **Sprint 8 — N14** : thread the swarm envelope anchor so the
        // runtime sets it on the trace it emits.
        ctx.swarm_envelope_trace_id = envelope_trace_id;

        match runtime
            .execute_with_context(agent, input.clone(), &ctx)
            .await
        {
            Ok(resp) => {
                outcome.status = OpStatus::Success;
                outcome.output = Some(resp.output.clone());
                outcome.cost_cents = resp.cost_cents;
                outcome.latency_ms = resp.latency_ms;
                outcome.trace_id = Some(resp.trace_id);
                cost_tracker.add(resp.cost_cents);

                // Populate idempotency cache when policy allows.
                let ttl = match &agent.idempotency {
                    efe_nano_sdk::IdempotencyPolicy::Auto { ttl_sec } => Some(*ttl_sec),
                    efe_nano_sdk::IdempotencyPolicy::Manual { ttl_sec } => Some(*ttl_sec),
                    efe_nano_sdk::IdempotencyPolicy::Disabled => None,
                };
                // Sprint 8 — N13 : capture a snapshot of prod outcome
                // BEFORE consuming `resp.output` into the cache entry,
                // so the shadow scheduler has a stable reference.
                let prod_snapshot = ProdOutcomeSnapshot {
                    agent_version: agent.version.clone(),
                    output: resp.output.clone(),
                    cost_cents: resp.cost_cents,
                    latency_ms: resp.latency_ms,
                    trace_id: resp.trace_id,
                };

                if let Some(ttl_sec) = ttl {
                    // Sprint 8 — N12 : build a v2 CacheEntry carrying
                    // full provenance.  `original_trace_id = resp.trace_id`
                    // is what the cache-hit trace will later declare as
                    // its `parent_trace_ids[0]`.
                    let entry = CacheEntry::new(
                        resp.output,
                        resp.cost_cents,
                        ttl_sec,
                        tenant_id,
                        agent.id.clone(),
                        agent.version.clone(),
                        crate::audit::hash_json(&input),
                        resp.trace_id,
                        mode_c_metadata_from(agent),
                    );
                    idempotency.put(cache_key, entry).await;
                }

                // Sprint 8 — N13 : if shadow policy, spawn shadow now.
                if let (Some(scheduler), Some((shadow_agent, comparison, cost_attribution, shadow_budget, _on_cache_hit))) =
                    (shadow_scheduler.as_ref(), shadow_target.as_ref())
                {
                    // **N14 (B2 fix)** : count the scheduled shadow.
                    if let Some(c) = rollout_counters.as_ref() {
                        c.inc_shadow_run();
                    }
                    ShadowScheduler::schedule(
                        scheduler,
                        prod_snapshot,
                        Arc::clone(shadow_agent),
                        Arc::clone(&prod_agent),
                        // **N13 (B2 fix)** : default gates arm auto-
                        // rollback on Shadow drift breach.
                        Some(efe_nano_sdk::PromotionGates::default()),
                        *shadow_budget,
                        ShadowDispatchContext {
                            tenant_id,
                            swarm_request_id,
                            op_id: handle.op.op_id.clone(),
                            caller_agent_id: caller_agent_id.clone(),
                            input: input.clone(),
                            comparison: comparison.clone(),
                            cost_attribution: *cost_attribution,
                            swarm_envelope_trace_id: envelope_trace_id,
                        },
                    );
                }

                return outcome;
            }
            Err(e) => {
                let op_err: OperationError = (&e).into();
                last_err = Some(op_err);
                if attempt < max_attempts {
                    if backoff_ms > 0 {
                        tokio::time::sleep(tokio::time::Duration::from_millis(backoff_ms as u64))
                            .await;
                    }
                    continue;
                }
            }
        }
    }

    // All attempts failed.
    outcome.status = OpStatus::Failed;
    outcome.error = last_err;
    outcome
}

// ─────────────────────────────────────────────────────────────────────
//  Sprint 8 — N12 § 4.3 : cache-hit trace + Mode C metadata builder
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N12** : Extract Mode-C-relevant metadata from a
/// `NanoAgent` for storage in [`CacheEntry::mode_c_metadata`].
///
/// Returns `None` for Mode A and Mode B agents.  The `tokens_used` +
/// `llm_cost_cents` fields default to 0 here because the
/// `NanoAgentResponse` doesn't surface those at the executor boundary
/// — Sprint 9 will plumb them through (M30 cost surfacing).  The
/// `prompt_template_version` defaults to 0 until the M21 prompt
/// registry is wired (Sprint 9 deliverable).
fn mode_c_metadata_from(agent: &efe_nano_sdk::NanoAgent) -> Option<LlmCacheMetadata> {
    if let efe_nano_sdk::ExecutionMode::LlmAugmented {
        provider,
        model,
        prompt_template_id,
        ..
    } = &agent.execution_mode
    {
        Some(LlmCacheMetadata {
            provider: provider.clone(),
            model: model.clone(),
            prompt_template_id: prompt_template_id.clone(),
            prompt_template_version: 0,
            tokens_used: 0,
            llm_cost_cents: 0.0,
        })
    } else {
        None
    }
}

/// **Sprint 8 — N12 § 4.3** : Write a cache-hit trace.
///
/// Called from `run_one_op` whenever the idempotency cache serves a hit,
/// so the HMAC audit chain stays contiguous.  The trace is
/// **pointer-shaped** — it carries pointer hashes (input, output) but
/// NOT the full bodies (those live on the original cold-call trace,
/// reachable via `parent_trace_ids[0] = original_trace_id`).
///
/// # Three subtle invariants
///
/// 1. **`error_kind = Some("CacheHit")`** — sentinel discriminator the
///    auditor uses to spot cache-served rows without an enum schema
///    change.  Sprint 9 will introduce `TraceKind::CacheHit | Fresh | ...`.
/// 2. **`full_input` / `full_output` = `None`** — pointer-shaped trace,
///    not data-shaped.  Bodies live on the original trace.
/// 3. **`parent_trace_ids = [original_trace_id]`** — the N1 lineage
///    link that lets the auditor walk back to the cold call to replay.
///
/// # Concurrency
///
/// Audit writes are fire-and-forget (the `AuditWriter` impl is
/// responsible for buffering/dropping on backend failure — see
/// `HmacChainAuditWriter::record`).  Failure here MUST NOT crash the
/// runtime — that would let an attacker DoS the platform by poisoning
/// the audit backend.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
async fn write_cache_hit_trace(
    audit: &dyn AuditWriter,
    tenant_id: efe_nano_sdk::TenantId,
    swarm_request_id: Uuid,
    op_id: &str,
    caller_agent_id: Option<String>,
    agent: &efe_nano_sdk::NanoAgent,
    input: &serde_json::Value,
    cached: &crate::idempotency::CachedOutput,
    envelope_trace_id: Option<TraceId>,
) {
    let now = Utc::now();
    let input_hash = crate::audit::hash_json(input);
    let output_hash = crate::audit::hash_json(&cached.output);
    let trace = NanoAgentTrace {
        trace_id: TraceId::new(),
        // N1 lineage link — parent is the cold call that populated the entry.
        parent_trace_ids: vec![cached.original_trace_id],
        swarm_request_id,
        operation_id: op_id.to_string(),
        tenant_id,
        caller_agent_id,
        nano_agent_id: agent.id.clone(),
        // N12 G4 : record the **producing** version, not the live
        // catalog version (which might have changed after the cold call).
        nano_agent_version: cached.resolved_version.clone(),
        execution_mode: agent.execution_mode.mode_str().to_string(),
        // For Literal inputs and the cache path, source == resolved.
        // The runtime path computes input_hash_source from the AST,
        // but here at the cache layer we only have the resolved Value.
        input_hash_source: input_hash,
        input_hash_resolved: input_hash,
        output_hash: Some(output_hash),
        // Sentinel discriminator — sprint 9 promotes to TraceKind enum.
        error_kind: Some("CacheHit".to_string()),
        started_at: now,
        completed_at: Some(now),
        latency_ms: Some(0),
        cost_cents: cached.cost_cents,
        // Mode C : surface the original `tokens_used` on the cache-hit
        // trace so billing reconciliation stays correct.
        llm_tokens_used: cached
            .mode_c_metadata
            .as_ref()
            .map(|m| m.tokens_used),
        // Pointer-shaped : the bodies live on the original trace.
        full_input: None,
        full_output: None,
        tombstone: None,
        // **N14** : envelope id propagated from the caller.
        swarm_envelope_trace_id: envelope_trace_id,
    };
    audit.record(trace).await;
}

/// Verify that every op_id in `request` appears in `outcomes`.  Used by
/// the dispatcher orchestrator before assembling the response.
#[allow(dead_code)]
pub(crate) fn assert_all_ops_resolved(
    request: &SwarmRequest,
    outcomes: &HashMap<String, OperationOutcome>,
) -> Result<(), DispatcherError> {
    let actual: HashSet<&str> = outcomes.keys().map(|s| s.as_str()).collect();
    let expected: HashSet<&str> = request.operations.iter().map(|o| o.op_id.as_str()).collect();
    let missing: Vec<String> = expected.difference(&actual).map(|s| s.to_string()).collect();
    if !missing.is_empty() {
        return Err(DispatcherError::InvalidDag {
            reason: format!("ops missing from outcomes : {missing:?}"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::stub::InMemoryAuditWriter;
    use crate::catalog::InMemoryCatalog;
    use crate::dispatcher::dag::plan;
    use crate::runtime::pure_rust::{HandlerRegistry, PureRustRuntime};
    use crate::starter_pack::register_all_starter_pack;
    use efe_nano_sdk::{NanoAgentId, NanoAgentRef, Operation, OperationInput, RetryPolicy};

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

    async fn fixture() -> (
        InMemoryCatalog,
        Arc<dyn NanoAgentExecutor>,
        Arc<dyn AuditWriter>,
        Arc<IdempotencyCache>,
    ) {
        let catalog = InMemoryCatalog::new();
        let registry = HandlerRegistry::new();
        register_all_starter_pack(&catalog, &registry).await.unwrap();
        let runtime: Arc<dyn NanoAgentExecutor> = Arc::new(PureRustRuntime::new(registry));
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let idempotency = Arc::new(IdempotencyCache::new());
        (catalog, runtime, audit, idempotency)
    }

    fn req(ops: Vec<Operation>) -> SwarmRequest {
        SwarmRequest {
            request_id: Uuid::new_v4(),
            tenant_id: efe_nano_sdk::TenantId::system(),
            caller_agent_id: None,
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

    #[tokio::test]
    async fn executes_single_op_successfully() {
        let (catalog, runtime, audit, idem) = fixture().await;
        let request = req(vec![op(
            "validate",
            "json_validate",
            &[],
            lit(serde_json::json!({
                "value": {"x": 1},
                "schema": {"type": "object", "required": ["x"]}
            })),
        )]);

        let plan = plan(&request, &catalog).await.unwrap();
        let outcomes = execute_plan(&plan, &request, runtime, audit, idem).await.unwrap();

        let o = outcomes.get("validate").unwrap();
        assert_eq!(o.status, OpStatus::Success);
        assert!(o.output.is_some());
        assert!(!o.cache_hit);
        // Sprint 7 — N1 : trace_id MUST be populated.
        assert!(o.trace_id.is_some());
    }

    #[tokio::test]
    async fn ref_input_pulls_from_upstream_output() {
        let (catalog, runtime, audit, idem) = fixture().await;
        let request = req(vec![
            op(
                "a",
                "json_validate",
                &[],
                lit(serde_json::json!({
                    "value": {"x": 1},
                    "schema": {"type": "object"}
                })),
            ),
            op(
                "b",
                "json_validate",
                &["a"],
                OperationInput::Merge {
                    sources: vec![
                        lit(serde_json::json!({
                            "schema": {"type": "boolean"}
                        })),
                        OperationInput::Merge {
                            sources: vec![
                                OperationInput::Ref {
                                    op_id: "a".to_string(),
                                    json_path: "$".to_string(),
                                },
                                lit(serde_json::json!({"value": true})),
                            ],
                        },
                    ],
                },
            ),
        ]);

        let plan = plan(&request, &catalog).await.unwrap();
        let outcomes = execute_plan(&plan, &request, runtime, audit, idem).await.unwrap();

        assert_eq!(outcomes.get("a").unwrap().status, OpStatus::Success);
        assert_eq!(outcomes.get("b").unwrap().status, OpStatus::Success);
    }

    #[tokio::test]
    async fn failed_dependency_marks_downstream_as_skipped() {
        let (catalog, runtime, audit, idem) = fixture().await;
        let request = req(vec![
            op(
                "a",
                "json_validate",
                &[],
                lit(serde_json::json!({
                    "value": {"x": 1}
                })),
            ),
            op(
                "b",
                "json_validate",
                &["a"],
                lit(serde_json::json!({
                    "value": {},
                    "schema": {"type": "object"}
                })),
            ),
        ]);

        let plan = plan(&request, &catalog).await.unwrap();
        let outcomes = execute_plan(&plan, &request, runtime, audit, idem).await.unwrap();

        assert_eq!(outcomes.get("a").unwrap().status, OpStatus::Failed);
        assert_eq!(outcomes.get("b").unwrap().status, OpStatus::Skipped);
        let b_err = outcomes.get("b").unwrap().error.as_ref().unwrap();
        assert_eq!(b_err.kind, OperationErrorKind::DependencyFailed);
    }

    #[tokio::test]
    async fn idempotency_cache_hit_on_second_run() {
        let (catalog, runtime, audit, idem) = fixture().await;
        let request = req(vec![op(
            "v",
            "json_validate",
            &[],
            lit(serde_json::json!({
                "value": {"x": 1},
                "schema": {"type": "object"}
            })),
        )]);

        let plan = plan(&request, &catalog).await.unwrap();
        let _ = execute_plan(&plan, &request, runtime.clone(), audit.clone(), idem.clone())
            .await
            .unwrap();
        let outcomes = execute_plan(&plan, &request, runtime, audit, idem).await.unwrap();
        assert!(outcomes.get("v").unwrap().cache_hit);
    }

    #[tokio::test]
    async fn parallel_layer_completes_all_ops() {
        let (catalog, runtime, audit, idem) = fixture().await;
        let mut ops: Vec<Operation> = Vec::new();
        for i in 0..5 {
            ops.push(op(
                &format!("op-{i}"),
                "json_validate",
                &[],
                lit(serde_json::json!({
                    "value": i,
                    "schema": {"type": "integer"}
                })),
            ));
        }
        let request = req(ops);
        let plan = plan(&request, &catalog).await.unwrap();
        assert_eq!(plan.depth(), 1);
        assert_eq!(plan.max_width, 5);
        let outcomes = execute_plan(&plan, &request, runtime, audit, idem).await.unwrap();
        assert_eq!(outcomes.len(), 5);
        for o in outcomes.values() {
            assert_eq!(o.status, OpStatus::Success);
        }
    }

    #[tokio::test]
    async fn json_path_supports_dot_notation() {
        let v = serde_json::json!({"a": {"b": 42}});
        assert_eq!(json_path_lookup(&v, "$.a.b").unwrap(), serde_json::json!(42));
        assert_eq!(
            json_path_lookup(&v, "$.a").unwrap(),
            serde_json::json!({"b": 42})
        );
        assert_eq!(json_path_lookup(&v, "$").unwrap(), v);
    }

    #[tokio::test]
    async fn json_path_supports_array_index() {
        let v = serde_json::json!({"items": [10, 20, 30]});
        assert_eq!(json_path_lookup(&v, "$.items[0]").unwrap(), serde_json::json!(10));
        assert_eq!(json_path_lookup(&v, "$.items[2]").unwrap(), serde_json::json!(30));
    }

    #[tokio::test]
    async fn json_path_invalid_index_errors() {
        let v = serde_json::json!({"items": [1, 2]});
        assert!(json_path_lookup(&v, "$.items[99]").is_err());
        assert!(json_path_lookup(&v, "$.missing").is_err());
    }

    #[tokio::test]
    async fn json_path_unterminated_bracket_errors() {
        let v = serde_json::json!({"items": [1, 2]});
        let err = json_path_lookup(&v, "$.items[0").unwrap_err();
        assert!(err.contains("unterminated"));
    }

    #[tokio::test]
    async fn json_path_non_numeric_index_errors() {
        let v = serde_json::json!({"items": [1, 2]});
        let err = json_path_lookup(&v, "$.items[abc]").unwrap_err();
        assert!(err.contains("invalid array index"));
    }

    #[tokio::test]
    async fn json_path_on_primitive_errors() {
        let v = serde_json::json!(42);
        let err = json_path_lookup(&v, "$.key").unwrap_err();
        assert!(err.contains("expected object"));
    }

    #[tokio::test]
    async fn json_path_index_on_object_errors() {
        let v = serde_json::json!({"key": "value"});
        let err = json_path_lookup(&v, "$[0]").unwrap_err();
        assert!(err.contains("expected array"));
    }

    #[tokio::test]
    async fn json_path_chained_array_object_access() {
        let v = serde_json::json!({"matches": [{"score": 0.9}, {"score": 0.1}]});
        assert_eq!(
            json_path_lookup(&v, "$.matches[0].score").unwrap(),
            serde_json::json!(0.9)
        );
    }

    #[tokio::test]
    async fn json_path_double_index_into_nested_array() {
        let v = serde_json::json!({"grid": [[1, 2], [3, 4]]});
        assert_eq!(
            json_path_lookup(&v, "$.grid[1][0]").unwrap(),
            serde_json::json!(3)
        );
    }

    #[tokio::test]
    async fn merge_with_zero_sources_errors() {
        let outcomes = std::collections::HashMap::new();
        let input = OperationInput::Merge { sources: vec![] };
        let err = resolve_input(&input, &outcomes).unwrap_err();
        assert!(err.contains("0 sources"));
    }

    #[tokio::test]
    async fn merge_array_inputs_concatenates() {
        let mut outcomes: std::collections::HashMap<String, OperationOutcome> =
            std::collections::HashMap::new();
        let mut prev = OperationOutcome::pending("prev");
        prev.output = Some(serde_json::json!([4, 5, 6]));
        prev.status = efe_nano_sdk::OpStatus::Success;
        outcomes.insert("prev".to_string(), prev);

        let input = OperationInput::Merge {
            sources: vec![
                OperationInput::Literal {
                    value: serde_json::json!([1, 2, 3]),
                },
                OperationInput::Ref {
                    op_id: "prev".to_string(),
                    json_path: "$".to_string(),
                },
            ],
        };
        let got = resolve_input(&input, &outcomes).unwrap();
        assert_eq!(got, serde_json::json!([1, 2, 3, 4, 5, 6]));
    }

    #[tokio::test]
    async fn merge_object_inputs_shallow_merges_later_wins() {
        let outcomes: std::collections::HashMap<String, OperationOutcome> =
            std::collections::HashMap::new();
        let input = OperationInput::Merge {
            sources: vec![
                OperationInput::Literal {
                    value: serde_json::json!({"a": 1, "b": 2}),
                },
                OperationInput::Literal {
                    value: serde_json::json!({"b": 99, "c": 3}),
                },
            ],
        };
        let got = resolve_input(&input, &outcomes).unwrap();
        assert_eq!(got, serde_json::json!({"a": 1, "b": 99, "c": 3}));
    }

    // ─────────────────────────────────────────────────────────────────
    //  Sprint 7 — N6 : hash_operation_input
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn hash_operation_input_literal_stable() {
        let a = OperationInput::Literal {
            value: serde_json::json!({"x": 1}),
        };
        let b = OperationInput::Literal {
            value: serde_json::json!({"x": 1}),
        };
        assert_eq!(hash_operation_input(&a), hash_operation_input(&b));
    }

    #[test]
    fn hash_operation_input_literal_vs_ref_differ() {
        let a = OperationInput::Literal {
            value: serde_json::json!({"x": 1}),
        };
        let b = OperationInput::Ref {
            op_id: "x".to_string(),
            json_path: "$.field".to_string(),
        };
        assert_ne!(hash_operation_input(&a), hash_operation_input(&b));
    }

    #[test]
    fn hash_operation_input_canonical_under_key_order() {
        let a = OperationInput::Literal {
            value: serde_json::json!({"a": 1, "b": 2}),
        };
        let b = OperationInput::Literal {
            value: serde_json::json!({"b": 2, "a": 1}),
        };
        // Canonical JSON sorts keys ⇒ same bytes ⇒ same hash.
        assert_eq!(hash_operation_input(&a), hash_operation_input(&b));
    }

    // ─────────────────────────────────────────────────────────────────
    //  Sprint 7 — N1 : collect_parent_trace_ids
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn collect_parents_literal_returns_empty() {
        let op = Operation {
            op_id: "a".into(),
            agent: NanoAgentRef::new(NanoAgentId::new("json_validate").unwrap()),
            input: OperationInput::Literal {
                value: serde_json::json!(null),
            },
            depends_on: vec![],
            timeout_ms: None,
            retry: RetryPolicy::default(),
        };
        let parents = collect_parent_trace_ids(&op, &HashMap::new());
        assert!(parents.is_empty());
    }

    #[test]
    fn collect_parents_ref_returns_one() {
        let mut map = HashMap::new();
        let tid = TraceId::new();
        map.insert("upstream".to_string(), tid);
        let op = Operation {
            op_id: "downstream".into(),
            agent: NanoAgentRef::new(NanoAgentId::new("json_validate").unwrap()),
            input: OperationInput::Ref {
                op_id: "upstream".to_string(),
                json_path: "$".to_string(),
            },
            depends_on: vec!["upstream".into()],
            timeout_ms: None,
            retry: RetryPolicy::default(),
        };
        let parents = collect_parent_trace_ids(&op, &map);
        assert_eq!(parents, vec![tid]);
    }

    #[test]
    fn collect_parents_merge_returns_unique_sorted() {
        let mut map = HashMap::new();
        let t_a = TraceId::new();
        let t_b = TraceId::new();
        map.insert("a".to_string(), t_a);
        map.insert("b".to_string(), t_b);
        let op = Operation {
            op_id: "c".into(),
            agent: NanoAgentRef::new(NanoAgentId::new("json_validate").unwrap()),
            input: OperationInput::Merge {
                sources: vec![
                    OperationInput::Ref {
                        op_id: "b".to_string(),
                        json_path: "$".to_string(),
                    },
                    OperationInput::Ref {
                        op_id: "a".to_string(),
                        json_path: "$".to_string(),
                    },
                    // Duplicate ref to `a` — must be deduped.
                    OperationInput::Ref {
                        op_id: "a".to_string(),
                        json_path: "$.x".to_string(),
                    },
                ],
            },
            depends_on: vec!["a".into(), "b".into()],
            timeout_ms: None,
            retry: RetryPolicy::default(),
        };
        let parents = collect_parent_trace_ids(&op, &map);
        // Sorted by op_id : a comes before b.
        assert_eq!(parents, vec![t_a, t_b]);
    }

    #[test]
    fn collect_parents_depends_on_without_ref_still_included() {
        let mut map = HashMap::new();
        let tid = TraceId::new();
        map.insert("ordering_only".to_string(), tid);
        let op = Operation {
            op_id: "after".into(),
            agent: NanoAgentRef::new(NanoAgentId::new("json_validate").unwrap()),
            input: OperationInput::Literal {
                value: serde_json::json!(null),
            },
            depends_on: vec!["ordering_only".into()],
            timeout_ms: None,
            retry: RetryPolicy::default(),
        };
        let parents = collect_parent_trace_ids(&op, &map);
        assert_eq!(parents, vec![tid]);
    }
}
