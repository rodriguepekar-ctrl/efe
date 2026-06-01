//! Chain integrity verifier (Sprint 6).
//!
//! Walks a tenant's chain entry-by-entry, recomputing payload_hash + HMAC
//! and comparing against stored values.  First mismatch returns an error
//! with the offending trace_id.
//!
//! # Verification cost
//!
//! Recomputing one HMAC is ~ 50 μs.  Verifying a 10k-entry chain takes
//! ~ 500 ms single-threaded.  For larger chains, callers may verify in
//! batches (the `since` parameter on `ChainStore::chain_for_tenant`
//! supports incremental verification).
//!
//! # Sub-chain verification
//!
//! Replay only needs to verify the entries of a single swarm.  The
//! [`verify_subchain`] function takes a `Vec<ChainEntry>` and verifies
//! the chain relations + HMACs assuming the first entry's
//! `chain_parent_id` references a valid prior entry (not loaded).

use chrono::{DateTime, Utc};
use efe_nano_sdk::TenantId;

use crate::audit::chain_store::{ChainEntry, ChainStore};
use crate::audit::hkdf::HmacKeySource;
use crate::audit::payload::{compute_hmac, compute_payload_hash};
use crate::error::AuditChainError;

// ─────────────────────────────────────────────────────────────────────
//  ChainIntegrityReport
// ─────────────────────────────────────────────────────────────────────

/// Result of a successful chain verification.
#[derive(Debug, Clone)]
pub struct ChainIntegrityReport {
    pub tenant: TenantId,
    /// Number of entries verified.
    pub entries_verified: u64,
    /// First entry's trace_id (genesis or sub-chain anchor).
    pub first_trace_id: Option<uuid::Uuid>,
    /// Last entry's trace_id (current tip).
    pub last_trace_id: Option<uuid::Uuid>,
    /// Total wall-clock time spent verifying (ms).
    pub elapsed_ms: u32,
}

// ─────────────────────────────────────────────────────────────────────
//  verify_chain — full per-tenant verification
// ─────────────────────────────────────────────────────────────────────

/// Verify the full HMAC chain for a tenant.  Walks all entries since
/// `since` (or genesis if None), recomputing payload_hash + HMAC and
/// checking parent links.
pub async fn verify_chain(
    store: &dyn ChainStore,
    key_source: &HmacKeySource,
    tenant: TenantId,
    since: Option<DateTime<Utc>>,
) -> Result<ChainIntegrityReport, AuditChainError> {
    let started = std::time::Instant::now();
    let chain = store
        .chain_for_tenant(tenant, since)
        .await
        .map_err(AuditChainError::Store)?;

    if chain.is_empty() {
        return Ok(ChainIntegrityReport {
            tenant,
            entries_verified: 0,
            first_trace_id: None,
            last_trace_id: None,
            elapsed_ms: started.elapsed().as_millis().min(u32::MAX as u128) as u32,
        });
    }

    let key = key_source.derive(tenant)?;
    let mut prev_id: Option<uuid::Uuid> = chain[0].chain_parent_id;
    let first_id = chain[0].trace_id();

    for entry in &chain {
        verify_entry(entry, prev_id, &key)?;
        prev_id = Some(entry.trace_id());
    }

    let last_id = chain.last().map(|e| e.trace_id());
    Ok(ChainIntegrityReport {
        tenant,
        entries_verified: chain.len() as u64,
        first_trace_id: Some(first_id),
        last_trace_id: last_id,
        elapsed_ms: started.elapsed().as_millis().min(u32::MAX as u128) as u32,
    })
}

// ─────────────────────────────────────────────────────────────────────
//  verify_subchain — verify a slice of the chain (e.g. for replay)
// ─────────────────────────────────────────────────────────────────────

/// Verify a slice of chain entries.  Used by replay to verify only the
/// entries of a single swarm.
///
/// The first entry's `chain_parent_id` is NOT verified to exist in the
/// store ; we trust it.  The subsequent entries' parent links are
/// verified to match the previous entry's trace_id.
pub async fn verify_subchain(
    entries: &[ChainEntry],
    key_source: &HmacKeySource,
    tenant: TenantId,
) -> Result<ChainIntegrityReport, AuditChainError> {
    let started = std::time::Instant::now();
    if entries.is_empty() {
        return Ok(ChainIntegrityReport {
            tenant,
            entries_verified: 0,
            first_trace_id: None,
            last_trace_id: None,
            elapsed_ms: 0,
        });
    }

    let key = key_source.derive(tenant)?;
    let mut prev_id: Option<uuid::Uuid> = entries[0].chain_parent_id;

    for entry in entries {
        // Cross-tenant guard.
        if entry.trace.tenant_id != tenant {
            return Err(AuditChainError::Store(
                crate::error::ChainStoreError::CrossTenant {
                    actual: tenant,
                    requested: entry.trace.tenant_id,
                },
            ));
        }
        verify_entry(entry, prev_id, &key)?;
        prev_id = Some(entry.trace_id());
    }

    Ok(ChainIntegrityReport {
        tenant,
        entries_verified: entries.len() as u64,
        first_trace_id: Some(entries[0].trace_id()),
        last_trace_id: Some(entries[entries.len() - 1].trace_id()),
        elapsed_ms: started.elapsed().as_millis().min(u32::MAX as u128) as u32,
    })
}

// ─────────────────────────────────────────────────────────────────────
//  verify_entry — single-row check
// ─────────────────────────────────────────────────────────────────────

fn verify_entry(
    entry: &ChainEntry,
    expected_parent: Option<uuid::Uuid>,
    key: &[u8],
) -> Result<(), AuditChainError> {
    // 1. Parent link.
    if entry.chain_parent_id != expected_parent {
        return Err(AuditChainError::ChainBroken(entry.trace_id()));
    }

    // 2. Payload hash : recompute and compare.
    let recomputed = compute_payload_hash(&entry.trace);
    if recomputed != entry.payload_hash {
        return Err(AuditChainError::HmacMismatch(entry.trace_id()));
    }

    // 3. HMAC : recompute and compare.
    let recomputed_hmac = compute_hmac(expected_parent, &entry.payload_hash, key);
    if recomputed_hmac != entry.hmac {
        return Err(AuditChainError::HmacMismatch(entry.trace_id()));
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::chain_store::InMemoryChainStore;
    use crate::audit::hmac_chain::HmacChainAuditWriter;
    use crate::audit::{AuditWriter, NanoAgentTrace};
    use efe_nano_sdk::{NanoAgentId, NanoAgentVersion, TraceId};
    use std::sync::Arc;
    use uuid::Uuid;

    fn trace(tenant: TenantId, swarm: Uuid, idx: u32) -> NanoAgentTrace {
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

    async fn populate(
        writer: &HmacChainAuditWriter,
        tenant: TenantId,
        count: u32,
    ) {
        for i in 0..count {
            writer.record(trace(tenant, Uuid::new_v4(), i)).await;
        }
    }

    #[tokio::test]
    async fn verify_empty_chain_returns_zero() {
        let store = Arc::new(InMemoryChainStore::new());
        let key = HmacKeySource::test_key();
        let report = verify_chain(&*store, &key, TenantId::system(), None).await.unwrap();
        assert_eq!(report.entries_verified, 0);
    }

    #[tokio::test]
    async fn verify_valid_chain_succeeds() {
        let store = Arc::new(InMemoryChainStore::new());
        let key = HmacKeySource::test_key();
        let writer = HmacChainAuditWriter::new(
            Arc::clone(&store) as Arc<dyn ChainStore>,
            key.clone(),
        );
        let tenant = TenantId::system();
        populate(&writer, tenant, 10).await;
        let report = verify_chain(&*store, &key, tenant, None).await.unwrap();
        assert_eq!(report.entries_verified, 10);
    }

    #[tokio::test]
    async fn verify_detects_tampered_payload() {
        let store = Arc::new(InMemoryChainStore::new());
        let key = HmacKeySource::test_key();
        let writer = HmacChainAuditWriter::new(
            Arc::clone(&store) as Arc<dyn ChainStore>,
            key.clone(),
        );
        let tenant = TenantId::system();
        populate(&writer, tenant, 3).await;

        // Tamper : modify the second entry's cost_cents in-memory.
        // (We can't easily do this with the public API since DashMap is
        // not exposed.  Instead, we reconstruct a tampered chain by hand.)
        let mut chain = store.chain_for_tenant(tenant, None).await.unwrap();
        chain[1].trace.cost_cents = 9999.99;

        let err = verify_subchain(&chain, &key, tenant).await.unwrap_err();
        match err {
            AuditChainError::HmacMismatch(_) => {}
            other => panic!("expected HmacMismatch, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn verify_detects_wrong_parent_link() {
        let store = Arc::new(InMemoryChainStore::new());
        let key = HmacKeySource::test_key();
        let writer = HmacChainAuditWriter::new(
            Arc::clone(&store) as Arc<dyn ChainStore>,
            key.clone(),
        );
        let tenant = TenantId::system();
        populate(&writer, tenant, 3).await;

        let mut chain = store.chain_for_tenant(tenant, None).await.unwrap();
        // Tamper : change chain_parent_id of entry 1 to a wrong UUID.
        chain[1].chain_parent_id = Some(Uuid::new_v4());

        let err = verify_subchain(&chain, &key, tenant).await.unwrap_err();
        match err {
            AuditChainError::ChainBroken(_) => {}
            other => panic!("expected ChainBroken, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn verify_detects_wrong_key() {
        let store = Arc::new(InMemoryChainStore::new());
        let original_key = HmacKeySource::test_key();
        let writer = HmacChainAuditWriter::new(
            Arc::clone(&store) as Arc<dyn ChainStore>,
            original_key,
        );
        let tenant = TenantId::system();
        populate(&writer, tenant, 3).await;

        // Verify with a different key → HMAC mismatch.
        let wrong_key = HmacKeySource::StaticMaster(
            b"different-master-32-bytes-fixed_x".to_vec(),
        );
        let err = verify_chain(&*store, &wrong_key, tenant, None).await.unwrap_err();
        match err {
            AuditChainError::HmacMismatch(_) => {}
            other => panic!("expected HmacMismatch, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn verify_subchain_cross_tenant_rejected() {
        let store = Arc::new(InMemoryChainStore::new());
        let key = HmacKeySource::test_key();
        let writer = HmacChainAuditWriter::new(
            Arc::clone(&store) as Arc<dyn ChainStore>,
            key.clone(),
        );
        let t1 = TenantId::new();
        populate(&writer, t1, 2).await;
        let chain = store.chain_for_tenant(t1, None).await.unwrap();

        // Verify pretending the chain belongs to a DIFFERENT tenant.
        let t2 = TenantId::new();
        let err = verify_subchain(&chain, &key, t2).await.unwrap_err();
        // Either CrossTenant store error (caught by tenant_id check)
        // OR HmacMismatch (different key derived).  Both prove isolation.
        let _ = err;
    }

    #[tokio::test]
    async fn verify_report_includes_first_and_last_ids() {
        let store = Arc::new(InMemoryChainStore::new());
        let key = HmacKeySource::test_key();
        let writer = HmacChainAuditWriter::new(
            Arc::clone(&store) as Arc<dyn ChainStore>,
            key.clone(),
        );
        let tenant = TenantId::system();
        populate(&writer, tenant, 5).await;
        let report = verify_chain(&*store, &key, tenant, None).await.unwrap();
        let chain = store.chain_for_tenant(tenant, None).await.unwrap();
        assert_eq!(report.first_trace_id, Some(chain[0].trace_id()));
        assert_eq!(report.last_trace_id, Some(chain[4].trace_id()));
        assert_eq!(report.entries_verified, 5);
    }

    #[tokio::test]
    async fn verify_handles_1k_entry_chain() {
        let store = Arc::new(InMemoryChainStore::new());
        let key = HmacKeySource::test_key();
        let writer = HmacChainAuditWriter::new(
            Arc::clone(&store) as Arc<dyn ChainStore>,
            key.clone(),
        );
        let tenant = TenantId::system();
        populate(&writer, tenant, 1_000).await;
        let report = verify_chain(&*store, &key, tenant, None).await.unwrap();
        assert_eq!(report.entries_verified, 1_000);
        // Should complete in well under a second.
        assert!(report.elapsed_ms < 5_000, "verify took {} ms", report.elapsed_ms);
    }
}
