// SPDX-License-Identifier: UNLICENSED
//! **Sprint 9 — N17** : Provenance store — trait + in-memory impl.
//!
//! Architecture § 7.6.

use std::collections::HashMap;

use async_trait::async_trait;
use dashmap::DashMap;
use efe_nano_sdk::{
    LlmProvenanceRecord, ModelIdentity, ProvenanceId, TenantId,
};
use thiserror::Error;
use uuid::Uuid;

use crate::query::{CostSummary, DateRange, ModelCostRow, ModelHashMismatchEvent};

// ─────────────────────────────────────────────────────────────────────
//  Error taxonomy
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N17** : closed taxonomy of store errors.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ProvenanceStoreError {
    #[error("provenance not found for trace {trace_id}")]
    NotFound { trace_id: Uuid },

    #[error("cross-tenant access : query tenant {query_tenant}, stored {stored_tenant}")]
    CrossTenant {
        query_tenant: TenantId,
        stored_tenant: TenantId,
    },

    /// **Post-audit fix — I10 immutability** : attempting to insert
    /// a record whose `trace_id` already exists in the store.
    /// Provenance traces are READ-ONLY post-write ; the only
    /// acceptable write is the first.
    #[error("provenance for trace {trace_id} already exists (I10 read-only)")]
    AlreadyExists { trace_id: Uuid },

    #[error("backend error : {message}")]
    Backend { message: String },
}

// ─────────────────────────────────────────────────────────────────────
//  Trait
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N17** : persistent provenance store abstraction.
#[async_trait]
pub trait LlmProvenanceStore: Send + Sync + 'static {
    /// **N17** : Insert a fresh provenance record.
    async fn insert(&self, record: LlmProvenanceRecord) -> Result<(), ProvenanceStoreError>;

    /// **N17** : Fetch a record by trace_id.  Enforces tenant scope.
    async fn get_by_trace(
        &self,
        trace_id: Uuid,
        tenant: TenantId,
    ) -> Result<LlmProvenanceRecord, ProvenanceStoreError>;

    /// **N17** : Fetch a record by provenance_id.  Enforces tenant scope.
    async fn get_by_provenance(
        &self,
        provenance_id: ProvenanceId,
        tenant: TenantId,
    ) -> Result<LlmProvenanceRecord, ProvenanceStoreError>;

    /// **N17** : List records for a swarm.  Tenant-scoped.
    async fn list_by_swarm(
        &self,
        swarm_request_id: Uuid,
        tenant: TenantId,
    ) -> Result<Vec<LlmProvenanceRecord>, ProvenanceStoreError>;

    /// **N17** : List records for a model identity within a date window.
    async fn list_by_model(
        &self,
        model: &ModelIdentity,
        tenant: TenantId,
        window: DateRange,
    ) -> Result<Vec<LlmProvenanceRecord>, ProvenanceStoreError>;

    /// **N17** : Aggregate cost summary for a tenant in a window.
    async fn cost_summary(
        &self,
        tenant: TenantId,
        window: DateRange,
    ) -> Result<CostSummary, ProvenanceStoreError>;

    /// **N17** : List model-hash-mismatch events for a tenant in a window.
    async fn list_drift_events(
        &self,
        tenant: TenantId,
        window: DateRange,
    ) -> Result<Vec<ModelHashMismatchEvent>, ProvenanceStoreError>;

    /// **N17** : Record a drift event (called by collector on F3
    /// detection).
    async fn record_drift_event(
        &self,
        event: ModelHashMismatchEvent,
    ) -> Result<(), ProvenanceStoreError>;

    /// **Sprint 19 — N27** : patch the four streaming fields onto
    /// an already-written record.  Returns the patched record.
    ///
    /// Default impl returns `Ok(None)` for stores that don't
    /// support in-place mutation (e.g. an append-only ledger
    /// where the streaming fields would land in a separate
    /// table).  The [`InMemoryLlmProvenanceStore`] overrides
    /// to update the record in `by_trace`.
    async fn patch_streaming_fields(
        &self,
        _provenance_id: ProvenanceId,
        _streaming_mode: bool,
        _retry_count: u8,
        _retry_outcome: efe_nano_sdk::RetryOutcome,
        _time_to_first_token_ms: Option<u32>,
    ) -> Result<Option<LlmProvenanceRecord>, ProvenanceStoreError> {
        Ok(None)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  InMemoryLlmProvenanceStore
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N17** : in-memory implementation.  Production deploys
/// `PostgresLlmProvenanceStore` (Sprint 10).
#[derive(Default)]
pub struct InMemoryLlmProvenanceStore {
    by_trace: DashMap<Uuid, LlmProvenanceRecord>,
    by_provenance: DashMap<ProvenanceId, Uuid>,
    drift_events: DashMap<(TenantId, Uuid), ModelHashMismatchEvent>,
}

impl InMemoryLlmProvenanceStore {
    /// **N17** : Construct an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// **N17** : Record count.
    pub fn len(&self) -> usize {
        self.by_trace.len()
    }

    /// **N17** : True if empty.
    pub fn is_empty(&self) -> bool {
        self.by_trace.is_empty()
    }

    /// **N17** : Drift event count.
    pub fn drift_event_count(&self) -> usize {
        self.drift_events.len()
    }

    fn check_tenant(
        stored: TenantId,
        query: TenantId,
    ) -> Result<(), ProvenanceStoreError> {
        if stored == query {
            Ok(())
        } else {
            Err(ProvenanceStoreError::CrossTenant {
                query_tenant: query,
                stored_tenant: stored,
            })
        }
    }
}

impl std::fmt::Debug for InMemoryLlmProvenanceStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryLlmProvenanceStore")
            .field("records", &self.by_trace.len())
            .field("drift_events", &self.drift_events.len())
            .finish()
    }
}

#[async_trait]
impl LlmProvenanceStore for InMemoryLlmProvenanceStore {
    async fn insert(
        &self,
        record: LlmProvenanceRecord,
    ) -> Result<(), ProvenanceStoreError> {
        // **Post-audit fix — bug D + I10 immutability** : reject
        // duplicate `trace_id` insertions.  Provenance traces are
        // write-once.  DashMap's `entry().or_insert(...)` is the
        // atomic check-and-insert primitive.
        let trace_id = record.trace_id;
        match self.by_trace.entry(trace_id) {
            dashmap::Entry::Occupied(_) => {
                Err(ProvenanceStoreError::AlreadyExists { trace_id })
            }
            dashmap::Entry::Vacant(slot) => {
                self.by_provenance.insert(record.provenance_id, trace_id);
                slot.insert(record);
                Ok(())
            }
        }
    }

    async fn get_by_trace(
        &self,
        trace_id: Uuid,
        tenant: TenantId,
    ) -> Result<LlmProvenanceRecord, ProvenanceStoreError> {
        let r = self
            .by_trace
            .get(&trace_id)
            .ok_or(ProvenanceStoreError::NotFound { trace_id })?;
        Self::check_tenant(r.tenant_id, tenant)?;
        Ok(r.clone())
    }

    async fn get_by_provenance(
        &self,
        provenance_id: ProvenanceId,
        tenant: TenantId,
    ) -> Result<LlmProvenanceRecord, ProvenanceStoreError> {
        let trace_id = self
            .by_provenance
            .get(&provenance_id)
            .map(|r| *r.value())
            .ok_or(ProvenanceStoreError::NotFound {
                trace_id: provenance_id.as_uuid(),
            })?;
        self.get_by_trace(trace_id, tenant).await
    }

    async fn list_by_swarm(
        &self,
        swarm_request_id: Uuid,
        tenant: TenantId,
    ) -> Result<Vec<LlmProvenanceRecord>, ProvenanceStoreError> {
        let out: Vec<LlmProvenanceRecord> = self
            .by_trace
            .iter()
            .filter(|r| {
                r.value().swarm_request_id == swarm_request_id
                    && r.value().tenant_id == tenant
            })
            .map(|r| r.value().clone())
            .collect();
        Ok(out)
    }

    async fn list_by_model(
        &self,
        model: &ModelIdentity,
        tenant: TenantId,
        window: DateRange,
    ) -> Result<Vec<LlmProvenanceRecord>, ProvenanceStoreError> {
        let out: Vec<LlmProvenanceRecord> = self
            .by_trace
            .iter()
            .filter(|r| {
                let rec = r.value();
                rec.tenant_id == tenant
                    && model_identity_match(&rec.model_identity, model)
                    && window.contains(rec.recorded_at)
            })
            .map(|r| r.value().clone())
            .collect();
        Ok(out)
    }

    async fn cost_summary(
        &self,
        tenant: TenantId,
        window: DateRange,
    ) -> Result<CostSummary, ProvenanceStoreError> {
        let mut rows: HashMap<ModelIdentity, ModelCostRow> = HashMap::new();
        let mut total_cents = 0.0;
        let mut total_client_tokens: u64 = 0;
        let mut total_provider_tokens: Option<u64> = Some(0);
        let mut call_count = 0u64;

        for r in self.by_trace.iter() {
            let rec = r.value();
            if rec.tenant_id != tenant || !window.contains(rec.recorded_at) {
                continue;
            }
            call_count = call_count.saturating_add(1);
            total_cents += rec.cost_accounting.cost_cents;
            total_client_tokens = total_client_tokens.saturating_add(u64::from(
                rec.token_usage.client_estimated_input + rec.token_usage.client_estimated_output,
            ));
            let provider_tokens = rec.token_usage.provider_reported_input.and_then(|i| {
                rec.token_usage.provider_reported_output.map(|o| u64::from(i + o))
            });
            match provider_tokens {
                Some(p) => {
                    if let Some(t) = total_provider_tokens.as_mut() {
                        *t = t.saturating_add(p);
                    }
                }
                None => {
                    total_provider_tokens = None;
                }
            }
            let row =
                rows.entry(rec.model_identity.clone())
                    .or_insert_with(|| ModelCostRow {
                        model_identity: rec.model_identity.clone(),
                        cost_cents: 0.0,
                        call_count: 0,
                        client_estimated_tokens: 0,
                        provider_reported_tokens: Some(0),
                    });
            row.cost_cents += rec.cost_accounting.cost_cents;
            row.call_count = row.call_count.saturating_add(1);
            row.client_estimated_tokens = row.client_estimated_tokens.saturating_add(u64::from(
                rec.token_usage.client_estimated_input + rec.token_usage.client_estimated_output,
            ));
            row.provider_reported_tokens = match (row.provider_reported_tokens, provider_tokens) {
                (Some(a), Some(b)) => Some(a.saturating_add(b)),
                _ => None,
            };
        }

        let drift_ratio = total_provider_tokens.and_then(|provider| {
            if total_client_tokens == 0 {
                None
            } else {
                let drift = (provider as f64) - (total_client_tokens as f64);
                Some(drift / total_client_tokens as f64)
            }
        });

        Ok(CostSummary {
            by_model: rows.into_values().collect(),
            total_cents,
            total_client_estimated_tokens: total_client_tokens,
            total_provider_reported_tokens: total_provider_tokens,
            drift_ratio,
            call_count,
        })
    }

    async fn list_drift_events(
        &self,
        tenant: TenantId,
        window: DateRange,
    ) -> Result<Vec<ModelHashMismatchEvent>, ProvenanceStoreError> {
        let out: Vec<ModelHashMismatchEvent> = self
            .drift_events
            .iter()
            .filter(|r| {
                r.key().0 == tenant && window.contains(r.value().detected_at)
            })
            .map(|r| r.value().clone())
            .collect();
        Ok(out)
    }

    async fn record_drift_event(
        &self,
        event: ModelHashMismatchEvent,
    ) -> Result<(), ProvenanceStoreError> {
        self.drift_events
            .insert((event.tenant_id, event.event_id), event);
        Ok(())
    }

    /// **Sprint 19 — N27** : in-place patch of the four streaming
    /// fields on a record already in the store.  We locate the
    /// record via the `provenance_id → trace_id` map, mutate the
    /// entry in `by_trace`, and return the updated record.
    ///
    /// **Atomicity** : `DashMap::get_mut` holds a write lock for
    /// the entry, so the patch is atomic w.r.t. concurrent reads
    /// (per-key locking).  This satisfies G14 "one stream → one
    /// record" even under concurrent dispatch.
    async fn patch_streaming_fields(
        &self,
        provenance_id: ProvenanceId,
        streaming_mode: bool,
        retry_count: u8,
        retry_outcome: efe_nano_sdk::RetryOutcome,
        time_to_first_token_ms: Option<u32>,
    ) -> Result<Option<LlmProvenanceRecord>, ProvenanceStoreError> {
        let trace_id = self
            .by_provenance
            .get(&provenance_id)
            .map(|r| *r.value())
            .ok_or(ProvenanceStoreError::NotFound {
                trace_id: Uuid::nil(),
            })?;
        if let Some(mut entry) = self.by_trace.get_mut(&trace_id) {
            entry.streaming_mode = streaming_mode;
            entry.retry_count = retry_count;
            entry.retry_outcome = retry_outcome;
            entry.time_to_first_token_ms = time_to_first_token_ms;
            Ok(Some(entry.clone()))
        } else {
            Err(ProvenanceStoreError::NotFound { trace_id })
        }
    }
}

/// **N17** : Equality for `ModelIdentity` for query purposes.  Same
/// `(provider, model_id, requested_version)` tuple — `served_version`
/// + `content_hash` ignored.
fn model_identity_match(a: &ModelIdentity, b: &ModelIdentity) -> bool {
    a.provider == b.provider
        && a.model_id == b.model_id
        && a.requested_version == b.requested_version
}

// ─────────────────────────────────────────────────────────────────────
//  Re-exports for the trait + impl bundle
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::{
        Blake3Hash, CostAccounting, FinishReason, NanoAgentId, NanoAgentVersion, PiiStrategy,
        ProvenanceId, SamplingParams, TokenUsage,
    };
    use std::time::{Duration, SystemTime};

    fn record(tenant: TenantId, swarm: Uuid, model: ModelIdentity) -> LlmProvenanceRecord {
        LlmProvenanceRecord {
            provenance_id: ProvenanceId::new(),
            trace_id: Uuid::new_v4(),
            swarm_request_id: swarm,
            op_id: "op".into(),
            tenant_id: tenant,
            agent_id: NanoAgentId::new("agent").unwrap(),
            agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
            model_identity: model,
            model_hash_verified: false,
            prompt_template_id: "t".into(),
            prompt_template_version: "1.0.0".into(),
            prompt_inputs_hash: Blake3Hash::ZERO,
            system_prompt_hash: Blake3Hash::ZERO,
            user_prompt_hash: Blake3Hash::ZERO,
            final_prompt_hash: Blake3Hash::ZERO,
            sampling: SamplingParams::default(),
            token_usage: TokenUsage {
                client_estimated_input: 100,
                client_estimated_output: 50,
                provider_reported_input: Some(110),
                provider_reported_output: Some(45),
                is_estimated: false,
            },
            cost_accounting: CostAccounting {
                cost_cents: 5.0,
                prices_table_version: 1,
                provider_quoted_cost_cents: None,
                overflow_capped: false,
            },
            provider_request_id: Some("req_x".into()),
            provider_response_time_ms: 100,
            provider_finish_reason: FinishReason::Stop,
            provider_region: None,
            publisher_id: None,
            publisher_kyc_tier: None,
            gold_case_id: None,
            provider_metadata: HashMap::new(),
            output_hash: Some(Blake3Hash::from_bytes([1u8; 32])),
            completeness: efe_nano_sdk::Completeness::Complete,
            safety_flags: vec![],
            tool_calls: vec![],
            pii_strategy_applied: PiiStrategy::Strict,
            redaction_token_set: vec![],
            replay_of: None,
            cache_hit: None,
            recorded_at: SystemTime::now(),
            // **Sprint 19 — N27** : buffered fixture, no streaming.
            streaming_mode: false,
            retry_count: 0,
            retry_outcome: efe_nano_sdk::RetryOutcome::NoRetry,
            time_to_first_token_ms: None,
            compute_attestation: efe_nano_sdk::ComputeAttestation::default(),
        }
    }

    #[tokio::test]
    async fn insert_then_get_by_trace() {
        let s = InMemoryLlmProvenanceStore::new();
        let tenant = TenantId::new();
        let r = record(tenant, Uuid::new_v4(), ModelIdentity::hosted("p", "m", "v"));
        let trace_id = r.trace_id;
        s.insert(r.clone()).await.unwrap();
        let got = s.get_by_trace(trace_id, tenant).await.unwrap();
        assert_eq!(got, r);
    }

    #[tokio::test]
    async fn get_by_trace_unknown_returns_not_found() {
        let s = InMemoryLlmProvenanceStore::new();
        let err = s.get_by_trace(Uuid::new_v4(), TenantId::new()).await.unwrap_err();
        assert!(matches!(err, ProvenanceStoreError::NotFound { .. }));
    }

    #[tokio::test]
    async fn cross_tenant_blocked() {
        let s = InMemoryLlmProvenanceStore::new();
        let tenant = TenantId::new();
        let r = record(tenant, Uuid::new_v4(), ModelIdentity::hosted("p", "m", "v"));
        let trace_id = r.trace_id;
        s.insert(r).await.unwrap();
        let err = s.get_by_trace(trace_id, TenantId::new()).await.unwrap_err();
        assert!(matches!(err, ProvenanceStoreError::CrossTenant { .. }));
    }

    #[tokio::test]
    async fn get_by_provenance_resolves_to_trace() {
        let s = InMemoryLlmProvenanceStore::new();
        let tenant = TenantId::new();
        let r = record(tenant, Uuid::new_v4(), ModelIdentity::hosted("p", "m", "v"));
        let pid = r.provenance_id;
        s.insert(r.clone()).await.unwrap();
        let got = s.get_by_provenance(pid, tenant).await.unwrap();
        assert_eq!(got.trace_id, r.trace_id);
    }

    #[tokio::test]
    async fn list_by_swarm_filters_by_swarm_and_tenant() {
        let s = InMemoryLlmProvenanceStore::new();
        let t1 = TenantId::new();
        let t2 = TenantId::new();
        let swarm = Uuid::new_v4();
        let other_swarm = Uuid::new_v4();
        s.insert(record(t1, swarm, ModelIdentity::hosted("p", "m", "v"))).await.unwrap();
        s.insert(record(t1, swarm, ModelIdentity::hosted("p", "m", "v"))).await.unwrap();
        s.insert(record(t1, other_swarm, ModelIdentity::hosted("p", "m", "v"))).await.unwrap();
        s.insert(record(t2, swarm, ModelIdentity::hosted("p", "m", "v"))).await.unwrap();
        let got = s.list_by_swarm(swarm, t1).await.unwrap();
        assert_eq!(got.len(), 2);
        for r in got {
            assert_eq!(r.tenant_id, t1);
            assert_eq!(r.swarm_request_id, swarm);
        }
    }

    #[tokio::test]
    async fn list_by_model_filters_by_identity_and_window() {
        let s = InMemoryLlmProvenanceStore::new();
        let tenant = TenantId::new();
        let m_a = ModelIdentity::hosted("openai", "gpt-4", "2024");
        let m_b = ModelIdentity::hosted("openai", "gpt-3.5", "2024");
        s.insert(record(tenant, Uuid::new_v4(), m_a.clone())).await.unwrap();
        s.insert(record(tenant, Uuid::new_v4(), m_a.clone())).await.unwrap();
        s.insert(record(tenant, Uuid::new_v4(), m_b)).await.unwrap();
        let window = DateRange::since_minutes_ago(60);
        let got = s.list_by_model(&m_a, tenant, window).await.unwrap();
        assert_eq!(got.len(), 2);
    }

    #[tokio::test]
    async fn cost_summary_aggregates_per_model() {
        let s = InMemoryLlmProvenanceStore::new();
        let tenant = TenantId::new();
        let m = ModelIdentity::hosted("openai", "gpt-4", "2024");
        s.insert(record(tenant, Uuid::new_v4(), m.clone())).await.unwrap();
        s.insert(record(tenant, Uuid::new_v4(), m.clone())).await.unwrap();
        let summary = s
            .cost_summary(tenant, DateRange::since_minutes_ago(60))
            .await
            .unwrap();
        assert!((summary.total_cents - 10.0).abs() < 1e-6);
        let row = summary.row_for(&m).unwrap();
        assert_eq!(row.call_count, 2);
        assert!((row.cost_cents - 10.0).abs() < 1e-6);
    }

    #[tokio::test]
    async fn cost_summary_drift_ratio_computed() {
        let s = InMemoryLlmProvenanceStore::new();
        let tenant = TenantId::new();
        let m = ModelIdentity::hosted("p", "m", "v");
        s.insert(record(tenant, Uuid::new_v4(), m)).await.unwrap();
        let summary = s
            .cost_summary(tenant, DateRange::since_minutes_ago(60))
            .await
            .unwrap();
        // client: 100+50=150 ; provider: 110+45=155 ; drift = (155-150)/150 ≈ 0.033
        let drift = summary.drift_ratio.unwrap();
        assert!((drift - 0.033333).abs() < 1e-3);
    }

    #[tokio::test]
    async fn cost_summary_drift_none_when_any_record_lacks_provider() {
        let s = InMemoryLlmProvenanceStore::new();
        let tenant = TenantId::new();
        let m = ModelIdentity::hosted("p", "m", "v");
        let mut r = record(tenant, Uuid::new_v4(), m);
        r.token_usage.provider_reported_input = None;
        r.token_usage.provider_reported_output = None;
        s.insert(r).await.unwrap();
        let summary = s
            .cost_summary(tenant, DateRange::since_minutes_ago(60))
            .await
            .unwrap();
        assert!(summary.drift_ratio.is_none());
    }

    #[tokio::test]
    async fn record_drift_event_and_list() {
        let s = InMemoryLlmProvenanceStore::new();
        let tenant = TenantId::new();
        let event = ModelHashMismatchEvent {
            event_id: Uuid::new_v4(),
            tenant_id: tenant,
            model_identity: ModelIdentity::hosted("p", "m", "v"),
            expected_hash: Blake3Hash::from_bytes([1u8; 32]),
            actual_hash: Blake3Hash::from_bytes([2u8; 32]),
            detected_at: SystemTime::now(),
            trace_id: None,
        };
        s.record_drift_event(event.clone()).await.unwrap();
        let got = s
            .list_drift_events(tenant, DateRange::since_minutes_ago(60))
            .await
            .unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].event_id, event.event_id);
    }

    #[tokio::test]
    async fn list_drift_events_filters_by_tenant() {
        let s = InMemoryLlmProvenanceStore::new();
        let t1 = TenantId::new();
        let t2 = TenantId::new();
        for tenant in [t1, t2] {
            s.record_drift_event(ModelHashMismatchEvent {
                event_id: Uuid::new_v4(),
                tenant_id: tenant,
                model_identity: ModelIdentity::hosted("p", "m", "v"),
                expected_hash: Blake3Hash::ZERO,
                actual_hash: Blake3Hash::ZERO,
                detected_at: SystemTime::now(),
                trace_id: None,
            })
            .await
            .unwrap();
        }
        let got = s.list_drift_events(t1, DateRange::since_minutes_ago(60)).await.unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].tenant_id, t1);
    }

    #[tokio::test]
    async fn debug_format_does_not_panic() {
        let s = InMemoryLlmProvenanceStore::new();
        let _ = format!("{s:?}");
    }

    /// **Post-audit fix — bug D + I10** : re-insertion of an
    /// existing `trace_id` MUST be rejected.  Provenance traces
    /// are write-once.
    #[tokio::test]
    async fn duplicate_insert_rejected_for_immutability() {
        let s = InMemoryLlmProvenanceStore::new();
        let tenant = TenantId::new();
        let r = record(tenant, Uuid::new_v4(), ModelIdentity::hosted("p", "m", "v"));
        s.insert(r.clone()).await.unwrap();
        let err = s.insert(r.clone()).await.unwrap_err();
        assert!(matches!(err, ProvenanceStoreError::AlreadyExists { .. }));
    }

    /// **Post-audit fix — bug D** : a different record with a
    /// different `trace_id` MUST still insert cleanly.
    #[tokio::test]
    async fn different_trace_ids_inserts_concurrently() {
        let s = InMemoryLlmProvenanceStore::new();
        let tenant = TenantId::new();
        let r1 = record(tenant, Uuid::new_v4(), ModelIdentity::hosted("p", "m", "v"));
        let r2 = record(tenant, Uuid::new_v4(), ModelIdentity::hosted("p", "m", "v"));
        s.insert(r1).await.unwrap();
        s.insert(r2).await.unwrap();
    }

    #[test]
    fn date_range_contains() {
        let now = SystemTime::now();
        let r = DateRange {
            from: now - Duration::from_secs(60),
            to: now + Duration::from_secs(60),
        };
        assert!(r.contains(now));
        assert!(!r.contains(now - Duration::from_secs(120)));
    }

    #[test]
    fn date_range_since_minutes_ago() {
        let r = DateRange::since_minutes_ago(60);
        assert!(r.contains(SystemTime::now()));
    }
}
