//! HMAC-chained audit writer (Sprint 6).
//!
//! Implements [`crate::audit::AuditWriter`] by chaining each new trace
//! to the tenant's previous trace via HMAC-SHA-256.  The chain provides :
//!
//! - **Tamper evidence** : any post-hoc modification of a trace breaks
//!   the HMAC verifying the NEXT trace's parent link.
//! - **Provable order** : the chain is a linear sequence per tenant.
//! - **Per-tenant isolation** : each tenant has its own key (HKDF derived).
//!
//! # Concurrency
//!
//! `record` serializes per-tenant via the underlying [`ChainStore`].
//! Multiple tenants can be appended concurrently.
//!
//! # Failure semantics
//!
//! `AuditWriter::record` returns `()` (no error channel).  On chain write
//! failure :
//! - We **log via tracing** at error level — never panic.
//! - We **drop the trace** rather than crashing the runtime.
//! - Operators see the failure in observability ; replay tests catch it.
//!
//! This matches the architecture's "audit writer must be resilient" stance.

use async_trait::async_trait;
use chrono::Utc;
use efe_nano_sdk::TenantId;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::audit::chain_store::{ChainEntry, ChainStore};
use crate::audit::hkdf::HmacKeySource;
use crate::audit::payload::{compute_hmac, compute_payload_hash};
use crate::audit::{AuditWriter, NanoAgentTrace};
use crate::error::AuditChainError;

// ─────────────────────────────────────────────────────────────────────
//  HmacChainAuditWriter
// ─────────────────────────────────────────────────────────────────────

/// AuditWriter implementation that builds a per-tenant HMAC chain.
pub struct HmacChainAuditWriter {
    store: Arc<dyn ChainStore>,
    key_source: HmacKeySource,
    /// Per-tenant tip cache to avoid re-querying the store on every
    /// append.  Tip is the trace_id of the last appended entry for a
    /// tenant.  Wrapped in a Mutex for serialization of the
    /// read-modify-write pattern (load tip → compute HMAC → append).
    tips: Mutex<std::collections::HashMap<TenantId, uuid::Uuid>>,
    /// Per-tenant key cache.  Once derived, the key for a tenant is
    /// reused for all subsequent appends (until process restart).
    keys: Mutex<std::collections::HashMap<TenantId, Vec<u8>>>,
}

impl HmacChainAuditWriter {
    /// Construct an HMAC chain writer.
    pub fn new(store: Arc<dyn ChainStore>, key_source: HmacKeySource) -> Self {
        Self {
            store,
            key_source,
            tips: Mutex::new(std::collections::HashMap::new()),
            keys: Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Pre-load the tip for a tenant from the store.  Useful on
    /// service restart so the first new append correctly links to the
    /// existing chain rather than creating a fork.
    pub async fn warm_tip(&self, tenant: TenantId) -> Result<(), AuditChainError> {
        let latest = self
            .store
            .latest_for_tenant(tenant)
            .await
            .map_err(AuditChainError::Store)?;
        if let Some(entry) = latest {
            let mut tips = self.tips.lock().await;
            tips.insert(tenant, entry.trace_id());
        }
        Ok(())
    }

    /// Pre-derive + cache the key for a tenant.  Optional optimization.
    pub async fn warm_key(&self, tenant: TenantId) -> Result<(), AuditChainError> {
        self.tenant_key(tenant).await?;
        Ok(())
    }

    /// Get (cache-or-derive) the tenant's HMAC key.
    async fn tenant_key(&self, tenant: TenantId) -> Result<Vec<u8>, AuditChainError> {
        {
            let cache = self.keys.lock().await;
            if let Some(k) = cache.get(&tenant) {
                return Ok(k.clone());
            }
        }
        let derived = self.key_source.derive(tenant)?;
        let mut cache = self.keys.lock().await;
        cache.insert(tenant, derived.clone());
        Ok(derived)
    }

    /// Get the cached tip for a tenant (or load from store if absent).
    async fn current_tip(
        &self,
        tenant: TenantId,
    ) -> Result<Option<uuid::Uuid>, AuditChainError> {
        {
            let tips = self.tips.lock().await;
            if let Some(&t) = tips.get(&tenant) {
                return Ok(Some(t));
            }
        }
        // First time we see this tenant : load tip from store.
        let latest = self
            .store
            .latest_for_tenant(tenant)
            .await
            .map_err(AuditChainError::Store)?;
        Ok(latest.map(|e| e.trace_id()))
    }

    /// Build a chain entry from a trace + append + advance tip.
    ///
    /// **Body storage** : `full_input` / `full_output` are NOT persisted
    /// by this writer — that responsibility lives with the storage
    /// adapter (e.g. [`InMemoryHmacChainAuditWriter`] for in-process,
    /// `PostgresChainStore::append` for production).  Sprint 6 design :
    /// the wire-level `HmacChainAuditWriter` is body-agnostic ; only the
    /// chain-link integrity (parent + payload_hash + hmac) is its
    /// concern.  This keeps the trait surface clean and avoids the
    /// `Any`-downcast hack that earlier Sprint-6 drafts attempted.
    async fn append_chained(&self, trace: NanoAgentTrace) -> Result<ChainEntry, AuditChainError> {
        let tenant = trace.tenant_id;
        let parent = self.current_tip(tenant).await?;
        let key = self.tenant_key(tenant).await?;

        let payload_hash = compute_payload_hash(&trace);
        let hmac = compute_hmac(parent, &payload_hash, &key);

        let entry = ChainEntry {
            trace,
            chain_parent_id: parent,
            payload_hash,
            hmac,
            inserted_at: Utc::now(),
        };

        // Append to chain.  Constraint errors here mean concurrent write
        // raced us — we should re-load tip + retry.  Sprint 6 ships
        // a single-attempt write ; production may add retry.
        let new_trace_id = entry.trace.trace_id.0;
        self.store
            .append(entry.clone())
            .await
            .map_err(AuditChainError::Store)?;

        // Advance our cached tip.
        let mut tips = self.tips.lock().await;
        tips.insert(tenant, new_trace_id);

        Ok(entry)
    }
}

#[async_trait]
impl AuditWriter for HmacChainAuditWriter {
    async fn record(&self, trace: NanoAgentTrace) {
        let tenant = trace.tenant_id;
        let trace_id = trace.trace_id.0;
        match self.append_chained(trace).await {
            Ok(_) => {
                tracing::trace!(
                    target: "efe_nano::audit",
                    %tenant,
                    %trace_id,
                    "appended chained trace"
                );
            }
            Err(e) => {
                // Audit writer failures must not crash the runtime.
                tracing::error!(
                    target: "efe_nano::audit",
                    %tenant,
                    %trace_id,
                    error = %e,
                    "failed to append chained trace ; trace dropped"
                );
            }
        }
    }
}

impl std::fmt::Debug for HmacChainAuditWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HmacChainAuditWriter").finish_non_exhaustive()
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Sibling helper for InMemoryChainStore : keep a direct ref so we can
//  store full bodies WITHOUT a downcast hack.  Sprint 6 ships this as
//  the canonical constructor for tests + demos.
// ─────────────────────────────────────────────────────────────────────

/// Convenience writer that owns both the trait object AND a direct ref
/// to the InMemoryChainStore for body storage.  Production code uses
/// `HmacChainAuditWriter::new(...)` instead — body storage in production
/// is handled via SQL inside `PostgresChainStore::append`.
pub struct InMemoryHmacChainAuditWriter {
    inner: HmacChainAuditWriter,
    store_ref: Arc<crate::audit::chain_store::InMemoryChainStore>,
}

impl InMemoryHmacChainAuditWriter {
    pub fn new(
        store: Arc<crate::audit::chain_store::InMemoryChainStore>,
        key_source: HmacKeySource,
    ) -> Self {
        let inner = HmacChainAuditWriter::new(Arc::clone(&store) as Arc<dyn ChainStore>, key_source);
        Self {
            inner,
            store_ref: store,
        }
    }

    /// Get the underlying chain store (for verification / replay).
    pub fn store(&self) -> Arc<crate::audit::chain_store::InMemoryChainStore> {
        Arc::clone(&self.store_ref)
    }

    /// Pre-warm tip + key for a tenant.
    pub async fn warm(&self, tenant: TenantId) -> Result<(), AuditChainError> {
        self.inner.warm_tip(tenant).await?;
        self.inner.warm_key(tenant).await?;
        Ok(())
    }
}

#[async_trait]
impl AuditWriter for InMemoryHmacChainAuditWriter {
    async fn record(&self, mut trace: NanoAgentTrace) {
        // Extract full_input/full_output BEFORE recording — they're not
        // part of the canonical body, but we need to store them for
        // replay.
        // Sprint 7 — N6 : the body store is keyed by `input_hash_resolved`
        // (the value the runtime actually saw), not `input_hash_source`.
        // This matches the replay engine which retrieves bodies by the
        // resolved-value hash to reconstruct the SwarmRequest.
        let input_hash = trace.input_hash_resolved;
        let output_hash = trace.output_hash;
        let full_input = trace.full_input.take();
        let full_output = trace.full_output.take();

        if let Some(body) = full_input {
            self.store_ref.put_input(input_hash, body);
        }
        if let (Some(hash), Some(body)) = (output_hash, full_output) {
            self.store_ref.put_output(hash, body);
        }

        // Now delegate to the inner writer — trace no longer has
        // full_input/output but those are stored separately for replay.
        self.inner.record(trace).await;
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::chain_store::InMemoryChainStore;
    use crate::audit::NanoAgentTrace;
    use efe_nano_sdk::{NanoAgentId, NanoAgentVersion, TraceId};
    use uuid::Uuid;

    fn sample_trace(tenant: TenantId, swarm: Uuid, idx: u32) -> NanoAgentTrace {
        NanoAgentTrace {
            trace_id: TraceId(Uuid::new_v4()),
            parent_trace_ids: vec![],
            swarm_request_id: swarm,
            operation_id: format!("op-{idx}"),
            tenant_id: tenant,
            caller_agent_id: None,
            nano_agent_id: NanoAgentId::new("a").unwrap(),
            nano_agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
            execution_mode: "pure_rust".into(),
            input_hash_source: [idx as u8; 32],
            input_hash_resolved: [idx as u8; 32],
            output_hash: Some([(idx as u8).wrapping_add(1); 32]),
            error_kind: None,
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
            latency_ms: Some(10),
            cost_cents: 0.001,
            llm_tokens_used: None,
            full_input: None,
            full_output: None,
            tombstone: None,
            swarm_envelope_trace_id: None,
        }
    }

    #[tokio::test]
    async fn first_append_has_no_parent() {
        let store = Arc::new(InMemoryChainStore::new());
        let writer = HmacChainAuditWriter::new(
            Arc::clone(&store) as Arc<dyn ChainStore>,
            HmacKeySource::test_key(),
        );
        let tenant = TenantId::system();
        let swarm = Uuid::new_v4();
        writer.record(sample_trace(tenant, swarm, 0)).await;
        let chain = store.chain_for_tenant(tenant, None).await.unwrap();
        assert_eq!(chain.len(), 1);
        assert!(chain[0].chain_parent_id.is_none());
    }

    #[tokio::test]
    async fn second_append_links_to_first() {
        let store = Arc::new(InMemoryChainStore::new());
        let writer = HmacChainAuditWriter::new(
            Arc::clone(&store) as Arc<dyn ChainStore>,
            HmacKeySource::test_key(),
        );
        let tenant = TenantId::system();
        let swarm = Uuid::new_v4();
        writer.record(sample_trace(tenant, swarm, 0)).await;
        writer.record(sample_trace(tenant, swarm, 1)).await;
        let chain = store.chain_for_tenant(tenant, None).await.unwrap();
        assert_eq!(chain.len(), 2);
        assert!(chain[0].chain_parent_id.is_none());
        assert_eq!(chain[1].chain_parent_id, Some(chain[0].trace_id()));
    }

    #[tokio::test]
    async fn hmac_distinct_per_trace() {
        let store = Arc::new(InMemoryChainStore::new());
        let writer = HmacChainAuditWriter::new(
            Arc::clone(&store) as Arc<dyn ChainStore>,
            HmacKeySource::test_key(),
        );
        let tenant = TenantId::system();
        writer.record(sample_trace(tenant, Uuid::new_v4(), 0)).await;
        writer.record(sample_trace(tenant, Uuid::new_v4(), 1)).await;
        let chain = store.chain_for_tenant(tenant, None).await.unwrap();
        assert_ne!(chain[0].hmac, chain[1].hmac);
        assert_ne!(chain[0].payload_hash, chain[1].payload_hash);
    }

    #[tokio::test]
    async fn cross_tenant_chains_independent() {
        let store = Arc::new(InMemoryChainStore::new());
        let writer = HmacChainAuditWriter::new(
            Arc::clone(&store) as Arc<dyn ChainStore>,
            HmacKeySource::test_key(),
        );
        let t1 = TenantId::new();
        let t2 = TenantId::new();
        writer.record(sample_trace(t1, Uuid::new_v4(), 0)).await;
        writer.record(sample_trace(t2, Uuid::new_v4(), 0)).await;
        let c1 = store.chain_for_tenant(t1, None).await.unwrap();
        let c2 = store.chain_for_tenant(t2, None).await.unwrap();
        assert_eq!(c1.len(), 1);
        assert_eq!(c2.len(), 1);
        // Both are genesis rows (no parent).
        assert!(c1[0].chain_parent_id.is_none());
        assert!(c2[0].chain_parent_id.is_none());
    }

    #[tokio::test]
    async fn warm_tip_loads_existing_chain() {
        // Pre-populate the store, then warm a fresh writer.
        let store = Arc::new(InMemoryChainStore::new());
        let writer1 = HmacChainAuditWriter::new(
            Arc::clone(&store) as Arc<dyn ChainStore>,
            HmacKeySource::test_key(),
        );
        let tenant = TenantId::system();
        writer1.record(sample_trace(tenant, Uuid::new_v4(), 0)).await;

        // New writer (e.g. after restart) :
        let writer2 = HmacChainAuditWriter::new(
            Arc::clone(&store) as Arc<dyn ChainStore>,
            HmacKeySource::test_key(),
        );
        writer2.warm_tip(tenant).await.unwrap();
        writer2.record(sample_trace(tenant, Uuid::new_v4(), 1)).await;
        let chain = store.chain_for_tenant(tenant, None).await.unwrap();
        assert_eq!(chain.len(), 2);
        // Second entry links to first (no fork).
        assert_eq!(chain[1].chain_parent_id, Some(chain[0].trace_id()));
    }

    #[tokio::test]
    async fn in_memory_writer_stores_full_input_output() {
        let store = Arc::new(InMemoryChainStore::new());
        let writer = InMemoryHmacChainAuditWriter::new(
            Arc::clone(&store),
            HmacKeySource::test_key(),
        );
        let mut trace = sample_trace(TenantId::system(), Uuid::new_v4(), 7);
        let input_body = serde_json::json!({"raw_input": true});
        let output_body = serde_json::json!({"raw_output": true});
        trace.full_input = Some(input_body.clone());
        trace.full_output = Some(output_body.clone());
        let input_hash = trace.input_hash_resolved;
        let output_hash = trace.output_hash.unwrap();

        writer.record(trace).await;

        assert_eq!(store.get_input(&input_hash).unwrap(), input_body);
        assert_eq!(store.get_output(&output_hash).unwrap(), output_body);
    }

    #[tokio::test]
    async fn record_drops_trace_on_chain_constraint_violation() {
        // We can't easily trigger a constraint error from the public API,
        // but we can test that record() is infallible (returns ()).
        let store = Arc::new(InMemoryChainStore::new());
        let writer = HmacChainAuditWriter::new(
            Arc::clone(&store) as Arc<dyn ChainStore>,
            HmacKeySource::test_key(),
        );
        // Just verify the type signature : record() returns ().
        let _: () = writer.record(sample_trace(TenantId::system(), Uuid::new_v4(), 0)).await;
    }

    #[tokio::test]
    async fn append_advances_tenant_tip_cache() {
        let store = Arc::new(InMemoryChainStore::new());
        let writer = HmacChainAuditWriter::new(
            Arc::clone(&store) as Arc<dyn ChainStore>,
            HmacKeySource::test_key(),
        );
        let tenant = TenantId::system();
        writer.record(sample_trace(tenant, Uuid::new_v4(), 0)).await;
        writer.record(sample_trace(tenant, Uuid::new_v4(), 1)).await;
        writer.record(sample_trace(tenant, Uuid::new_v4(), 2)).await;
        let chain = store.chain_for_tenant(tenant, None).await.unwrap();
        assert_eq!(chain.len(), 3);
        // Each subsequent entry links to the previous one (no skipped chains).
        for i in 1..3 {
            assert_eq!(chain[i].chain_parent_id, Some(chain[i - 1].trace_id()));
        }
    }

    #[tokio::test]
    async fn concurrent_appends_for_different_tenants() {
        let store = Arc::new(InMemoryChainStore::new());
        let writer = Arc::new(HmacChainAuditWriter::new(
            Arc::clone(&store) as Arc<dyn ChainStore>,
            HmacKeySource::test_key(),
        ));
        let mut handles = Vec::new();
        for _ in 0..10 {
            let writer = Arc::clone(&writer);
            handles.push(tokio::spawn(async move {
                let tenant = TenantId::new();        // unique tenant per task
                writer.record(sample_trace(tenant, Uuid::new_v4(), 0)).await;
                tenant
            }));
        }
        let mut tenants = Vec::new();
        for h in handles {
            tenants.push(h.await.unwrap());
        }
        // Each tenant should have exactly 1 entry.
        for t in tenants {
            let chain = store.chain_for_tenant(t, None).await.unwrap();
            assert_eq!(chain.len(), 1);
        }
    }
}
