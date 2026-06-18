// SPDX-License-Identifier: UNLICENSED
//! **Sprint 9 — N17** : Query API surface — wraps the
//! [`LlmProvenanceStore`] with metric instrumentation + a stable
//! call shape suitable for HTTP handlers.
//!
//! Architecture § 7.7 + § 9.1.
//!
//! # Tenant scoping
//!
//! Every method takes a `tenant: TenantId` parameter.  The caller
//! (HTTP layer / dispatcher) is responsible for authenticating the
//! request and extracting the tenant from the JWT claims.  This
//! layer enforces the tenant scope at the store level — cross-tenant
//! reads return [`ProvenanceStoreError::CrossTenant`].

use std::sync::Arc;
use std::time::Instant;

use efe_nano_sdk::{LlmProvenanceRecord, ModelIdentity, ProvenanceId, TenantId};
use uuid::Uuid;

use crate::metrics::N17Metrics;
use crate::query::{CostSummary, DateRange, ModelHashMismatchEvent};
use crate::store::{LlmProvenanceStore, ProvenanceStoreError};

/// **Sprint 9 — N17** : provenance query API.
///
/// Thin layer over [`LlmProvenanceStore`] that records query
/// latency in [`N17Metrics`] (architecture § 7.8 metric #12) and
/// exposes the five canonical access patterns.
pub struct LlmProvenanceQueryApi {
    store: Arc<dyn LlmProvenanceStore>,
    metrics: Option<Arc<N17Metrics>>,
}

impl std::fmt::Debug for LlmProvenanceQueryApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmProvenanceQueryApi")
            .field("has_metrics", &self.metrics.is_some())
            .finish()
    }
}

impl LlmProvenanceQueryApi {
    /// **N17** : Construct an API instance.
    pub fn new(store: Arc<dyn LlmProvenanceStore>) -> Arc<Self> {
        Arc::new(Self { store, metrics: None })
    }

    /// **N17** : Construct with metrics installed.
    pub fn with_metrics(
        store: Arc<dyn LlmProvenanceStore>,
        metrics: Arc<N17Metrics>,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            metrics: Some(metrics),
        })
    }

    /// **N17** : Access the underlying store.
    pub fn store(&self) -> &Arc<dyn LlmProvenanceStore> {
        &self.store
    }

    fn observe_latency(&self, endpoint: &str, started: Instant) {
        if let Some(m) = &self.metrics {
            m.observe_query_latency(endpoint, started.elapsed().as_secs_f64());
        }
    }

    /// **N17** : `GET /v1/provenance/{trace_id}` — fetch a record by
    /// trace id.  Tenant-scoped.
    pub async fn get_by_trace(
        &self,
        trace_id: Uuid,
        tenant: TenantId,
    ) -> Result<LlmProvenanceRecord, ProvenanceStoreError> {
        let started = Instant::now();
        let result = self.store.get_by_trace(trace_id, tenant).await;
        self.observe_latency("get_by_trace", started);
        result
    }

    /// **N17** : Fetch by provenance id.  Tenant-scoped.
    pub async fn get_by_provenance(
        &self,
        provenance_id: ProvenanceId,
        tenant: TenantId,
    ) -> Result<LlmProvenanceRecord, ProvenanceStoreError> {
        let started = Instant::now();
        let result = self.store.get_by_provenance(provenance_id, tenant).await;
        self.observe_latency("get_by_provenance", started);
        result
    }

    /// **N17** : `GET /v1/provenance/swarm/{swarm_request_id}` —
    /// list all records for a swarm.  Tenant-scoped.
    pub async fn list_by_swarm(
        &self,
        swarm_request_id: Uuid,
        tenant: TenantId,
    ) -> Result<Vec<LlmProvenanceRecord>, ProvenanceStoreError> {
        let started = Instant::now();
        let result = self.store.list_by_swarm(swarm_request_id, tenant).await;
        self.observe_latency("list_by_swarm", started);
        result
    }

    /// **N17** : `GET /v1/provenance/model?...` — list records for a
    /// model identity within a window.  Tenant-scoped.
    pub async fn list_by_model(
        &self,
        model: &ModelIdentity,
        tenant: TenantId,
        window: DateRange,
    ) -> Result<Vec<LlmProvenanceRecord>, ProvenanceStoreError> {
        let started = Instant::now();
        let result = self.store.list_by_model(model, tenant, window).await;
        self.observe_latency("list_by_model", started);
        result
    }

    /// **N17** : `GET /v1/provenance/cost?...` — aggregate cost
    /// summary for a tenant within a window.
    pub async fn cost_summary(
        &self,
        tenant: TenantId,
        window: DateRange,
    ) -> Result<CostSummary, ProvenanceStoreError> {
        let started = Instant::now();
        let result = self.store.cost_summary(tenant, window).await;
        self.observe_latency("cost_summary", started);
        result
    }

    /// **N17** : `GET /v1/provenance/drift?...` — list model-hash
    /// mismatch events within a window.
    pub async fn list_drift_events(
        &self,
        tenant: TenantId,
        window: DateRange,
    ) -> Result<Vec<ModelHashMismatchEvent>, ProvenanceStoreError> {
        let started = Instant::now();
        let result = self.store.list_drift_events(tenant, window).await;
        self.observe_latency("list_drift_events", started);
        result
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::InMemoryLlmProvenanceStore;
    use efe_nano_sdk::{
        Blake3Hash, CostAccounting, FinishReason, NanoAgentId, NanoAgentVersion, PiiStrategy,
        SamplingParams, TokenUsage,
    };
    use std::collections::HashMap;
    use std::time::SystemTime;

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
            token_usage: TokenUsage::default(),
            cost_accounting: CostAccounting::default(),
            provider_request_id: None,
            provider_response_time_ms: 0,
            provider_finish_reason: FinishReason::Stop,
            provider_region: None,
            publisher_id: None,
            publisher_kyc_tier: None,
            gold_case_id: None,
            provider_metadata: HashMap::new(),
            output_hash: None,
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

    fn api() -> (Arc<LlmProvenanceQueryApi>, Arc<InMemoryLlmProvenanceStore>, Arc<N17Metrics>) {
        let store = Arc::new(InMemoryLlmProvenanceStore::new());
        let metrics = N17Metrics::new();
        let api = LlmProvenanceQueryApi::with_metrics(
            store.clone() as Arc<dyn LlmProvenanceStore>,
            metrics.clone(),
        );
        (api, store, metrics)
    }

    #[tokio::test]
    async fn get_by_trace_works() {
        let (api, store, _metrics) = api();
        let tenant = TenantId::new();
        let r = record(tenant, Uuid::new_v4(), ModelIdentity::hosted("p", "m", "v"));
        let trace_id = r.trace_id;
        store.insert(r.clone()).await.unwrap();
        let got = api.get_by_trace(trace_id, tenant).await.unwrap();
        assert_eq!(got, r);
    }

    #[tokio::test]
    async fn get_by_trace_records_query_latency() {
        let (api, store, metrics) = api();
        let tenant = TenantId::new();
        let r = record(tenant, Uuid::new_v4(), ModelIdentity::hosted("p", "m", "v"));
        let trace_id = r.trace_id;
        store.insert(r).await.unwrap();
        api.get_by_trace(trace_id, tenant).await.unwrap();
        let out = metrics.render_prometheus();
        assert!(out.contains("n17_provenance_query_latency_seconds_count{endpoint=\"get_by_trace\"} 1"));
    }

    #[tokio::test]
    async fn cross_tenant_blocked_via_api() {
        let (api, store, _metrics) = api();
        let tenant_a = TenantId::new();
        let tenant_b = TenantId::new();
        let r = record(tenant_a, Uuid::new_v4(), ModelIdentity::hosted("p", "m", "v"));
        let trace_id = r.trace_id;
        store.insert(r).await.unwrap();
        let err = api.get_by_trace(trace_id, tenant_b).await.unwrap_err();
        assert!(matches!(err, ProvenanceStoreError::CrossTenant { .. }));
    }

    #[tokio::test]
    async fn list_by_swarm_records_latency_per_endpoint() {
        let (api, _store, metrics) = api();
        let tenant = TenantId::new();
        let swarm = Uuid::new_v4();
        api.list_by_swarm(swarm, tenant).await.unwrap();
        api.list_by_swarm(swarm, tenant).await.unwrap();
        let out = metrics.render_prometheus();
        assert!(out
            .contains("n17_provenance_query_latency_seconds_count{endpoint=\"list_by_swarm\"} 2"));
    }

    #[tokio::test]
    async fn cost_summary_endpoint_returns_empty_when_no_records() {
        let (api, _store, _metrics) = api();
        let summary = api
            .cost_summary(TenantId::new(), DateRange::since_minutes_ago(60))
            .await
            .unwrap();
        assert_eq!(summary.call_count, 0);
        assert_eq!(summary.total_cents, 0.0);
        assert!(summary.by_model.is_empty());
    }

    #[tokio::test]
    async fn list_drift_events_returns_empty_initially() {
        let (api, _store, _metrics) = api();
        let events = api
            .list_drift_events(TenantId::new(), DateRange::since_minutes_ago(60))
            .await
            .unwrap();
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn debug_format_does_not_panic() {
        let (api, _store, _metrics) = api();
        let _ = format!("{api:?}");
    }
}
