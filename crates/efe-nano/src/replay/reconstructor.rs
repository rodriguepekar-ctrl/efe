//! SwarmRequest reconstruction from stored chain entries.
//!
//! # The problem
//!
//! Replay needs to re-dispatch a swarm that originally ran days/years
//! ago.  The chain store has the per-op TRACE rows + (optionally) the
//! full INPUT bodies.  The reconstructor weaves these into a fresh
//! `SwarmRequest` that the dispatcher can execute.
//!
//! # Body retrieval — injected via closure
//!
//! Body storage is intentionally NOT a `ChainStore` trait method (cf
//! `audit/chain_store.rs` rationale : production may push bodies to
//! S3 / cold-store while keeping chain rows in Postgres).  The
//! reconstructor takes a `body_getter: impl Fn(&[u8; 32]) -> Option<Value>`
//! closure, which the caller wires to whatever body backend they have :
//!
//! - In-process Sprint 6 ship :
//!   `move |hash| chain_store_inmem.get_input(hash)`
//! - Phase 4 Postgres + S3 :
//!   `move |hash| sqlx::query!("SELECT body FROM input_bodies WHERE hash = $1", hash).fetch_optional(&pool).await?`
//!
//! # Dependency reconstruction
//!
//! Each trace records `parent_trace_id` — the upstream trace that
//! produced the input.  The reconstructor walks this graph and emits
//! `depends_on` as the op_ids of the parent traces.
//!
//! # Limitations
//!
//! - **`parent_trace_id` always `None` (Sprint 5 limitation)** : the
//!   dispatcher's executor doesn't currently propagate the parent
//!   trace_id when running ops (cf `dispatcher/executor.rs:116`).  So
//!   reconstructed `depends_on` is always `vec![]` in Sprint 6 — every
//!   op replays as layer-0 parallel.  This is fine for Mode A (pure
//!   deterministic) and Mode B (seeded local model) where ops are
//!   independent given their literal inputs.  Mode C is `Skipped`
//!   anyway.  Phase 4 / Sprint 7 promotes `parent_trace_id` to
//!   `Vec<TraceId>` to preserve the full DAG.
//!
//! - **`caller_agent_id` always `None` (Sprint 5 limitation)** : not
//!   propagated from `SwarmRequest::caller_agent_id` into per-op
//!   `NanoAgentTrace::caller_agent_id`.  The reconstructor outputs
//!   `None` here.  Replay still works — `caller_agent_id` doesn't
//!   affect determinism.
//!
//! - **Mode C with PII redaction** : original input was redacted before
//!   the LLM saw it.  If `full_input` stored the redacted form, replay
//!   sees the same redaction (correct).  If `full_input` stored the raw
//!   form (audit.include_input=true with redact_pii=false), replay
//!   re-redacts pre-LLM and may produce a different output_hash.  For
//!   strict replay, agents should set `audit.redact_pii=true`.
//!
//! - **OperationInput::Ref / Merge** : the original op may have used
//!   refs to upstream output.  We replay with `Literal { value: stored_input }`
//!   instead — equivalent for deterministic ops since stored_input IS
//!   the value the original Ref resolved to.

use efe_nano_sdk::{
    NanoAgentId, NanoAgentRef, Operation, OperationInput, RetryPolicy, SwarmRequest,
};
use std::collections::HashMap;
use uuid::Uuid;

use crate::audit::chain_store::ChainEntry;
use crate::error::ReplayError;

// ─────────────────────────────────────────────────────────────────────
//  SwarmReconstructor
// ─────────────────────────────────────────────────────────────────────

pub struct SwarmReconstructor;

impl SwarmReconstructor {
    /// Reconstruct a [`SwarmRequest`] from the chain entries of a swarm.
    ///
    /// `entries` must all belong to the same `swarm_request_id` and the
    /// same `tenant_id`.  The function asserts these invariants.
    ///
    /// `body_getter` resolves an `input_hash` to its full body — see the
    /// module-level doc for backend-specific wiring.  Missing bodies
    /// surface as [`ReplayError::MissingFullInput`].
    pub fn reconstruct(
        entries: &[ChainEntry],
        body_getter: impl Fn(&[u8; 32]) -> Option<serde_json::Value>,
    ) -> Result<SwarmRequest, ReplayError> {
        if entries.is_empty() {
            return Err(ReplayError::NotFound(Uuid::nil()));
        }

        let swarm_id = entries[0].trace.swarm_request_id;
        let tenant = entries[0].trace.tenant_id;
        for e in entries {
            if e.trace.swarm_request_id != swarm_id {
                return Err(ReplayError::Internal(format!(
                    "mixed swarm_request_ids : {} vs {swarm_id}",
                    e.trace.swarm_request_id
                )));
            }
            if e.trace.tenant_id != tenant {
                return Err(ReplayError::Internal(format!(
                    "mixed tenants : {} vs {tenant}",
                    e.trace.tenant_id
                )));
            }
        }

        // trace_id → op_id (for parent → depends_on resolution).
        let trace_to_op: HashMap<Uuid, String> = entries
            .iter()
            .map(|e| (e.trace_id(), e.trace.operation_id.clone()))
            .collect();

        let mut operations: Vec<Operation> = Vec::with_capacity(entries.len());
        let mut caller_agent_id: Option<String> = None;

        for entry in entries {
            // Capture caller_agent_id from the first non-null trace
            // (Sprint 5 limit : currently always None, see module doc).
            if caller_agent_id.is_none() {
                caller_agent_id = entry.trace.caller_agent_id.clone();
            }

            // Look up full_input from the body store via the injected getter.
            // Sprint 7 — N6 : full body is keyed by `input_hash_resolved`
            // (the value the runtime saw).  The `input_hash_source` is
            // only used for forensic comparison, not body retrieval.
            let input_value = body_getter(&entry.trace.input_hash_resolved)
                .ok_or_else(|| ReplayError::MissingFullInput(entry.trace_id()))?;

            // Sprint 7 — N1 : reconstruct depends_on from the full
            // parent_trace_ids vec.  Pre-Sprint-7 this was always empty
            // because the dispatcher never populated parent_trace_id ;
            // post-Sprint-7 N1 fix, Ref / Merge ops carry their full
            // ancestry and the DAG round-trips through replay.
            let mut depends_on: Vec<String> = entry
                .trace
                .parent_trace_ids
                .iter()
                .filter_map(|p| trace_to_op.get(&p.0).cloned())
                .collect();
            // Sort for deterministic order (replay must produce the same
            // SwarmRequest each time).
            depends_on.sort();
            depends_on.dedup();

            let agent_id = NanoAgentId::new(entry.trace.nano_agent_id.as_str())
                .map_err(|e| ReplayError::Internal(format!("invalid agent id : {e}")))?;
            let agent_ref = NanoAgentRef::new(agent_id);

            operations.push(Operation {
                op_id: entry.trace.operation_id.clone(),
                agent: agent_ref,
                input: OperationInput::Literal { value: input_value },
                depends_on,
                // Use the original latency as a hint for timeout.
                timeout_ms: entry.trace.latency_ms.map(|l| l.saturating_mul(2).max(1_000)),
                retry: RetryPolicy::default(),
            });
        }

        Ok(SwarmRequest {
            request_id: swarm_id,
            tenant_id: tenant,
            caller_agent_id,
            operations,
            max_parallelism: 100,
            timeout_ms: 60_000,
            idempotency_key: None,
            streaming: false,
            audit_tags: vec!["replay".to_string()],
            // Sprint 7 — N7 : replay never enforces a cost cap (the
            // original swarm already enforced it ; replay is for
            // audit-grade verification, not cost control).
            cost_cap_cents: None,
            parent_envelope_trace_id: None,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::chain_store::ChainEntry;
    use crate::audit::NanoAgentTrace;
    use efe_nano_sdk::{NanoAgentVersion, TenantId, TraceId};

    /// Body getter that always returns `None` — used to assert behaviour
    /// when bodies are missing (e.g. agent.audit.include_input=false).
    fn no_bodies(_hash: &[u8; 32]) -> Option<serde_json::Value> {
        None
    }

    /// Body getter that returns a canned JSON for every hash.
    fn canned_body(_hash: &[u8; 32]) -> Option<serde_json::Value> {
        Some(serde_json::json!({"replay_input": true}))
    }

    fn entry(
        tenant: TenantId,
        swarm: Uuid,
        op_id: &str,
        agent_id: &str,
        parent_trace: Option<Uuid>,
    ) -> ChainEntry {
        let trace_id = Uuid::new_v4();
        ChainEntry {
            trace: NanoAgentTrace {
                trace_id: TraceId(trace_id),
                parent_trace_ids: parent_trace.into_iter().map(TraceId).collect(),
                swarm_request_id: swarm,
                operation_id: op_id.to_string(),
                tenant_id: tenant,
                caller_agent_id: Some("test-caller".into()),
                nano_agent_id: NanoAgentId::new(agent_id).unwrap(),
                nano_agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
                execution_mode: "pure_rust".into(),
                input_hash_source: [0u8; 32],
                input_hash_resolved: [0u8; 32],
                output_hash: Some([1u8; 32]),
                error_kind: None,
                started_at: chrono::Utc::now(),
                completed_at: Some(chrono::Utc::now()),
                latency_ms: Some(10),
                cost_cents: 0.001,
                llm_tokens_used: None,
                full_input: None,
                full_output: None,
                tombstone: None,
                swarm_envelope_trace_id: None,
            },
            chain_parent_id: None,
            payload_hash: [0u8; 32],
            hmac: [0u8; 32],
            inserted_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn reconstruct_empty_returns_not_found() {
        let err = SwarmReconstructor::reconstruct(&[], no_bodies).unwrap_err();
        assert!(matches!(err, ReplayError::NotFound(_)));
    }

    #[test]
    fn reconstruct_mixed_swarm_ids_errors() {
        let tenant = TenantId::system();
        let e1 = entry(tenant, Uuid::new_v4(), "a", "agent_x", None);
        let e2 = entry(tenant, Uuid::new_v4(), "b", "agent_y", None);
        let err = SwarmReconstructor::reconstruct(&[e1, e2], no_bodies).unwrap_err();
        match err {
            ReplayError::Internal(msg) => assert!(msg.contains("swarm_request_ids")),
            other => panic!("expected Internal, got {other:?}"),
        }
    }

    #[test]
    fn reconstruct_mixed_tenants_errors() {
        let swarm = Uuid::new_v4();
        let e1 = entry(TenantId::new(), swarm, "a", "agent_x", None);
        let e2 = entry(TenantId::new(), swarm, "b", "agent_y", None);
        let err = SwarmReconstructor::reconstruct(&[e1, e2], no_bodies).unwrap_err();
        match err {
            ReplayError::Internal(msg) => assert!(msg.contains("tenants")),
            other => panic!("expected Internal, got {other:?}"),
        }
    }

    #[test]
    fn reconstruct_missing_body_surfaces_missing_full_input() {
        // Sprint 6 contract : every op needs its full_input body to be
        // replayable.  When the getter returns None, we must surface a
        // MissingFullInput pointing at the offending trace_id.
        let tenant = TenantId::system();
        let swarm = Uuid::new_v4();
        let e1 = entry(tenant, swarm, "a", "agent_x", None);
        let expected = e1.trace_id();
        let err = SwarmReconstructor::reconstruct(&[e1], no_bodies).unwrap_err();
        match err {
            ReplayError::MissingFullInput(id) => assert_eq!(id, expected),
            other => panic!("expected MissingFullInput, got {other:?}"),
        }
    }

    #[test]
    fn reconstruct_with_bodies_builds_swarm_request() {
        let tenant = TenantId::system();
        let swarm = Uuid::new_v4();
        let e1 = entry(tenant, swarm, "alpha", "json_validate", None);
        let e2 = entry(tenant, swarm, "beta", "regex_extract", None);
        let req = SwarmReconstructor::reconstruct(&[e1, e2], canned_body).unwrap();
        assert_eq!(req.request_id, swarm);
        assert_eq!(req.tenant_id, tenant);
        assert_eq!(req.operations.len(), 2);
        assert_eq!(req.operations[0].op_id, "alpha");
        assert_eq!(req.operations[1].op_id, "beta");
        // depends_on always empty (Sprint 5 limit : parent_trace_id is None).
        assert!(req.operations[0].depends_on.is_empty());
        assert!(req.operations[1].depends_on.is_empty());
        // audit_tags marked as replay.
        assert!(req.audit_tags.contains(&"replay".to_string()));
        // Every op input is the canned literal.
        for op in &req.operations {
            match &op.input {
                efe_nano_sdk::OperationInput::Literal { value } => {
                    assert_eq!(value["replay_input"], true);
                }
                other => panic!("expected Literal, got {other:?}"),
            }
        }
    }
}
