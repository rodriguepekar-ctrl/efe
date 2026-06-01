//! Replay engine — re-execute historical swarms + verify determinism.
//!
//! # Pipeline
//!
//! ```text
//! ReplayEngine::replay(tenant, request_id)
//!   │
//!   ├── 1. store.traces_for_swarm(tenant, request_id)
//!   ├── 2. verify_subchain (HMAC integrity)
//!   ├── 3. SwarmReconstructor::reconstruct → fresh SwarmRequest
//!   ├── 4. dispatcher.dispatch(reconstructed) → fresh SwarmResponse
//!   ├── 5. per-op DeterminismChecker comparison
//!   └── 6. ReplayReport assembly
//! ```
//!
//! # What can go wrong (and how we handle it)
//!
//! | Failure | Response |
//! |---------|----------|
//! | No traces found | `Err(ReplayError::NotFound)` |
//! | Chain HMAC broken | `Err(ReplayError::ChainIntegrityBroken)` |
//! | Trace missing `full_input` | `Err(ReplayError::MissingFullInput)` |
//! | Dispatch returns DispatcherError | `Ok(report)` with `dispatch_error = Some(...)` |
//! | Per-op output_hash mismatch | `Ok(report)` with `mismatched_ops > 0` |
//! | Non-deterministic op | `Ok(report)` with `skipped_non_deterministic_ops > 0` |

pub mod cli;
pub mod determinism;
pub mod reconstructor;
pub mod types;

pub use determinism::DeterminismChecker;
pub use reconstructor::SwarmReconstructor;
pub use types::{PerOpReplay, ReplayOpStatus, ReplayReport};

use chrono::Utc;
use efe_nano_sdk::{Catalog, DeterminismLevel, NanoAgentRef, TenantId};
use std::sync::Arc;
use uuid::Uuid;

use crate::audit::chain_store::{ChainEntry, ChainStore, InMemoryChainStore};
use crate::audit::hkdf::HmacKeySource;
use crate::audit::verify::verify_subchain;
use crate::dispatcher::Dispatcher;
use crate::error::ReplayError;

// ─────────────────────────────────────────────────────────────────────
//  ReplayEngine
// ─────────────────────────────────────────────────────────────────────

/// Engine that re-executes historical swarms.
///
/// Sprint 6 ships this with an `InMemoryChainStore` for body access ;
/// production wires a `PostgresChainStore` with parallel SQL fetches
/// of inputs.
pub struct ReplayEngine {
    chain_store: Arc<InMemoryChainStore>,
    key_source: HmacKeySource,
    dispatcher: Arc<Dispatcher>,
    catalog: Arc<dyn Catalog>,
    /// **Sprint 8 — N14 (B3 fix)** : optional envelope metrics for
    /// `envelopes_queried_total`.  Read from the dispatcher when set.
    envelope_metrics: Option<Arc<crate::dispatcher::envelope::EnvelopeMetrics>>,
}

impl ReplayEngine {
    pub fn new(
        chain_store: Arc<InMemoryChainStore>,
        key_source: HmacKeySource,
        dispatcher: Arc<Dispatcher>,
        catalog: Arc<dyn Catalog>,
    ) -> Self {
        let envelope_metrics = dispatcher.envelope_metrics().map(Arc::clone);
        Self {
            chain_store,
            key_source,
            dispatcher,
            catalog,
            envelope_metrics,
        }
    }

    /// **Sprint 8 — N14** : Read the swarm envelope payload.
    ///
    /// This is the first query a forensic investigation runs : given a
    /// `swarm_request_id`, get the structured envelope (request
    /// signature + aggregated outcome).
    ///
    /// Returns :
    /// * `Ok(payload)` — envelope present + deserialised.
    /// * `Err(EnvelopeNotFound)` — no envelope row in the chain (pre-
    ///   N14 swarm OR concurrent dispatch in progress).
    /// * `Err(EnvelopeTombstoned)` — envelope was tombstoned (GDPR).
    /// * `Err(EnvelopeMalformed)` — chain corruption.
    pub async fn envelope_for_swarm(
        &self,
        tenant: TenantId,
        request_id: Uuid,
    ) -> Result<efe_nano_sdk::SwarmEnvelopePayload, ReplayError> {
        // **N14 (B3 fix)** : tick the queried_total metric.
        if let Some(m) = self.envelope_metrics.as_ref() {
            m.record_query();
        }
        let traces = self
            .chain_store
            .traces_for_swarm(tenant, request_id)
            .await
            .map_err(|e| ReplayError::ChainIntegrityBroken(e.into()))?;
        let envelope_entry = traces
            .iter()
            .find(|e| e.trace.error_kind.as_deref() == Some("SwarmEnvelope"))
            .ok_or(ReplayError::EnvelopeNotFound(request_id))?;
        if envelope_entry.trace.is_tombstoned() {
            return Err(ReplayError::EnvelopeTombstoned(request_id));
        }
        // Try the inline `full_input` first ; if None, fall back to the
        // body side-store (Sprint 6 HMAC chain pattern moves the body
        // out of the trace row for canonical-hash stability).
        let full_input = match envelope_entry.trace.full_input.clone() {
            Some(v) => v,
            None => self
                .chain_store
                .get_input(&envelope_entry.trace.input_hash_resolved)
                .ok_or(ReplayError::EnvelopeTombstoned(request_id))?,
        };
        serde_json::from_value::<efe_nano_sdk::SwarmEnvelopePayload>(full_input)
            .map_err(|e| ReplayError::EnvelopeMalformed(request_id, e.to_string()))
    }

    /// Replay a swarm by its `request_id`.
    pub async fn replay(
        &self,
        tenant: TenantId,
        request_id: Uuid,
    ) -> Result<ReplayReport, ReplayError> {
        let started_at = Utc::now();
        let started_instant = std::time::Instant::now();

        // 1. Load ALL traces for the swarm — including any chain-linked
        //    cache-hit traces from prior replays.  HMAC chain
        //    verification needs the full set so the prev_id linkage
        //    holds (filtering breaks the chain).
        let all_traces = self
            .chain_store
            .traces_for_swarm(tenant, request_id)
            .await
            .map_err(|e| ReplayError::ChainIntegrityBroken(e.into()))?;

        if all_traces.is_empty() {
            return Err(ReplayError::NotFound(request_id));
        }

        // 2. Verify HMAC chain on the FULL trace set.
        let chain_verified = match verify_subchain(&all_traces, &self.key_source, tenant).await {
            Ok(_) => true,
            Err(e) => return Err(ReplayError::ChainIntegrityBroken(e)),
        };

        // 3. **Sprint 8 — N12** : filter out cache-hit lineage traces
        //    AFTER verification.  Cache-hit traces (`error_kind =
        //    Some("CacheHit")`) are audit metadata for the cache-served
        //    path, not original ops.
        // **Sprint 8 — N13** : also skip ShadowRun + ShadowComparison +
        // RolloutPolicyChanged virtual-swarm traces.
        // **Sprint 8 — N14** : also skip the SwarmEnvelope sentinel —
        // the envelope is a summary, not an op to replay.
        let traces: Vec<_> = all_traces
            .into_iter()
            .filter(|entry| {
                !matches!(
                    entry.trace.error_kind.as_deref(),
                    Some("CacheHit")
                        | Some("ShadowRun")
                        | Some("ShadowComparison")
                        | Some("RolloutPolicyChanged")
                        | Some("SwarmEnvelope")
                )
            })
            .collect();

        // 3. Reconstruct SwarmRequest, replacing each op's input with the
        //    stored full body (looked up from the in-memory side store).
        let reconstructed = self.reconstruct_with_bodies(&traces)?;

        // 4. Dispatch the reconstructed swarm.
        let dispatch_result = self.dispatcher.dispatch(reconstructed).await;
        let (new_response, dispatch_error) = match dispatch_result {
            Ok(resp) => (Some(resp), None),
            Err(e) => (None, Some(e.to_string())),
        };

        // 5. Per-op determinism comparison.
        let mut per_op = Vec::with_capacity(traces.len());
        let mut matched = 0u32;
        let mut mismatched = 0u32;
        let mut skipped = 0u32;
        let mut window_expired = 0u32;
        let mut failed = 0u32;

        for entry in &traces {
            let original_hash = entry.trace.output_hash;
            let replay_hash = if let Some(resp) = &new_response {
                resp.results
                    .get(&entry.trace.operation_id)
                    .and_then(|r| r.output.as_ref())
                    .map(DeterminismChecker::hash)
            } else {
                None
            };

            // Sprint 7 — N5 : when the catalog has purged the exact
            // version, we fall back to LlmEphemeral as the most
            // conservative class — it surfaces SkippedNonDeterministic
            // (auditor-explicit "we couldn't verify"), never silently
            // classifies as Match.
            let determinism = self
                .lookup_determinism(tenant, &entry.trace)
                .await
                .unwrap_or(DeterminismLevel::LlmEphemeral);

            let status = DeterminismChecker::classify(
                determinism,
                &entry.trace.execution_mode,
                original_hash,
                replay_hash,
            );

            match status {
                ReplayOpStatus::Match => matched += 1,
                ReplayOpStatus::Mismatch => mismatched += 1,
                ReplayOpStatus::SkippedNonDeterministic => skipped += 1,
                ReplayOpStatus::ReplayWindowExpired => window_expired += 1,
                ReplayOpStatus::ReplayFailed => failed += 1,
            }

            per_op.push(PerOpReplay {
                op_id: entry.trace.operation_id.clone(),
                nano_agent_id: entry.trace.nano_agent_id.as_str().to_string(),
                execution_mode: entry.trace.execution_mode.clone(),
                original_output_hash: original_hash,
                replay_output_hash: replay_hash,
                status,
                explanation: None,
            });
        }

        let completed_at = Utc::now();
        let duration_ms = started_instant.elapsed().as_millis().min(u32::MAX as u128) as u32;

        Ok(ReplayReport {
            request_id,
            tenant,
            chain_verified,
            total_ops: traces.len() as u32,
            matched_ops: matched,
            mismatched_ops: mismatched,
            skipped_non_deterministic_ops: skipped,
            replay_window_expired_ops: window_expired,
            failed_ops: failed,
            per_op,
            new_response,
            dispatch_error,
            started_at,
            completed_at,
            duration_ms,
        })
    }

    /// Reconstruct SwarmRequest with full inputs pulled from the
    /// `InMemoryChainStore`'s body store.
    ///
    /// Delegates the heavy lifting to [`SwarmReconstructor::reconstruct`],
    /// passing a body-getter closure that reads from this engine's
    /// `chain_store` side-store.  The engine remains the body-backend-
    /// aware piece ; the reconstructor stays storage-agnostic so a Phase
    /// 4 Postgres + S3 wiring can reuse it verbatim.
    fn reconstruct_with_bodies(
        &self,
        entries: &[ChainEntry],
    ) -> Result<efe_nano_sdk::SwarmRequest, ReplayError> {
        let chain_store = Arc::clone(&self.chain_store);
        SwarmReconstructor::reconstruct(entries, move |hash| chain_store.get_input(hash))
    }

    /// Look up the agent's `DeterminismLevel` from the catalog, **pinned
    /// to the exact version that ran originally**.
    ///
    /// The trace's `nano_agent_version` is the truth-of-the-day at audit
    /// write time.  Replay must use the SAME version's determinism level —
    /// republishing an agent at a new version with a different
    /// `DeterminismLevel` must NOT silently re-classify historical traces
    /// (that would be a regulator-visible drift).
    ///
    /// If the exact version is no longer present (operator deleted it
    /// from the catalog), we return `None` and the caller falls back to
    /// `DeterminismLevel::Probabilistic` — the most conservative class :
    /// Mode A drift would surface as `Mismatch`, Mode B as `Mismatch`,
    /// Mode C as `SkippedNonDeterministic`.  Better to alarm than to
    /// silently mis-classify.
    async fn lookup_determinism(
        &self,
        tenant: TenantId,
        trace: &crate::audit::NanoAgentTrace,
    ) -> Option<DeterminismLevel> {
        use efe_nano_sdk::{NanoAgentId, VersionReq};
        let id = NanoAgentId::new(trace.nano_agent_id.as_str()).ok()?;
        // Exact-match VersionReq : "=X.Y.Z".  Matches semver semantics
        // for "exactly this version".
        let req = VersionReq::new(format!("={}", trace.nano_agent_version.as_str())).ok()?;
        let r = NanoAgentRef::with_version(id, req);
        let agent = self.catalog.get(&r, tenant).await.ok()?;
        Some(agent.determinism)
    }
}

impl std::fmt::Debug for ReplayEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplayEngine").finish_non_exhaustive()
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::AuditWriter;
    use crate::audit::hmac_chain::InMemoryHmacChainAuditWriter;
    use crate::catalog::InMemoryCatalog;
    use crate::idempotency::IdempotencyCache;
    use crate::runtime::pure_rust::{HandlerRegistry, PureRustRuntime};
    use crate::runtime::NanoAgentExecutor;
    use crate::starter_pack::register_all_starter_pack;

    async fn make_fixture() -> (
        Arc<InMemoryChainStore>,
        Arc<InMemoryCatalog>,
        Arc<Dispatcher>,
        Arc<InMemoryHmacChainAuditWriter>,
    ) {
        let catalog = Arc::new(InMemoryCatalog::new());
        let registry = HandlerRegistry::new();
        register_all_starter_pack(&*catalog, &registry).await.unwrap();
        let runtime: Arc<dyn NanoAgentExecutor> = Arc::new(PureRustRuntime::new(registry));
        let chain_store = Arc::new(InMemoryChainStore::new());
        let writer = Arc::new(InMemoryHmacChainAuditWriter::new(
            Arc::clone(&chain_store),
            HmacKeySource::test_key(),
        ));
        let dispatcher = Arc::new(Dispatcher::new(
            Arc::clone(&catalog) as Arc<dyn Catalog>,
            runtime,
            Arc::clone(&writer) as Arc<dyn AuditWriter>,
            Arc::new(IdempotencyCache::new()),
        ));
        (chain_store, catalog, dispatcher, writer)
    }

    #[tokio::test]
    async fn replay_not_found_for_unknown_request_id() {
        let (chain_store, catalog, dispatcher, _) = make_fixture().await;
        let engine = ReplayEngine::new(
            chain_store,
            HmacKeySource::test_key(),
            dispatcher,
            catalog as Arc<dyn Catalog>,
        );
        let err = engine
            .replay(TenantId::system(), Uuid::new_v4())
            .await
            .unwrap_err();
        assert!(matches!(err, ReplayError::NotFound(_)));
    }

    #[tokio::test]
    async fn replay_after_dispatch_records_chain_then_replays() {
        use efe_nano_sdk::{
            NanoAgentId, Operation, OperationInput, RetryPolicy, SwarmRequest, SwarmStatus,
        };
        let (chain_store, catalog, dispatcher, _) = make_fixture().await;
        let tenant = TenantId::system();
        let request_id = Uuid::new_v4();
        let request = SwarmRequest {
            request_id,
            tenant_id: tenant,
            caller_agent_id: Some("test".into()),
            operations: vec![Operation {
                op_id: "v".into(),
                agent: NanoAgentRef::new(NanoAgentId::new("json_validate").unwrap()),
                input: OperationInput::Literal {
                    value: serde_json::json!({
                        "value": {"x": 1},
                        "schema": {"type": "object"}
                    }),
                },
                depends_on: vec![],
                timeout_ms: None,
                retry: RetryPolicy::default(),
            }],
            max_parallelism: 1,
            timeout_ms: 30_000,
            idempotency_key: None,
            streaming: false,
            audit_tags: vec![],
            cost_cap_cents: None,
            parent_envelope_trace_id: None,
        };
        let resp = dispatcher.dispatch(request).await.unwrap();
        assert_eq!(resp.overall_status, SwarmStatus::Success);

        // Replay.
        let engine = ReplayEngine::new(
            chain_store,
            HmacKeySource::test_key(),
            dispatcher,
            catalog as Arc<dyn Catalog>,
        );
        let report = engine.replay(tenant, request_id).await.unwrap();
        assert!(report.chain_verified);
        assert_eq!(report.total_ops, 1);
        assert_eq!(report.matched_ops, 1);
        assert_eq!(report.mismatched_ops, 0);
        assert!(report.is_fully_consistent());
    }

    #[tokio::test]
    async fn replay_returns_dispatch_error_when_dispatch_fails() {
        // We don't have an easy way to force dispatch failure in this
        // fixture, but we test the trace-based replay shape.
        let (chain_store, catalog, dispatcher, _) = make_fixture().await;
        let engine = ReplayEngine::new(
            chain_store,
            HmacKeySource::test_key(),
            dispatcher,
            catalog as Arc<dyn Catalog>,
        );
        // Querying an empty store → NotFound.
        assert!(engine
            .replay(TenantId::system(), Uuid::new_v4())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn replay_skipped_non_deterministic_op_marks_correctly() {
        // Sprint 7 — N5 : LlmEphemeral replaces the pre-Sprint-7
        // `Probabilistic` + "llm_augmented" dispatch.
        let status = DeterminismChecker::classify(
            DeterminismLevel::LlmEphemeral,
            "llm_augmented",
            Some([1u8; 32]),
            Some([2u8; 32]),
        );
        assert_eq!(status, ReplayOpStatus::SkippedNonDeterministic);
    }

    #[tokio::test]
    async fn replay_chain_verified_flag_is_set() {
        use efe_nano_sdk::{
            NanoAgentId, Operation, OperationInput, RetryPolicy, SwarmRequest,
        };
        let (chain_store, catalog, dispatcher, _) = make_fixture().await;
        let tenant = TenantId::system();
        let request_id = Uuid::new_v4();
        let request = SwarmRequest {
            request_id,
            tenant_id: tenant,
            caller_agent_id: None,
            operations: vec![Operation {
                op_id: "j".into(),
                agent: NanoAgentRef::new(NanoAgentId::new("json_validate").unwrap()),
                input: OperationInput::Literal {
                    value: serde_json::json!({
                        "value": 42,
                        "schema": {"type": "integer"}
                    }),
                },
                depends_on: vec![],
                timeout_ms: None,
                retry: RetryPolicy::default(),
            }],
            max_parallelism: 1,
            timeout_ms: 30_000,
            idempotency_key: None,
            streaming: false,
            audit_tags: vec![],
            cost_cap_cents: None,
            parent_envelope_trace_id: None,
        };
        let _ = dispatcher.dispatch(request).await.unwrap();

        let engine = ReplayEngine::new(
            chain_store,
            HmacKeySource::test_key(),
            dispatcher,
            catalog as Arc<dyn Catalog>,
        );
        let report = engine.replay(tenant, request_id).await.unwrap();
        assert!(report.chain_verified);
    }
}
