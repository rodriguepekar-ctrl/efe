//! **Sprint 8 — N14** : Swarm envelope construction + writing.
//!
//! See architecture doc `EFE_M50_N14_SWARM_ENVELOPE_TRACE_ARCHITECTURE.md`.
//!
//! # Responsibilities
//!
//! 1. [`EnvelopeBuilder`] — captures the request signature at swarm
//!    start, accumulates aggregated outcome at swarm end, produces the
//!    final [`SwarmEnvelopePayload`].
//! 2. [`build_swarm_envelope_trace`] — turns a payload into the
//!    [`NanoAgentTrace`] that goes on the audit chain.
//! 3. [`write_envelope`] — writes the trace via the audit writer.
//!    Resilient : audit failures don't propagate (N6 contract).
//! 4. [`compute_request_hash`] — deterministic SHA-256 over the
//!    canonical-serialised `SwarmRequest`.
//!
//! # Sentinel constants
//!
//! - `TRACE_KIND_SWARM_ENVELOPE = "SwarmEnvelope"` — `error_kind`
//!   discriminator on the envelope trace.
//! - `SWARM_ENVELOPE_AGENT_ID = "swarm_envelope"` — reserved agent id
//!   the catalog rejects.
//! - `SWARM_ENVELOPE_VERSION = "0.0.0"` — placeholder semver.
//! - `SWARM_ENVELOPE_EXECUTION_MODE = "swarm_envelope"` — discriminator.

use chrono::{DateTime, Utc};
use efe_nano_sdk::{
    EnvelopeKind, NanoAgentId, NanoAgentVersion, OpStatusCounts, SwarmEnvelopePayload,
    SwarmRequest, SwarmResponse, TenantId, TraceId,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use uuid::Uuid;

use crate::audit::{AuditWriter, NanoAgentTrace};
use crate::dispatcher::types::OperationOutcome;

// ─────────────────────────────────────────────────────────────────────
//  Sentinel constants
// ─────────────────────────────────────────────────────────────────────

/// **N14** : `error_kind` sentinel marking a trace as a swarm envelope.
pub const TRACE_KIND_SWARM_ENVELOPE: &str = "SwarmEnvelope";

/// **N14** : reserved agent id for the envelope's `nano_agent_id` field.
/// The catalog rejects `publish` calls with this id.
pub const SWARM_ENVELOPE_AGENT_ID: &str = "swarm_envelope";

/// **N14** : reserved placeholder version for the envelope.
pub const SWARM_ENVELOPE_VERSION: &str = "0.0.0";

/// **N14** : reserved execution_mode string for the envelope.
pub const SWARM_ENVELOPE_EXECUTION_MODE: &str = "swarm_envelope";

// ─────────────────────────────────────────────────────────────────────
//  EnvelopeMetrics (B3 fix — § 16 of architecture)
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N14 (B3 fix)** : the 8 Prometheus-style metrics
/// committed by architecture § 16.  Surfaces atomic counters +
/// summed histograms ; the M40 metric pipeline reads via `snapshot()`.
///
/// Sprint 9 may swap these for true Prometheus `Counter` / `Histogram`
/// types behind a feature flag.  For Sprint 8 ship we use atomic
/// counters with `(_sum, _count)` histogram approximation (Prometheus-
/// compatible exposition format).
#[derive(Debug, Default)]
pub struct EnvelopeMetrics {
    /// Total envelopes successfully written.
    pub envelopes_written_total: AtomicU64,
    /// Total envelope writes that failed (audit backend errored).
    pub envelopes_write_failed_total: AtomicU64,
    /// Total envelopes tombstoned via `tombstone_swarm_envelope`.
    pub envelope_tombstones_total: AtomicU64,
    /// Total `ReplayEngine::envelope_for_swarm` queries.
    pub envelopes_queried_total: AtomicU64,

    /// Histogram : sum of envelope latencies (ms).
    pub envelope_latency_ms_sum: AtomicU64,
    /// Histogram : count of envelope latency observations.
    pub envelope_latency_ms_count: AtomicU64,

    /// Histogram : sum of envelope op counts.
    pub envelope_op_count_sum: AtomicU64,
    /// Histogram : count of envelope op-count observations.
    pub envelope_op_count_count: AtomicU64,

    /// Histogram : sum of envelope total_cost_cents, in micro-cents.
    pub envelope_total_cost_microcents_sum: AtomicU64,
    /// Histogram : count of envelope cost observations.
    pub envelope_total_cost_count: AtomicU64,

    /// Histogram : sum of envelope serialised size in bytes.
    pub envelope_size_bytes_sum: AtomicU64,
    /// Histogram : count of envelope size observations.
    pub envelope_size_bytes_count: AtomicU64,
}

impl EnvelopeMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a successful envelope write.  Observes latency + op_count
    /// + cost + size histograms in one call.
    pub fn record_written(&self, payload: &SwarmEnvelopePayload, serialised_size: u64) {
        self.envelopes_written_total.fetch_add(1, Ordering::Relaxed);
        self.envelope_latency_ms_sum
            .fetch_add(payload.total_latency_ms as u64, Ordering::Relaxed);
        self.envelope_latency_ms_count.fetch_add(1, Ordering::Relaxed);
        self.envelope_op_count_sum
            .fetch_add(payload.op_count as u64, Ordering::Relaxed);
        self.envelope_op_count_count.fetch_add(1, Ordering::Relaxed);
        let microcents = (payload.total_cost_cents * 1_000_000.0).max(0.0) as u64;
        self.envelope_total_cost_microcents_sum
            .fetch_add(microcents, Ordering::Relaxed);
        self.envelope_total_cost_count.fetch_add(1, Ordering::Relaxed);
        self.envelope_size_bytes_sum
            .fetch_add(serialised_size, Ordering::Relaxed);
        self.envelope_size_bytes_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_write_failure(&self) {
        self.envelopes_write_failed_total
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_tombstone(&self) {
        self.envelope_tombstones_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_query(&self) {
        self.envelopes_queried_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Snapshot the counters into a plain struct for downstream
    /// serialisation / assertion.
    pub fn snapshot(&self) -> EnvelopeMetricsSnapshot {
        EnvelopeMetricsSnapshot {
            envelopes_written_total: self.envelopes_written_total.load(Ordering::Relaxed),
            envelopes_write_failed_total: self.envelopes_write_failed_total.load(Ordering::Relaxed),
            envelope_tombstones_total: self.envelope_tombstones_total.load(Ordering::Relaxed),
            envelopes_queried_total: self.envelopes_queried_total.load(Ordering::Relaxed),
            envelope_latency_ms_sum: self.envelope_latency_ms_sum.load(Ordering::Relaxed),
            envelope_latency_ms_count: self.envelope_latency_ms_count.load(Ordering::Relaxed),
            envelope_op_count_sum: self.envelope_op_count_sum.load(Ordering::Relaxed),
            envelope_op_count_count: self.envelope_op_count_count.load(Ordering::Relaxed),
            envelope_total_cost_microcents_sum: self
                .envelope_total_cost_microcents_sum
                .load(Ordering::Relaxed),
            envelope_total_cost_count: self.envelope_total_cost_count.load(Ordering::Relaxed),
            envelope_size_bytes_sum: self.envelope_size_bytes_sum.load(Ordering::Relaxed),
            envelope_size_bytes_count: self.envelope_size_bytes_count.load(Ordering::Relaxed),
        }
    }
}

/// **N14 (B3 fix)** : Plain-data snapshot of [`EnvelopeMetrics`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnvelopeMetricsSnapshot {
    pub envelopes_written_total: u64,
    pub envelopes_write_failed_total: u64,
    pub envelope_tombstones_total: u64,
    pub envelopes_queried_total: u64,
    pub envelope_latency_ms_sum: u64,
    pub envelope_latency_ms_count: u64,
    pub envelope_op_count_sum: u64,
    pub envelope_op_count_count: u64,
    pub envelope_total_cost_microcents_sum: u64,
    pub envelope_total_cost_count: u64,
    pub envelope_size_bytes_sum: u64,
    pub envelope_size_bytes_count: u64,
}

// ─────────────────────────────────────────────────────────────────────
//  SwarmRolloutCounters (B2 fix)
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N14 (B2 fix)** : Shared per-swarm atomic counters for
/// shadow / canary activity.
///
/// One instance is created per `Dispatcher::dispatch` invocation,
/// passed through the executor + shadow scheduler so each op's
/// rollout-related event is counted into the envelope.
///
/// # Semantics
///
/// * `shadow_runs_scheduled` — incremented when the scheduler queues
///   a shadow task (regardless of whether the shadow ultimately
///   succeeds, fails, or is dropped due to budget exhaustion).
/// * `shadow_mismatches` — incremented by the shadow scheduler when
///   a `Mismatch` comparison outcome is recorded.  May not be visible
///   in the envelope if the shadow comparison completes AFTER the
///   envelope is written (acknowledged in architecture § F11).
/// * `canary_routed` — incremented when `resolve_for_dispatch` returns
///   a `Single` whose version differs from the policy's `prod_version`
///   AND the policy is `Canary` (i.e. the op landed on the canary
///   cohort).
#[derive(Debug, Default)]
pub struct SwarmRolloutCounters {
    pub shadow_runs_scheduled: AtomicU64,
    pub shadow_mismatches: AtomicU64,
    pub canary_routed: AtomicU64,
}

impl SwarmRolloutCounters {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn shadow_runs(&self) -> u32 {
        self.shadow_runs_scheduled.load(Ordering::Relaxed) as u32
    }
    pub fn shadow_mismatches(&self) -> u32 {
        self.shadow_mismatches.load(Ordering::Relaxed) as u32
    }
    pub fn canary_routed(&self) -> u32 {
        self.canary_routed.load(Ordering::Relaxed) as u32
    }

    pub fn inc_shadow_run(&self) {
        self.shadow_runs_scheduled.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_shadow_mismatch(&self) {
        self.shadow_mismatches.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_canary_routed(&self) {
        self.canary_routed.fetch_add(1, Ordering::Relaxed);
    }
}

// ─────────────────────────────────────────────────────────────────────
//  EnvelopeBuilder
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N14** : Construction-time + finalisation-time helper
/// for the swarm envelope.
///
/// Constructed at dispatch start with the request signature captured ;
/// finalised at dispatch end by walking the outcomes map and the
/// response.  Single-emit — Sprint 8 doesn't support streaming.
pub struct EnvelopeBuilder {
    envelope_trace_id: TraceId,
    // ─── request signature (frozen at construction) ───
    swarm_request_id: Uuid,
    tenant_id: TenantId,
    started_at: DateTime<Utc>,
    started_instant: Instant,
    caller_agent_id: Option<String>,
    idempotency_key: Option<String>,
    audit_tags: Vec<String>,
    max_parallelism: u32,
    timeout_ms: u32,
    cost_cap_cents: Option<f64>,
    streaming: bool,
    op_count: u32,
    op_ids: Vec<String>,
    request_hash: [u8; 32],
    parent_envelope_trace_id: Option<TraceId>,
}

impl EnvelopeBuilder {
    /// **N14** : construct from a `SwarmRequest` + pre-allocated envelope id.
    pub fn new(envelope_trace_id: TraceId, request: &SwarmRequest) -> Self {
        let op_ids: Vec<String> = request.operations.iter().map(|o| o.op_id.clone()).collect();
        let op_count = op_ids.len() as u32;
        let request_hash = compute_request_hash(request);
        Self {
            envelope_trace_id,
            swarm_request_id: request.request_id,
            tenant_id: request.tenant_id,
            started_at: Utc::now(),
            started_instant: Instant::now(),
            caller_agent_id: request.caller_agent_id.clone(),
            idempotency_key: request.idempotency_key.clone(),
            audit_tags: request.audit_tags.clone(),
            max_parallelism: request.max_parallelism,
            timeout_ms: request.timeout_ms,
            cost_cap_cents: request.cost_cap_cents,
            streaming: request.streaming,
            op_count,
            op_ids,
            request_hash,
            parent_envelope_trace_id: request.parent_envelope_trace_id,
        }
    }

    pub fn envelope_trace_id(&self) -> TraceId {
        self.envelope_trace_id
    }

    pub fn started_at(&self) -> DateTime<Utc> {
        self.started_at
    }

    /// **N14** : finalise the envelope at swarm end, aggregating
    /// outcomes + cache hits + shadow counts.
    ///
    /// **B1 fix** : op_status_counts is computed from
    /// `response.results` (authoritative aggregate) rather than
    /// `outcomes` — the failure / timeout paths populate `response`
    /// even when `outcomes` is empty, ensuring `op_status_counts.total()
    /// == op_count` (invariant I10).
    pub fn finalize(
        self,
        response: &SwarmResponse,
        outcomes: &HashMap<String, OperationOutcome>,
    ) -> SwarmEnvelopePayload {
        let op_status_counts = aggregate_op_statuses_from_response(response);
        let cache_hit_count = outcomes.values().filter(|o| o.cache_hit).count() as u32;
        let total_latency_ms = self
            .started_instant
            .elapsed()
            .as_millis()
            .min(u32::MAX as u128) as u32;

        SwarmEnvelopePayload {
            swarm_request_id: self.swarm_request_id,
            tenant_id: self.tenant_id,
            caller_agent_id: self.caller_agent_id,
            idempotency_key: self.idempotency_key,
            audit_tags: self.audit_tags,
            max_parallelism: self.max_parallelism,
            timeout_ms: self.timeout_ms,
            cost_cap_cents: self.cost_cap_cents,
            streaming: self.streaming,
            op_count: self.op_count,
            op_ids: self.op_ids,
            request_hash: self.request_hash,
            started_at: self.started_at,
            overall_status: response.overall_status,
            total_cost_cents: response.total_cost_cents,
            total_latency_ms: response.total_latency_ms.max(total_latency_ms),
            completed_at: Utc::now(),
            op_status_counts,
            cache_hit_count,
            // N13 counts : set by Wave-5 dispatcher integration via
            // `finalize_with_rollout_counts`.  Default 0 here.
            shadow_run_count: 0,
            shadow_mismatch_count: 0,
            canary_routed_count: 0,
            parent_envelope_trace_id: self.parent_envelope_trace_id,
            envelope_format_version: 1,
            envelope_kind: EnvelopeKind::Terminal,
        }
    }

    /// **N14** : alternative finalisation that explicitly carries the
    /// rollout-related counts.  Used when the dispatcher has access to
    /// the shadow scheduler + drift window.
    pub fn finalize_with_rollout_counts(
        self,
        response: &SwarmResponse,
        outcomes: &HashMap<String, OperationOutcome>,
        shadow_run_count: u32,
        shadow_mismatch_count: u32,
        canary_routed_count: u32,
    ) -> SwarmEnvelopePayload {
        let mut payload = self.finalize(response, outcomes);
        payload.shadow_run_count = shadow_run_count;
        payload.shadow_mismatch_count = shadow_mismatch_count;
        payload.canary_routed_count = canary_routed_count;
        payload
    }

    /// **N14 (B2 fix)** : Finalise from shared atomic counters.
    /// Reads `shadow_runs_scheduled` / `shadow_mismatches` /
    /// `canary_routed` from the `Arc<SwarmRolloutCounters>` shared
    /// across the dispatcher's lifetime.
    pub fn finalize_with_counters(
        self,
        response: &SwarmResponse,
        outcomes: &HashMap<String, OperationOutcome>,
        counters: &SwarmRolloutCounters,
    ) -> SwarmEnvelopePayload {
        self.finalize_with_rollout_counts(
            response,
            outcomes,
            counters.shadow_runs(),
            counters.shadow_mismatches(),
            counters.canary_routed(),
        )
    }
}

/// **B1 fix** : Aggregate op statuses from the SwarmResponse (the
/// authoritative aggregate), not from outcomes.  This handles the
/// planner-failure / internal-error / timeout paths where outcomes
/// may be empty but `response.results` is populated with synthesized
/// Skipped / Failed / Cancelled rows.
fn aggregate_op_statuses_from_response(response: &SwarmResponse) -> OpStatusCounts {
    use efe_nano_sdk::OpStatus;
    let mut c = OpStatusCounts::default();
    for r in response.results.values() {
        match r.status {
            OpStatus::Success => c.success += 1,
            OpStatus::Failed => c.failed += 1,
            OpStatus::Skipped => c.skipped += 1,
            OpStatus::Cancelled => c.cancelled += 1,
            OpStatus::Pending | OpStatus::Running => {
                // In-flight at envelope-write time — count as
                // cancelled for the envelope (best-effort summary).
                c.cancelled += 1;
            }
        }
    }
    c
}

// ─────────────────────────────────────────────────────────────────────
//  build_swarm_envelope_trace
// ─────────────────────────────────────────────────────────────────────

/// **N14** : turn a `SwarmEnvelopePayload` into the audit-chain trace.
pub fn build_swarm_envelope_trace(
    envelope_trace_id: TraceId,
    payload: &SwarmEnvelopePayload,
) -> NanoAgentTrace {
    // The envelope's `full_input` carries the request signature ; the
    // `full_output` carries the aggregated outcome.  We serialize the
    // payload once and split it across the two fields.
    let full_value = serde_json::to_value(payload).unwrap_or(serde_json::Value::Null);

    // Hash inputs : the canonical request hash IS the input_hash.
    let input_hash = payload.request_hash;
    // Output hash : SHA-256 over the canonical outcome part of the payload.
    let output_hash = compute_outcome_hash(payload);

    NanoAgentTrace {
        trace_id: envelope_trace_id,
        // Parent traces : the parent envelope, if any (meta-swarm).
        parent_trace_ids: payload
            .parent_envelope_trace_id
            .map(|p| vec![p])
            .unwrap_or_default(),
        swarm_request_id: payload.swarm_request_id,
        operation_id: SWARM_ENVELOPE_EXECUTION_MODE.to_string(),
        tenant_id: payload.tenant_id,
        caller_agent_id: payload.caller_agent_id.clone(),
        nano_agent_id: NanoAgentId::new(SWARM_ENVELOPE_AGENT_ID)
            .expect("reserved envelope agent id is always valid"),
        nano_agent_version: NanoAgentVersion::new(SWARM_ENVELOPE_VERSION)
            .expect("reserved envelope version is always valid"),
        execution_mode: SWARM_ENVELOPE_EXECUTION_MODE.to_string(),
        input_hash_source: input_hash,
        input_hash_resolved: input_hash,
        output_hash: Some(output_hash),
        error_kind: Some(TRACE_KIND_SWARM_ENVELOPE.to_string()),
        started_at: payload.started_at,
        completed_at: Some(payload.completed_at),
        latency_ms: Some(payload.total_latency_ms),
        cost_cents: payload.total_cost_cents,
        llm_tokens_used: None,
        full_input: Some(full_value.clone()),
        full_output: Some(full_value),
        tombstone: None,
        // The envelope IS the envelope ; no self-reference.
        swarm_envelope_trace_id: None,
    }
}

// ─────────────────────────────────────────────────────────────────────
//  write_envelope
// ─────────────────────────────────────────────────────────────────────

/// **N14** : write the envelope to the audit chain.  Resilient :
/// audit-write failures are logged but never propagated.
pub async fn write_envelope(
    audit: &dyn AuditWriter,
    builder: EnvelopeBuilder,
    response: &SwarmResponse,
    outcomes: &HashMap<String, OperationOutcome>,
) -> TraceId {
    let envelope_trace_id = builder.envelope_trace_id();
    let payload = builder.finalize(response, outcomes);
    let trace = build_swarm_envelope_trace(envelope_trace_id, &payload);
    audit.record(trace).await;
    envelope_trace_id
}

/// **N14 (B2 fix)** : write_envelope variant that reads rollout
/// counters from a shared `Arc<SwarmRolloutCounters>`.
pub async fn write_envelope_with_counters(
    audit: &dyn AuditWriter,
    builder: EnvelopeBuilder,
    response: &SwarmResponse,
    outcomes: &HashMap<String, OperationOutcome>,
    counters: &SwarmRolloutCounters,
) -> TraceId {
    write_envelope_full(audit, builder, response, outcomes, counters, None).await
}

/// **N14 (B3 fix)** : full envelope writer with metrics recording.
/// When `metrics` is `Some`, observes write success / failure +
/// latency + op_count + cost + size histograms.
pub async fn write_envelope_full(
    audit: &dyn AuditWriter,
    builder: EnvelopeBuilder,
    response: &SwarmResponse,
    outcomes: &HashMap<String, OperationOutcome>,
    counters: &SwarmRolloutCounters,
    metrics: Option<&EnvelopeMetrics>,
) -> TraceId {
    let envelope_trace_id = builder.envelope_trace_id();
    let payload = builder.finalize_with_counters(response, outcomes, counters);
    let trace = build_swarm_envelope_trace(envelope_trace_id, &payload);
    // Approximate serialised size by hashing full_input bytes (no
    // re-serialise — we already have the value as JSON Value).
    let size_bytes = trace
        .full_input
        .as_ref()
        .map(|v| serde_json::to_string(v).map(|s| s.len() as u64).unwrap_or(0))
        .unwrap_or(0);
    audit.record(trace).await;
    if let Some(m) = metrics {
        m.record_written(&payload, size_bytes);
    }
    envelope_trace_id
}

/// **N14** : write_envelope variant carrying rollout aggregate counts.
#[allow(clippy::too_many_arguments)]
pub async fn write_envelope_with_rollout(
    audit: &dyn AuditWriter,
    builder: EnvelopeBuilder,
    response: &SwarmResponse,
    outcomes: &HashMap<String, OperationOutcome>,
    shadow_run_count: u32,
    shadow_mismatch_count: u32,
    canary_routed_count: u32,
) -> TraceId {
    let envelope_trace_id = builder.envelope_trace_id();
    let payload = builder.finalize_with_rollout_counts(
        response,
        outcomes,
        shadow_run_count,
        shadow_mismatch_count,
        canary_routed_count,
    );
    let trace = build_swarm_envelope_trace(envelope_trace_id, &payload);
    audit.record(trace).await;
    envelope_trace_id
}

// ─────────────────────────────────────────────────────────────────────
//  compute_request_hash
// ─────────────────────────────────────────────────────────────────────

/// **N14** : SHA-256 over a canonical serialisation of the
/// `SwarmRequest`.  Deterministic across machines, processes, restarts.
pub fn compute_request_hash(request: &SwarmRequest) -> [u8; 32] {
    let canonical = canonicalize_swarm_request(request);
    let digest = Sha256::digest(canonical.as_bytes());
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

fn canonicalize_swarm_request(request: &SwarmRequest) -> String {
    // Use the existing canonical-JSON serializer from the idempotency
    // module so request_hash + cache key derivations align.
    let value = serde_json::to_value(request).unwrap_or(serde_json::Value::Null);
    crate::idempotency::canonicalize_json(&value)
}

fn compute_outcome_hash(payload: &SwarmEnvelopePayload) -> [u8; 32] {
    // Hash only the outcome-relevant subset of the payload.  This
    // makes output_hash deterministic over the outcome regardless of
    // future schema additions to the request part.
    let outcome = serde_json::json!({
        "overall_status": payload.overall_status,
        "total_cost_cents": format!("{:.6}", payload.total_cost_cents),
        "total_latency_ms": payload.total_latency_ms,
        "op_status_counts": payload.op_status_counts,
        "cache_hit_count": payload.cache_hit_count,
        "shadow_run_count": payload.shadow_run_count,
        "shadow_mismatch_count": payload.shadow_mismatch_count,
        "canary_routed_count": payload.canary_routed_count,
    });
    let canonical = crate::idempotency::canonicalize_json(&outcome);
    let digest = Sha256::digest(canonical.as_bytes());
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::{
        NanoAgentId, NanoAgentRef, Operation, OperationInput, OperationResult, RetryPolicy,
        SwarmStatus,
    };

    fn sample_request() -> SwarmRequest {
        SwarmRequest {
            request_id: Uuid::new_v4(),
            tenant_id: TenantId::system(),
            caller_agent_id: Some("claude".into()),
            operations: vec![Operation {
                op_id: "a".into(),
                agent: NanoAgentRef::new(NanoAgentId::new("test_agent").unwrap()),
                input: OperationInput::Literal { value: serde_json::json!({"x": 1}) },
                depends_on: vec![],
                timeout_ms: None,
                retry: RetryPolicy::default(),
            }],
            max_parallelism: 4,
            timeout_ms: 30_000,
            idempotency_key: Some("key-x".into()),
            streaming: false,
            audit_tags: vec!["t1".into()],
            cost_cap_cents: Some(1.0),
            parent_envelope_trace_id: None,
        }
    }

    fn sample_response(request_id: Uuid) -> SwarmResponse {
        let mut results = HashMap::new();
        results.insert(
            "a".into(),
            OperationResult {
                op_id: "a".into(),
                status: efe_nano_sdk::OpStatus::Success,
                output: Some(serde_json::json!({"ok": true})),
                error: None,
                latency_ms: 10,
                cost_cents: 0.001,
            },
        );
        SwarmResponse {
            request_id,
            overall_status: SwarmStatus::Success,
            results,
            total_latency_ms: 20,
            total_cost_cents: 0.001,
            swarm_envelope_trace_id: None,
        }
    }

    fn sample_outcomes() -> HashMap<String, OperationOutcome> {
        let mut o = HashMap::new();
        let mut outcome = OperationOutcome::pending("a");
        outcome.status = efe_nano_sdk::OpStatus::Success;
        outcome.cost_cents = 0.001;
        o.insert("a".into(), outcome);
        o
    }

    #[test]
    fn builder_captures_request_signature() {
        let req = sample_request();
        let etid = TraceId::new();
        let b = EnvelopeBuilder::new(etid, &req);
        assert_eq!(b.envelope_trace_id(), etid);
        assert_eq!(b.op_count, 1);
        assert_eq!(b.op_ids, vec!["a".to_string()]);
        assert_eq!(b.caller_agent_id.as_deref(), Some("claude"));
        assert_eq!(b.idempotency_key.as_deref(), Some("key-x"));
        assert_eq!(b.audit_tags, vec!["t1"]);
        assert_eq!(b.max_parallelism, 4);
        assert_eq!(b.timeout_ms, 30_000);
        assert_eq!(b.cost_cap_cents, Some(1.0));
        assert!(!b.streaming);
    }

    #[test]
    fn builder_finalize_aggregates() {
        let req = sample_request();
        let etid = TraceId::new();
        let b = EnvelopeBuilder::new(etid, &req);
        let resp = sample_response(req.request_id);
        let outcomes = sample_outcomes();
        let payload = b.finalize(&resp, &outcomes);
        assert_eq!(payload.swarm_request_id, req.request_id);
        assert_eq!(payload.op_count, 1);
        assert_eq!(payload.op_status_counts.success, 1);
        assert_eq!(payload.cache_hit_count, 0);
        assert_eq!(payload.envelope_format_version, 1);
        assert_eq!(payload.envelope_kind, EnvelopeKind::Terminal);
    }

    #[test]
    fn request_hash_deterministic_across_calls() {
        let req = sample_request();
        let h1 = compute_request_hash(&req);
        let h2 = compute_request_hash(&req);
        assert_eq!(h1, h2);
    }

    #[test]
    fn request_hash_changes_on_field_change() {
        let mut req = sample_request();
        let h1 = compute_request_hash(&req);
        req.max_parallelism = 99;
        let h2 = compute_request_hash(&req);
        assert_ne!(h1, h2);
    }

    #[test]
    fn outcome_hash_deterministic() {
        let req = sample_request();
        let etid = TraceId::new();
        let b = EnvelopeBuilder::new(etid, &req);
        let payload = b.finalize(&sample_response(req.request_id), &sample_outcomes());
        let h1 = compute_outcome_hash(&payload);
        let h2 = compute_outcome_hash(&payload);
        assert_eq!(h1, h2);
    }

    #[test]
    fn build_envelope_trace_has_sentinel() {
        let req = sample_request();
        let etid = TraceId::new();
        let b = EnvelopeBuilder::new(etid, &req);
        let payload = b.finalize(&sample_response(req.request_id), &sample_outcomes());
        let trace = build_swarm_envelope_trace(etid, &payload);
        assert_eq!(trace.error_kind.as_deref(), Some(TRACE_KIND_SWARM_ENVELOPE));
        assert_eq!(trace.nano_agent_id.as_str(), SWARM_ENVELOPE_AGENT_ID);
        assert_eq!(trace.execution_mode, SWARM_ENVELOPE_EXECUTION_MODE);
        assert!(trace.full_input.is_some());
        assert!(trace.full_output.is_some());
        // The envelope itself has no swarm_envelope_trace_id reference.
        assert!(trace.swarm_envelope_trace_id.is_none());
    }

    #[test]
    fn build_envelope_with_parent_sets_parent_trace_ids() {
        let mut req = sample_request();
        let parent_etid = TraceId::new();
        req.parent_envelope_trace_id = Some(parent_etid);
        let etid = TraceId::new();
        let b = EnvelopeBuilder::new(etid, &req);
        let payload = b.finalize(&sample_response(req.request_id), &sample_outcomes());
        let trace = build_swarm_envelope_trace(etid, &payload);
        assert_eq!(trace.parent_trace_ids, vec![parent_etid]);
    }

    #[test]
    fn finalize_with_rollout_counts() {
        let req = sample_request();
        let etid = TraceId::new();
        let b = EnvelopeBuilder::new(etid, &req);
        let payload = b.finalize_with_rollout_counts(
            &sample_response(req.request_id),
            &sample_outcomes(),
            3,
            1,
            2,
        );
        assert_eq!(payload.shadow_run_count, 3);
        assert_eq!(payload.shadow_mismatch_count, 1);
        assert_eq!(payload.canary_routed_count, 2);
    }

    #[test]
    fn cache_hit_count_aggregation() {
        let req = sample_request();
        let etid = TraceId::new();
        let b = EnvelopeBuilder::new(etid, &req);
        let mut outcomes = sample_outcomes();
        // Mark the op as a cache hit.
        if let Some(o) = outcomes.get_mut("a") {
            o.cache_hit = true;
        }
        let payload = b.finalize(&sample_response(req.request_id), &outcomes);
        assert_eq!(payload.cache_hit_count, 1);
    }

    #[test]
    fn op_status_counts_aggregates_all_statuses() {
        let mut results = HashMap::new();
        for (i, status) in [
            efe_nano_sdk::OpStatus::Success,
            efe_nano_sdk::OpStatus::Failed,
            efe_nano_sdk::OpStatus::Skipped,
            efe_nano_sdk::OpStatus::Cancelled,
        ]
        .iter()
        .enumerate()
        {
            results.insert(
                format!("o{i}"),
                efe_nano_sdk::OperationResult {
                    op_id: format!("o{i}"),
                    status: *status,
                    output: None,
                    error: None,
                    latency_ms: 0,
                    cost_cents: 0.0,
                },
            );
        }
        let response = efe_nano_sdk::SwarmResponse {
            request_id: Uuid::new_v4(),
            overall_status: efe_nano_sdk::SwarmStatus::PartialFailure,
            results,
            total_latency_ms: 0,
            total_cost_cents: 0.0,
            swarm_envelope_trace_id: None,
        };
        let counts = aggregate_op_statuses_from_response(&response);
        assert_eq!(counts.success, 1);
        assert_eq!(counts.failed, 1);
        assert_eq!(counts.skipped, 1);
        assert_eq!(counts.cancelled, 1);
    }

    #[test]
    fn op_status_pending_counts_as_cancelled_for_envelope() {
        let mut results = HashMap::new();
        results.insert(
            "p".into(),
            efe_nano_sdk::OperationResult {
                op_id: "p".into(),
                status: efe_nano_sdk::OpStatus::Pending,
                output: None,
                error: None,
                latency_ms: 0,
                cost_cents: 0.0,
            },
        );
        let response = efe_nano_sdk::SwarmResponse {
            request_id: Uuid::new_v4(),
            overall_status: efe_nano_sdk::SwarmStatus::PartialFailure,
            results,
            total_latency_ms: 0,
            total_cost_cents: 0.0,
            swarm_envelope_trace_id: None,
        };
        let counts = aggregate_op_statuses_from_response(&response);
        assert_eq!(counts.cancelled, 1);
    }
}
