//! **Sprint 7 — N9** : Epoch sealing orchestrator.
//!
//! # The model in one paragraph
//!
//! The pre-Sprint-7 audit chain (Sprint 6) is a linear HMAC chain :
//! every new trace links to the previous via `chain_parent_id +
//! payload_hash + HMAC`.  Verifying the integrity of a 1 M-trace chain
//! is O(N) — every HMAC must be recomputed.  At GA scale (100 M+ traces
//! per year per large tenant), that's hours of CPU on every audit.
//!
//! **Sprint 7 — N9** wraps the linear chain with a **Merkle epoch
//! layer** :
//!
//! - The linear HMAC chain stays untouched (additive change).
//! - Periodically, an [`EpochSealer`] runs over the chain, takes the
//!   next `epoch_size_traces` (default 1024) unsealed traces, builds
//!   a Merkle tree over their `payload_hash`es, stores the root as an
//!   [`EpochRoot`], and (optionally) submits it to a [`Notary`] for
//!   external attestation.
//! - Each [`EpochRoot`] commits to the previous one via
//!   `previous_epoch_root` — so the chain-of-roots is itself a hash
//!   chain that an auditor can walk in O(N / epoch_size) instead of
//!   O(N).
//! - Inclusion proof for a specific trace : O(log N).
//!
//! # Two-layer verification
//!
//! Sprint 7 audit verification becomes **two levels** :
//! 1. **Per-epoch** : recompute the Merkle root over the epoch's
//!    traces, compare to the stored `EpochRoot.root_hash`.
//! 2. **Cross-epoch** : walk the chain of `EpochRoot`s, confirm each
//!    `previous_epoch_root` matches the actual previous root hash.
//!
//! Optionally, **level 3** : verify the notary receipt for each
//! epoch root.
//!
//! # Concurrency model
//!
//! Sealing is **serialized per tenant** (via `Mutex<HashMap<TenantId,
//! Mutex<()>>>` inside the sealer).  Different tenants can seal in
//! parallel ; within one tenant, sealing is sequential — otherwise two
//! concurrent sealers would race on the "next epoch_id".

use chrono::{DateTime, Utc};
use efe_nano_sdk::TenantId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::audit::chain_store::{ChainEntry, ChainStore};
use crate::audit::merkle::store::EpochRootStore;
use crate::audit::merkle::tree::{
    build_tree, leaf_hash, verify_inclusion, InclusionProof, MerkleError, MerkleTree,
};
use crate::error::ChainStoreError;

// ─────────────────────────────────────────────────────────────────────
//  EpochRoot — the persisted entity
// ─────────────────────────────────────────────────────────────────────

/// A sealed Merkle epoch covering a contiguous slice of an HMAC chain.
///
/// Persisted in `epoch_roots` (migration v008 SQL).
///
/// # Semantics
///
/// - `epoch_id` : 1-indexed, monotonic per `tenant_id`.
/// - `leaf_count` : number of traces in the epoch (typically
///   `epoch_size_traces`, but the last epoch may be smaller).
/// - `root_hash` : the Merkle root, SHA-256.
/// - `previous_epoch_root` : `None` iff `epoch_id == 1` ; otherwise
///   the `root_hash` of `epoch_id - 1` for the same tenant.
/// - `signature` : HMAC of the canonical row body, keyed by the
///   tenant's audit HMAC key (same key source as the trace chain).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpochRoot {
    pub tenant_id: TenantId,
    pub epoch_id: u64,
    pub start_trace_id: Uuid,
    pub end_trace_id: Uuid,
    pub leaf_count: u32,
    pub root_hash: [u8; 32],
    /// `None` for the first epoch ; for epoch K > 1, this MUST equal
    /// `EpochRoot{epoch_id: K-1}.root_hash` (enforced by
    /// [`crate::audit::merkle::store::EpochRootStore::append`]).
    pub previous_epoch_root: Option<[u8; 32]>,
    pub sealed_at: DateTime<Utc>,
    /// HMAC-SHA256(canonical_root_body, tenant_audit_key).  Lets
    /// auditors verify the row hasn't been mutated post-write.
    pub signature: Vec<u8>,
}

/// Canonical bytes-for-signing of an [`EpochRoot`].  Excludes
/// `signature` (chicken-and-egg).  Used both at sealing (to compute
/// the signature) and at verification (to recompute and compare).
pub fn canonical_root_body(root: &EpochRoot) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"efe-nano-merkle-epoch-body-v1");
    h.update(root.tenant_id.to_string().as_bytes());
    h.update(root.epoch_id.to_be_bytes());
    h.update(root.start_trace_id.as_bytes());
    h.update(root.end_trace_id.as_bytes());
    h.update(root.leaf_count.to_be_bytes());
    h.update(root.root_hash);
    if let Some(p) = root.previous_epoch_root {
        h.update([1u8]);
        h.update(p);
    } else {
        h.update([0u8]);
    }
    h.update(root.sealed_at.timestamp_micros().to_be_bytes());
    let d = h.finalize();
    d.to_vec()
}

// ─────────────────────────────────────────────────────────────────────
//  Errors
// ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum SealError {
    #[error("chain store error : {0}")]
    Store(#[from] ChainStoreError),

    #[error("merkle tree error : {0}")]
    Merkle(#[from] MerkleError),

    #[error("HMAC key source error : {0}")]
    Key(String),

    #[error("trace {0} is not in any sealed epoch")]
    TraceNotInSealedEpoch(Uuid),

    #[error("internal : {0}")]
    Internal(String),
}

// ─────────────────────────────────────────────────────────────────────
//  EpochSealer
// ─────────────────────────────────────────────────────────────────────

/// Configuration for the sealer.
#[derive(Debug, Clone)]
pub struct EpochSealerConfig {
    /// Target number of traces per epoch.  Default 1 024.  The last
    /// epoch in any sealing pass may be smaller (≥ 1).
    pub epoch_size_traces: usize,
}

impl Default for EpochSealerConfig {
    fn default() -> Self {
        Self {
            epoch_size_traces: 1024,
        }
    }
}

/// Orchestrates Merkle epoch sealing.
///
/// Holds the read side ([`ChainStore`]) + write side
/// ([`EpochRootStore`]) + the per-tenant HMAC key source for signing
/// the epoch row.  Per-tenant serialization is enforced by an
/// internal `Mutex` map.
pub struct EpochSealer<S, R, K>
where
    S: ChainStore + ?Sized,
    R: EpochRootStore + ?Sized,
    K: SealerKeySource + ?Sized,
{
    chain_store: Arc<S>,
    epoch_store: Arc<R>,
    key_source: Arc<K>,
    config: EpochSealerConfig,
    /// Per-tenant lock to serialize sealing within a tenant.
    locks: Mutex<HashMap<TenantId, Arc<Mutex<()>>>>,
}

/// **Sprint 7 — N9** : key source for epoch-root signatures.  In
/// production this is wired to the existing
/// [`crate::audit::hkdf::HmacKeySource`] so epoch rows are signed by
/// the same per-tenant key as the linear HMAC chain.
///
/// Sync to match `HmacKeySource::derive` (HKDF is in-process, no I/O
/// in the Sprint 6 impl).  A Phase 4 Vault Transit impl can wrap an
/// async backend behind `tokio::task::block_in_place` if needed.
pub trait SealerKeySource: Send + Sync + 'static {
    fn derive_for(&self, tenant: TenantId) -> Result<Vec<u8>, SealError>;
}

impl SealerKeySource for crate::audit::hkdf::HmacKeySource {
    fn derive_for(&self, tenant: TenantId) -> Result<Vec<u8>, SealError> {
        self.derive(tenant)
            .map_err(|e| SealError::Key(format!("{e:?}")))
    }
}

impl<S, R, K> EpochSealer<S, R, K>
where
    S: ChainStore + ?Sized,
    R: EpochRootStore + ?Sized,
    K: SealerKeySource + ?Sized,
{
    pub fn new(
        chain_store: Arc<S>,
        epoch_store: Arc<R>,
        key_source: Arc<K>,
        config: EpochSealerConfig,
    ) -> Self {
        Self {
            chain_store,
            epoch_store,
            key_source,
            config,
            locks: Mutex::new(HashMap::new()),
        }
    }

    /// Acquire (and lazily create) the per-tenant sealing lock.
    async fn tenant_lock(&self, tenant: TenantId) -> Arc<Mutex<()>> {
        let mut locks = self.locks.lock().await;
        Arc::clone(locks.entry(tenant).or_insert_with(|| Arc::new(Mutex::new(()))))
    }

    /// Seal all pending epochs for `tenant`.
    ///
    /// Walks the tenant's HMAC chain from the trace immediately after
    /// the last sealed `end_trace_id` ; groups into epochs of
    /// `config.epoch_size_traces` ; for each full epoch, builds the
    /// Merkle tree, signs the row, appends to the [`EpochRootStore`].
    ///
    /// Returns the [`EpochRoot`]s that were newly sealed in this call.
    /// An empty return is normal (chain has fewer traces than one
    /// epoch's worth since the last sealing).
    pub async fn seal_pending_epochs(
        &self,
        tenant: TenantId,
    ) -> Result<Vec<EpochRoot>, SealError> {
        let lock = self.tenant_lock(tenant).await;
        let _guard = lock.lock().await;

        let latest = self
            .epoch_store
            .latest_for_tenant(tenant)
            .await?;
        let (mut next_epoch_id, mut previous_root, start_after) = match latest.as_ref() {
            Some(prev) => (prev.epoch_id + 1, Some(prev.root_hash), Some(prev.end_trace_id)),
            None => (1u64, None, None),
        };

        // Load the full chain.  Phase 4 Postgres impl will paginate.
        let full_chain = self.chain_store.chain_for_tenant(tenant, None).await?;

        // Slice off the already-sealed prefix.
        let mut start_idx = 0usize;
        if let Some(boundary) = start_after {
            // Walk forward to the entry just AFTER `boundary`.
            let pos = full_chain
                .iter()
                .position(|e| e.trace.trace_id.0 == boundary)
                .ok_or_else(|| {
                    SealError::Internal(format!(
                        "previously-sealed end_trace_id {boundary} not found in chain"
                    ))
                })?;
            start_idx = pos + 1;
        }

        let pending = &full_chain[start_idx..];
        let mut newly_sealed: Vec<EpochRoot> = Vec::new();

        for chunk in pending.chunks(self.config.epoch_size_traces) {
            // Only seal full epochs.  A partial last chunk waits for
            // more traces — sealing a 1-trace epoch is wasteful.
            // Caller can force-seal via `seal_partial_epoch` (below).
            if chunk.len() < self.config.epoch_size_traces {
                break;
            }
            let root = self
                .seal_chunk(tenant, next_epoch_id, previous_root, chunk)
                .await?;
            self.epoch_store.append(root.clone()).await?;
            previous_root = Some(root.root_hash);
            next_epoch_id += 1;
            newly_sealed.push(root);
        }

        Ok(newly_sealed)
    }

    /// Force-seal whatever's pending, even if it's smaller than
    /// `epoch_size_traces`.  Used for graceful shutdown and tests.
    pub async fn seal_partial_epoch(
        &self,
        tenant: TenantId,
    ) -> Result<Option<EpochRoot>, SealError> {
        let lock = self.tenant_lock(tenant).await;
        let _guard = lock.lock().await;

        let latest = self.epoch_store.latest_for_tenant(tenant).await?;
        let (next_epoch_id, previous_root, start_after) = match latest.as_ref() {
            Some(prev) => (prev.epoch_id + 1, Some(prev.root_hash), Some(prev.end_trace_id)),
            None => (1u64, None, None),
        };

        let full_chain = self.chain_store.chain_for_tenant(tenant, None).await?;
        let start_idx = match start_after {
            None => 0,
            Some(b) => full_chain
                .iter()
                .position(|e| e.trace.trace_id.0 == b)
                .ok_or_else(|| {
                    SealError::Internal(format!("missing previous end_trace_id {b}"))
                })?
                + 1,
        };

        let pending = &full_chain[start_idx..];
        if pending.is_empty() {
            return Ok(None);
        }
        let root = self
            .seal_chunk(tenant, next_epoch_id, previous_root, pending)
            .await?;
        self.epoch_store.append(root.clone()).await?;
        Ok(Some(root))
    }

    /// Build the Merkle tree + sign one chunk into an [`EpochRoot`].
    async fn seal_chunk(
        &self,
        tenant: TenantId,
        epoch_id: u64,
        previous_root: Option<[u8; 32]>,
        chunk: &[ChainEntry],
    ) -> Result<EpochRoot, SealError> {
        // Leaves are the per-trace `payload_hash`es — already SHA-256
        // of the canonical row body (Sprint 6).
        let leaves: Vec<Vec<u8>> = chunk.iter().map(|e| e.payload_hash.to_vec()).collect();
        let tree = build_tree(&leaves)?;
        let root_hash = *tree.root();

        let key = self
            .key_source
            .derive_for(tenant)
            .map_err(|e| SealError::Key(format!("{e:?}")))?;

        let mut row = EpochRoot {
            tenant_id: tenant,
            epoch_id,
            start_trace_id: chunk.first().unwrap().trace.trace_id.0,
            end_trace_id: chunk.last().unwrap().trace.trace_id.0,
            leaf_count: chunk.len() as u32,
            root_hash,
            previous_epoch_root: previous_root,
            sealed_at: Utc::now(),
            // Filled in next step.
            signature: vec![],
        };
        let body = canonical_root_body(&row);
        row.signature = hmac_sign(&key, &body);
        Ok(row)
    }

    /// Build an inclusion proof for a specific trace.
    ///
    /// Returns the proof + the [`EpochRoot`] that contains the trace,
    /// or [`SealError::TraceNotInSealedEpoch`] if the trace doesn't
    /// belong to any sealed epoch yet.
    pub async fn inclusion_proof_for_trace(
        &self,
        tenant: TenantId,
        trace_id: Uuid,
    ) -> Result<(InclusionProof, EpochRoot), SealError> {
        let chain = self.chain_store.chain_for_tenant(tenant, None).await?;
        let epochs = self.epoch_store.chain_for_tenant(tenant).await?;

        // Find which epoch the trace is in by walking the boundaries.
        // Each epoch is `chain[start_offset..start_offset+leaf_count]`.
        let mut offset = 0usize;
        for epoch in &epochs {
            let end = offset + epoch.leaf_count as usize;
            if end > chain.len() {
                return Err(SealError::Internal(format!(
                    "epoch {} extends past chain end (offset {} + count {} > len {})",
                    epoch.epoch_id,
                    offset,
                    epoch.leaf_count,
                    chain.len()
                )));
            }
            let slice = &chain[offset..end];
            // Search for the trace within this slice.
            if let Some(rel_idx) = slice.iter().position(|e| e.trace.trace_id.0 == trace_id) {
                // Rebuild the tree for THIS epoch only.
                let leaves: Vec<Vec<u8>> =
                    slice.iter().map(|e| e.payload_hash.to_vec()).collect();
                let tree: MerkleTree = build_tree(&leaves)?;
                let proof = tree.inclusion_proof(rel_idx)?;
                return Ok((proof, epoch.clone()));
            }
            offset = end;
        }
        Err(SealError::TraceNotInSealedEpoch(trace_id))
    }

    /// Verify the **full chain of epoch roots** for a tenant :
    /// 1. Each `previous_epoch_root` matches the actual previous
    ///    epoch's `root_hash`.
    /// 2. Each row's `signature` validates against the tenant's HMAC
    ///    key.
    /// 3. Each row's `root_hash` matches the Merkle tree rebuilt from
    ///    the underlying traces in [`ChainStore`].
    pub async fn verify_epoch_chain(&self, tenant: TenantId) -> Result<(), SealError> {
        let epochs = self.epoch_store.chain_for_tenant(tenant).await?;
        if epochs.is_empty() {
            return Ok(());
        }
        let chain = self.chain_store.chain_for_tenant(tenant, None).await?;
        let key = self
            .key_source
            .derive_for(tenant)
            .map_err(|e| SealError::Key(format!("{e:?}")))?;

        let mut offset = 0usize;
        let mut prev_root: Option<[u8; 32]> = None;
        for (i, epoch) in epochs.iter().enumerate() {
            // 1. Link-back.
            if epoch.previous_epoch_root != prev_root {
                return Err(SealError::Internal(format!(
                    "epoch {} previous_epoch_root linkback mismatch",
                    epoch.epoch_id
                )));
            }
            // 2. Signature.
            let body = canonical_root_body(epoch);
            let expected = hmac_sign(&key, &body);
            if expected != epoch.signature {
                return Err(SealError::Internal(format!(
                    "epoch {} signature invalid",
                    epoch.epoch_id
                )));
            }
            // 3. Root hash matches rebuilt tree.
            let end = offset + epoch.leaf_count as usize;
            if end > chain.len() {
                return Err(SealError::Internal(format!(
                    "epoch {} extends past chain end",
                    epoch.epoch_id
                )));
            }
            let slice = &chain[offset..end];
            let leaves: Vec<Vec<u8>> = slice.iter().map(|e| e.payload_hash.to_vec()).collect();
            let rebuilt = crate::audit::merkle::tree::build_root(&leaves)?;
            if rebuilt != epoch.root_hash {
                return Err(SealError::Internal(format!(
                    "epoch {} root_hash != rebuilt tree (drift at index {i})",
                    epoch.epoch_id
                )));
            }
            offset = end;
            prev_root = Some(epoch.root_hash);
        }
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Audit-grade inclusion verifier
// ─────────────────────────────────────────────────────────────────────

/// Given a leaf payload (the trace's `payload_hash`), an
/// [`InclusionProof`], and the trusted [`EpochRoot.root_hash`] from
/// the notary attestation, verify the trace is part of that epoch.
///
/// This is the **audit-grade** verification entry point :
/// - The auditor gets `(EpochRoot, NotaryReceipt)` from a trustworthy
///   source (Sigstore Rekor in Phase 4, the operator's signed list
///   in Sprint 7).
/// - They get the [`InclusionProof`] for a specific trace they care
///   about.
/// - They run this function to confirm the trace was committed to the
///   notarised root.
///
/// **Independent of any EFE-internal state** : works with proof bytes
/// + root bytes alone, no SQL access required.
pub fn verify_trace_in_epoch(
    leaf_payload: &[u8; 32],
    proof: &InclusionProof,
    expected_root: &[u8; 32],
) -> Result<(), SealError> {
    // 1. Confirm the proof's leaf_hash matches what we'd compute from
    //    the supplied payload — defends against the auditor being
    //    handed a proof for a DIFFERENT trace.
    let expected_leaf = leaf_hash(leaf_payload);
    if proof.leaf_hash != expected_leaf {
        return Err(SealError::Internal(
            "proof leaf_hash does not match supplied payload".into(),
        ));
    }
    // 2. Confirm the proof's claimed root matches the trusted root.
    if proof.root != *expected_root {
        return Err(SealError::Internal(
            "proof root_hash does not match supplied epoch root".into(),
        ));
    }
    // 3. Cryptographic verification of the proof itself.
    verify_inclusion(proof).map_err(SealError::Merkle)?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────
//  HMAC helpers
// ─────────────────────────────────────────────────────────────────────

fn hmac_sign(key: &[u8], body: &[u8]) -> Vec<u8> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac =
        <Hmac<Sha256>>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(body);
    mac.finalize().into_bytes().to_vec()
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::chain_store::{ChainEntry, InMemoryChainStore};
    use crate::audit::hkdf::HmacKeySource;
    use crate::audit::merkle::store::InMemoryEpochRootStore;
    use crate::audit::NanoAgentTrace;
    use efe_nano_sdk::{NanoAgentId, NanoAgentVersion, TenantId, TraceId};

    fn fake_trace(swarm: Uuid, idx: u32, tenant: TenantId) -> NanoAgentTrace {
        NanoAgentTrace {
            trace_id: TraceId(Uuid::new_v4()),
            parent_trace_ids: vec![],
            swarm_request_id: swarm,
            operation_id: format!("op-{idx}"),
            tenant_id: tenant,
            caller_agent_id: None,
            nano_agent_id: NanoAgentId::new("json_validate").unwrap(),
            nano_agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
            execution_mode: "pure_rust".into(),
            input_hash_source: [idx as u8; 32],
            input_hash_resolved: [idx as u8; 32],
            output_hash: Some([(idx as u8).wrapping_add(1); 32]),
            error_kind: None,
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
            latency_ms: Some(1),
            cost_cents: 0.0001,
            llm_tokens_used: None,
            full_input: None,
            full_output: None,
            tombstone: None,
            swarm_envelope_trace_id: None,
        }
    }

    async fn populate_chain(
        chain_store: &Arc<InMemoryChainStore>,
        tenant: TenantId,
        count: u32,
    ) -> Vec<ChainEntry> {
        use crate::audit::hmac_chain::HmacChainAuditWriter;
        use crate::audit::AuditWriter;
        let writer = HmacChainAuditWriter::new(
            Arc::clone(chain_store) as Arc<dyn ChainStore>,
            HmacKeySource::test_key(),
        );
        let swarm = Uuid::new_v4();
        for i in 0..count {
            writer.record(fake_trace(swarm, i, tenant)).await;
        }
        chain_store.chain_for_tenant(tenant, None).await.unwrap()
    }

    fn make_sealer(
        chain_store: Arc<InMemoryChainStore>,
        epoch_store: Arc<InMemoryEpochRootStore>,
        epoch_size: usize,
    ) -> EpochSealer<InMemoryChainStore, InMemoryEpochRootStore, HmacKeySource> {
        EpochSealer::new(
            chain_store,
            epoch_store,
            Arc::new(HmacKeySource::test_key()),
            EpochSealerConfig {
                epoch_size_traces: epoch_size,
            },
        )
    }

    // ─────────────────────────────────────────────────────────────────
    //  Basic sealing
    // ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn seal_no_traces_returns_empty() {
        let chain = Arc::new(InMemoryChainStore::new());
        let epochs = Arc::new(InMemoryEpochRootStore::new());
        let sealer = make_sealer(chain, epochs, 4);
        let sealed = sealer.seal_pending_epochs(TenantId::system()).await.unwrap();
        assert!(sealed.is_empty());
    }

    #[tokio::test]
    async fn seal_partial_chunk_skipped_until_full() {
        let chain = Arc::new(InMemoryChainStore::new());
        let epochs = Arc::new(InMemoryEpochRootStore::new());
        let t = TenantId::system();
        let _ = populate_chain(&chain, t, 3).await; // < epoch_size = 4
        let sealer = make_sealer(chain, epochs.clone(), 4);
        let sealed = sealer.seal_pending_epochs(t).await.unwrap();
        assert!(sealed.is_empty());
        assert_eq!(epochs.total_sealed_count(), 0);
    }

    #[tokio::test]
    async fn seal_exact_one_epoch() {
        let chain = Arc::new(InMemoryChainStore::new());
        let epochs = Arc::new(InMemoryEpochRootStore::new());
        let t = TenantId::system();
        let _entries = populate_chain(&chain, t, 4).await;
        let sealer = make_sealer(chain, epochs.clone(), 4);
        let sealed = sealer.seal_pending_epochs(t).await.unwrap();
        assert_eq!(sealed.len(), 1);
        assert_eq!(sealed[0].epoch_id, 1);
        assert_eq!(sealed[0].leaf_count, 4);
        assert!(sealed[0].previous_epoch_root.is_none());
        assert_eq!(epochs.total_sealed_count(), 1);
    }

    #[tokio::test]
    async fn seal_two_epochs_links_correctly() {
        let chain = Arc::new(InMemoryChainStore::new());
        let epochs = Arc::new(InMemoryEpochRootStore::new());
        let t = TenantId::system();
        let _ = populate_chain(&chain, t, 8).await; // 2 epochs of 4
        let sealer = make_sealer(chain, epochs.clone(), 4);
        let sealed = sealer.seal_pending_epochs(t).await.unwrap();
        assert_eq!(sealed.len(), 2);
        // Epoch 2's previous_epoch_root MUST == epoch 1's root_hash.
        assert_eq!(sealed[1].previous_epoch_root, Some(sealed[0].root_hash));
    }

    #[tokio::test]
    async fn seal_resumes_from_last_sealed_end() {
        let chain = Arc::new(InMemoryChainStore::new());
        let epochs = Arc::new(InMemoryEpochRootStore::new());
        let t = TenantId::system();
        let _ = populate_chain(&chain, t, 4).await;
        let sealer = make_sealer(Arc::clone(&chain), epochs.clone(), 4);
        let first = sealer.seal_pending_epochs(t).await.unwrap();
        assert_eq!(first.len(), 1);
        // Add 4 more traces to the SAME chain (production-style).
        let _ = populate_chain(&chain, t, 4).await;
        let second = sealer.seal_pending_epochs(t).await.unwrap();
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].epoch_id, 2);
        assert_eq!(second[0].previous_epoch_root, Some(first[0].root_hash));
    }

    #[tokio::test]
    async fn seal_partial_force_seals_remaining() {
        let chain = Arc::new(InMemoryChainStore::new());
        let epochs = Arc::new(InMemoryEpochRootStore::new());
        let t = TenantId::system();
        let _ = populate_chain(&chain, t, 5).await; // 1 full epoch + 1 leftover
        let sealer = make_sealer(chain, epochs.clone(), 4);
        let _ = sealer.seal_pending_epochs(t).await.unwrap();
        // 1 full sealed, 1 leftover.
        let partial = sealer.seal_partial_epoch(t).await.unwrap().unwrap();
        assert_eq!(partial.epoch_id, 2);
        assert_eq!(partial.leaf_count, 1);
        assert_eq!(epochs.total_sealed_count(), 2);
    }

    #[tokio::test]
    async fn seal_partial_returns_none_when_caught_up() {
        let chain = Arc::new(InMemoryChainStore::new());
        let epochs = Arc::new(InMemoryEpochRootStore::new());
        let t = TenantId::system();
        let _ = populate_chain(&chain, t, 4).await;
        let sealer = make_sealer(chain, epochs.clone(), 4);
        sealer.seal_pending_epochs(t).await.unwrap();
        let again = sealer.seal_partial_epoch(t).await.unwrap();
        assert!(again.is_none());
    }

    // ─────────────────────────────────────────────────────────────────
    //  Inclusion proofs
    // ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn inclusion_proof_for_every_trace_in_sealed_epoch() {
        let chain = Arc::new(InMemoryChainStore::new());
        let epochs = Arc::new(InMemoryEpochRootStore::new());
        let t = TenantId::system();
        let entries = populate_chain(&chain, t, 4).await;
        let sealer = make_sealer(chain, epochs, 4);
        let sealed = sealer.seal_pending_epochs(t).await.unwrap();
        let expected_root = sealed[0].root_hash;
        for e in entries.iter() {
            let (proof, epoch) = sealer
                .inclusion_proof_for_trace(t, e.trace_id())
                .await
                .unwrap();
            assert_eq!(epoch.epoch_id, 1);
            verify_trace_in_epoch(&e.payload_hash, &proof, &expected_root).unwrap();
        }
    }

    #[tokio::test]
    async fn inclusion_proof_for_unsealed_trace_errors() {
        let chain = Arc::new(InMemoryChainStore::new());
        let epochs = Arc::new(InMemoryEpochRootStore::new());
        let t = TenantId::system();
        let entries = populate_chain(&chain, t, 3).await; // < epoch_size = 4
        let sealer = make_sealer(chain, epochs, 4);
        let target = entries[0].trace_id();
        let err = sealer
            .inclusion_proof_for_trace(t, target)
            .await
            .unwrap_err();
        match err {
            SealError::TraceNotInSealedEpoch(id) => assert_eq!(id, target),
            other => panic!("expected TraceNotInSealedEpoch, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn inclusion_proof_finds_trace_across_multiple_epochs() {
        let chain = Arc::new(InMemoryChainStore::new());
        let epochs = Arc::new(InMemoryEpochRootStore::new());
        let t = TenantId::system();
        let entries = populate_chain(&chain, t, 12).await; // 3 epochs of 4
        let sealer = make_sealer(chain, epochs, 4);
        let sealed = sealer.seal_pending_epochs(t).await.unwrap();
        assert_eq!(sealed.len(), 3);
        // First trace of epoch 2 = entries[4].
        let target = entries[4].trace_id();
        let (proof, epoch) = sealer
            .inclusion_proof_for_trace(t, target)
            .await
            .unwrap();
        assert_eq!(epoch.epoch_id, 2);
        verify_trace_in_epoch(&entries[4].payload_hash, &proof, &epoch.root_hash).unwrap();
    }

    #[tokio::test]
    async fn verify_trace_in_epoch_rejects_wrong_payload() {
        let chain = Arc::new(InMemoryChainStore::new());
        let epochs = Arc::new(InMemoryEpochRootStore::new());
        let t = TenantId::system();
        let entries = populate_chain(&chain, t, 4).await;
        let sealer = make_sealer(chain, epochs, 4);
        sealer.seal_pending_epochs(t).await.unwrap();
        let (proof, epoch) = sealer
            .inclusion_proof_for_trace(t, entries[2].trace_id())
            .await
            .unwrap();
        // Use the wrong payload bytes — must reject.
        let bad_payload = [0xffu8; 32];
        let err = verify_trace_in_epoch(&bad_payload, &proof, &epoch.root_hash).unwrap_err();
        assert!(err.to_string().contains("payload"));
    }

    #[tokio::test]
    async fn verify_trace_in_epoch_rejects_wrong_root() {
        let chain = Arc::new(InMemoryChainStore::new());
        let epochs = Arc::new(InMemoryEpochRootStore::new());
        let t = TenantId::system();
        let entries = populate_chain(&chain, t, 4).await;
        let sealer = make_sealer(chain, epochs, 4);
        sealer.seal_pending_epochs(t).await.unwrap();
        let (proof, _epoch) = sealer
            .inclusion_proof_for_trace(t, entries[0].trace_id())
            .await
            .unwrap();
        let bad_root = [0xddu8; 32];
        let err = verify_trace_in_epoch(&entries[0].payload_hash, &proof, &bad_root)
            .unwrap_err();
        assert!(err.to_string().contains("root"));
    }

    // ─────────────────────────────────────────────────────────────────
    //  Chain-of-roots verification
    // ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn verify_epoch_chain_succeeds_on_clean_chain() {
        let chain = Arc::new(InMemoryChainStore::new());
        let epochs = Arc::new(InMemoryEpochRootStore::new());
        let t = TenantId::system();
        let _ = populate_chain(&chain, t, 12).await;
        let sealer = make_sealer(chain, epochs, 4);
        sealer.seal_pending_epochs(t).await.unwrap();
        sealer.verify_epoch_chain(t).await.unwrap();
    }

    #[tokio::test]
    async fn verify_epoch_chain_detects_root_tamper() {
        let chain = Arc::new(InMemoryChainStore::new());
        let epochs = Arc::new(InMemoryEpochRootStore::new());
        let t = TenantId::system();
        let _ = populate_chain(&chain, t, 8).await;
        let sealer = make_sealer(Arc::clone(&chain), Arc::clone(&epochs), 4);
        sealer.seal_pending_epochs(t).await.unwrap();

        // Build a fresh sealer wired to a TAMPERED epoch store : we
        // mutate epoch 1's root_hash + recompute the signature so the
        // sig check passes but the root no longer matches the chain.
        let chain_pull = epochs.chain_for_tenant(t).await.unwrap();
        let mut tampered = chain_pull.clone();
        tampered[0].root_hash[0] ^= 0xff;
        // Re-sign so the signature check would otherwise pass.
        let key = HmacKeySource::test_key().derive_for(t).unwrap();
        let body = canonical_root_body(&tampered[0]);
        tampered[0].signature = hmac_sign(&key, &body);
        // We also have to tamper epoch 2's previous_epoch_root to
        // match — otherwise the linkback test catches it first.
        tampered[1].previous_epoch_root = Some(tampered[0].root_hash);
        let body2 = canonical_root_body(&tampered[1]);
        tampered[1].signature = hmac_sign(&key, &body2);

        let tampered_store = Arc::new(InMemoryEpochRootStore::new());
        for r in tampered {
            tampered_store.force_append_for_test(r);
        }
        let tampered_sealer = make_sealer(chain, tampered_store, 4);
        let err = tampered_sealer.verify_epoch_chain(t).await.unwrap_err();
        // The root_hash drift is detected on tree rebuild.
        assert!(err.to_string().contains("rebuilt"));
    }

    #[tokio::test]
    async fn verify_epoch_chain_detects_signature_tamper() {
        let chain = Arc::new(InMemoryChainStore::new());
        let epochs = Arc::new(InMemoryEpochRootStore::new());
        let t = TenantId::system();
        let _ = populate_chain(&chain, t, 4).await;
        let sealer = make_sealer(Arc::clone(&chain), Arc::clone(&epochs), 4);
        sealer.seal_pending_epochs(t).await.unwrap();

        let mut tampered = epochs.chain_for_tenant(t).await.unwrap();
        tampered[0].signature[0] ^= 0xff;
        let tampered_store = Arc::new(InMemoryEpochRootStore::new());
        for r in tampered {
            tampered_store.force_append_for_test(r);
        }
        let tampered_sealer = make_sealer(chain, tampered_store, 4);
        let err = tampered_sealer.verify_epoch_chain(t).await.unwrap_err();
        assert!(err.to_string().contains("signature"));
    }

    #[tokio::test]
    async fn verify_epoch_chain_detects_linkback_tamper() {
        // Defense-in-depth : a compromised operator that bypasses
        // `EpochRootStore::append`'s validation (e.g. by writing to
        // Postgres directly with admin creds) should still be caught
        // by `verify_epoch_chain` reading from the same store.
        //
        // We simulate that by using `force_append_for_test` to
        // smuggle tampered rows past the store's monotonic-linkback
        // check.
        let chain = Arc::new(InMemoryChainStore::new());
        let epochs = Arc::new(InMemoryEpochRootStore::new());
        let t = TenantId::system();
        let _ = populate_chain(&chain, t, 8).await;
        let sealer = make_sealer(Arc::clone(&chain), Arc::clone(&epochs), 4);
        sealer.seal_pending_epochs(t).await.unwrap();

        let mut tampered = epochs.chain_for_tenant(t).await.unwrap();
        // Break the linkback : pretend epoch 2's previous was [0xff;32].
        tampered[1].previous_epoch_root = Some([0xff; 32]);
        // Re-sign so we get past the signature check first.
        let key = HmacKeySource::test_key().derive_for(t).unwrap();
        let body = canonical_root_body(&tampered[1]);
        tampered[1].signature = hmac_sign(&key, &body);

        let tampered_store = Arc::new(InMemoryEpochRootStore::new());
        // Use the test backdoor — bypasses the store's validation,
        // simulating a Postgres direct-write attack.
        for r in tampered {
            tampered_store.force_append_for_test(r);
        }
        let tampered_sealer = make_sealer(chain, tampered_store, 4);
        let err = tampered_sealer.verify_epoch_chain(t).await.unwrap_err();
        assert!(err.to_string().contains("linkback"));
    }

    // ─────────────────────────────────────────────────────────────────
    //  Per-tenant isolation + concurrent sealing
    // ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn cross_tenant_sealing_is_independent() {
        let chain = Arc::new(InMemoryChainStore::new());
        let epochs = Arc::new(InMemoryEpochRootStore::new());
        let t1 = TenantId::new();
        let t2 = TenantId::new();
        let _ = populate_chain(&chain, t1, 4).await;
        let _ = populate_chain(&chain, t2, 8).await;
        let sealer = make_sealer(chain, epochs.clone(), 4);
        sealer.seal_pending_epochs(t1).await.unwrap();
        sealer.seal_pending_epochs(t2).await.unwrap();
        // Each tenant has its own monotonic sequence.
        let chain1 = epochs.chain_for_tenant(t1).await.unwrap();
        let chain2 = epochs.chain_for_tenant(t2).await.unwrap();
        assert_eq!(chain1.len(), 1);
        assert_eq!(chain2.len(), 2);
        // Roots are different (different inputs).
        assert_ne!(chain1[0].root_hash, chain2[0].root_hash);
    }

    #[tokio::test]
    async fn canonical_root_body_is_deterministic() {
        let r = EpochRoot {
            tenant_id: TenantId::system(),
            epoch_id: 7,
            start_trace_id: Uuid::from_u128(1),
            end_trace_id: Uuid::from_u128(2),
            leaf_count: 1024,
            root_hash: [0xab; 32],
            previous_epoch_root: Some([0xcd; 32]),
            sealed_at: chrono::DateTime::parse_from_rfc3339("2026-05-19T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            signature: vec![],
        };
        assert_eq!(canonical_root_body(&r), canonical_root_body(&r));
    }

    #[tokio::test]
    async fn canonical_root_body_changes_per_field() {
        let base = EpochRoot {
            tenant_id: TenantId::system(),
            epoch_id: 1,
            start_trace_id: Uuid::from_u128(1),
            end_trace_id: Uuid::from_u128(2),
            leaf_count: 4,
            root_hash: [0xab; 32],
            previous_epoch_root: None,
            sealed_at: Utc::now(),
            signature: vec![],
        };
        let h_base = canonical_root_body(&base);
        let mut differ = base.clone();
        differ.epoch_id = 2;
        differ.previous_epoch_root = Some([0xab; 32]);
        assert_ne!(canonical_root_body(&differ), h_base);
    }
}
