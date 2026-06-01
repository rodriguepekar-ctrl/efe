// SPDX-License-Identifier: UNLICENSED
//! **Sprint 9 — N16** : Cross-region chain shipment protocol.
//!
//! Architecture § 7.3.  Sprint 9 ships an **in-memory in-process**
//! implementation suitable for tests + the single-region demo binary.
//! The real gRPC over mTLS implementation lands in Sprint 10 behind
//! the same [`ChainShipmentClient`] trait.
//!
//! # Atomic two-phase protocol (G4 + I17)
//!
//! ```text
//! 1. POST /chain/stage   { swarm_request_id, chain_bytes, head_hash }
//! 2. GET  /chain/verify  { staging_id } → { verified: bool }
//! 3. POST /chain/commit  { staging_id }
//! 4. POST /chain/abort   { staging_id }      ← idempotent rollback
//! ```
//!
//! Failure at any phase → staging area GC'd ; target NEVER reads
//! from a staged-but-uncommitted chain.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use dashmap::DashMap;
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use efe_nano_sdk::{RegionCode, TenantId, REPLAY_SCHEMA_VERSION};

// ─────────────────────────────────────────────────────────────────────
//  Wire types
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N16** : a single trace row shipped across regions.
#[derive(Debug, Clone, PartialEq)]
pub struct ShippedTraceRow {
    pub trace_id: Uuid,
    /// Canonical row body (Sprint 6 format) — opaque bytes.
    pub canonical_row_body: Vec<u8>,
    /// HMAC computed over the canonical row body.
    pub hmac: Vec<u8>,
    /// KMS ARNs referenced by encrypted bodies in this row.
    pub kms_key_arns: Vec<String>,
}

/// **Sprint 9 — N16** : a single body BLOB (full_input / full_output)
/// shipped alongside its trace row.
#[derive(Debug, Clone, PartialEq)]
pub struct ShippedBodyBlob {
    pub trace_id: Uuid,
    /// `"full_input"` or `"full_output"`.
    pub field: String,
    /// Encrypted body bytes.
    pub ciphertext: Vec<u8>,
    /// The KMS ARN that encrypted this body.
    pub kms_key_arn: String,
}

/// **Sprint 9 — N16** : the complete shipped chain ; assembled on the
/// source side, verified on the target side.
#[derive(Debug, Clone)]
pub struct ShippedChain {
    pub source_region: RegionCode,
    pub target_region: RegionCode,
    pub tenant_id: TenantId,
    pub swarm_request_id: Uuid,
    pub envelope_trace_id: Uuid,
    pub sdk_schema_version: u32,
    /// Per-tenant HKDF-derived key, signed by the source region's
    /// signing root.  Target uses this to verify the chain locally
    /// without needing cross-region master-key access (G2 + I2).
    pub derived_hmac_key: Vec<u8>,
    /// Signature over `(derived_hmac_key || tenant_id || sdk_schema)`
    /// by the source region's signing root.  Sprint 9 uses HMAC-SHA256 ;
    /// Sprint 10 will move to RSA-PSS via Vault Transit.
    pub derived_hmac_key_sig: Vec<u8>,
    /// `Blake3` digest over the canonical-row-body bytes — verified
    /// pre-replay (G4 + I18).
    pub head_hash: Vec<u8>,
    pub trace_rows: Vec<ShippedTraceRow>,
    pub body_blobs: Vec<ShippedBodyBlob>,
}

impl ShippedChain {
    /// **N16** : Number of trace rows in the chain.
    pub fn row_count(&self) -> usize {
        self.trace_rows.len()
    }

    /// **N16** : Re-compute the head hash from the row bodies and
    /// compare against the carried `head_hash`.  Returns true if the
    /// chain is internally consistent (I18).
    ///
    /// Implementation : SHA-256 over `concat(row.canonical_row_body)`.
    /// `Blake3` referenced in the architecture is the canonical
    /// choice for production ; Sprint 9 uses SHA-256 to avoid a
    /// new crate dep, matching the rest of the codebase.
    pub fn verify_head_hash(&self) -> bool {
        let mut hasher = Sha256::new();
        for row in &self.trace_rows {
            hasher.update(&row.canonical_row_body);
        }
        let expected = hasher.finalize();
        expected.as_slice() == self.head_hash.as_slice()
    }

    /// **N16** : Collect every distinct KMS ARN referenced by the
    /// chain.  Used by the coordinator's pre-flight check (G12).
    pub fn distinct_kms_arns(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        for row in &self.trace_rows {
            for arn in &row.kms_key_arns {
                seen.insert(arn.clone());
            }
        }
        for body in &self.body_blobs {
            seen.insert(body.kms_key_arn.clone());
        }
        let mut out: Vec<String> = seen.into_iter().collect();
        out.sort();
        out
    }
}

// ─────────────────────────────────────────────────────────────────────
//  ChainShipmentError
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N16** : closed taxonomy of shipment errors.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ChainShipmentError {
    /// Source region not reachable (F1).
    #[error("source region {region} unavailable")]
    SourceUnavailable { region: RegionCode },

    /// Source has no chain for `(tenant, swarm_request_id)`.
    #[error("chain not found for swarm {swarm_request_id} in {region}")]
    ChainNotFound {
        region: RegionCode,
        swarm_request_id: Uuid,
    },

    /// Schema version mismatch (F16).
    #[error("schema too new : source {source_version}, target max {target_max}")]
    SchemaTooNew { source_version: u32, target_max: u32 },

    /// Head hash mismatch (F3).
    #[error("head hash mismatch — chain integrity broken")]
    HeadHashMismatch,

    /// Staging area not found or expired.
    #[error("staging {staging_id} not found")]
    StagingNotFound { staging_id: Uuid },

    /// Generic transport / backend error.
    #[error("transport error : {0}")]
    Transport(String),
}

// ─────────────────────────────────────────────────────────────────────
//  ChainShipmentClient trait
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N16** : client-side chain shipment.  Used by the
/// coordinator's worker.
///
/// The protocol is two-phase :
/// 1. `fetch_and_stage` — fetches the chain into a staging area.
/// 2. `verify_and_commit` — atomically promotes staging → live.
/// 3. `abort` — rollback the staging area.
#[async_trait]
pub trait ChainShipmentClient: Send + Sync + 'static {
    /// **N16** : Stage the chain.  Returns a `staging_id` that
    /// subsequent calls use.
    async fn fetch_and_stage(
        &self,
        source_region: RegionCode,
        target_region: RegionCode,
        tenant_id: TenantId,
        swarm_request_id: Uuid,
    ) -> Result<StagedChainHandle, ChainShipmentError>;

    /// **N16** : Verify head hash + commit the staging area.  Returns
    /// the fully shipped chain.
    async fn verify_and_commit(
        &self,
        handle: &StagedChainHandle,
    ) -> Result<ShippedChain, ChainShipmentError>;

    /// **N16** : Abort the staging area (idempotent).
    async fn abort(&self, handle: &StagedChainHandle) -> Result<(), ChainShipmentError>;
}

/// **Sprint 9 — N16** : opaque handle to a staged chain.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StagedChainHandle {
    pub staging_id: Uuid,
    pub source_region: RegionCode,
    pub target_region: RegionCode,
    pub tenant_id: TenantId,
    pub swarm_request_id: Uuid,
}

// ─────────────────────────────────────────────────────────────────────
//  InMemoryChainShipmentClient — Sprint 9 ship
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N16** : in-memory chain shipment.  Source region
/// "exports" chains via [`Self::publish`] ; target region "fetches"
/// via the trait API.
///
/// Stages live for [`Self::STAGING_TTL`] (5 minutes by default).
pub struct InMemoryChainShipmentClient {
    /// (source_region, tenant_id, swarm_request_id) → chain.
    published: DashMap<(RegionCode, TenantId, Uuid), ShippedChain>,
    /// staging_id → (chain, staged_at).
    staged: DashMap<Uuid, (ShippedChain, SystemTime)>,
    /// Configured staging TTL.
    staging_ttl: Duration,
}

impl std::fmt::Debug for InMemoryChainShipmentClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryChainShipmentClient")
            .field("published_count", &self.published.len())
            .field("staged_count", &self.staged.len())
            .field("staging_ttl", &self.staging_ttl)
            .finish()
    }
}

impl InMemoryChainShipmentClient {
    /// **N16** : Default staging TTL (architecture § 7.3).
    pub const STAGING_TTL: Duration = Duration::from_secs(5 * 60);

    /// **N16** : Construct an empty client.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            published: DashMap::new(),
            staged: DashMap::new(),
            staging_ttl: Self::STAGING_TTL,
        })
    }

    /// **N16** : Construct with a custom staging TTL.
    pub fn with_staging_ttl(ttl: Duration) -> Arc<Self> {
        Arc::new(Self {
            published: DashMap::new(),
            staged: DashMap::new(),
            staging_ttl: ttl,
        })
    }

    /// **N16** : Publish a chain so that future `fetch_and_stage`
    /// calls for `(source_region, tenant_id, swarm_request_id)` can
    /// find it.  Test convenience — production fetches from the
    /// source region's Postgres directly.
    pub fn publish(&self, chain: ShippedChain) {
        let key = (chain.source_region, chain.tenant_id, chain.swarm_request_id);
        self.published.insert(key, chain);
    }

    /// **N16** : Sanity helper for tests — synthesize a minimal
    /// chain with one trace row.
    #[allow(clippy::too_many_arguments)] // builder fields are unavoidable
    pub fn build_test_chain(
        source_region: RegionCode,
        target_region: RegionCode,
        tenant_id: TenantId,
        swarm_request_id: Uuid,
        envelope_trace_id: Uuid,
        rows: Vec<ShippedTraceRow>,
        body_blobs: Vec<ShippedBodyBlob>,
    ) -> ShippedChain {
        let mut hasher = Sha256::new();
        for row in &rows {
            hasher.update(&row.canonical_row_body);
        }
        let head_hash = hasher.finalize().to_vec();
        // Derived key + signature are stubs — Sprint 10 replaces.
        ShippedChain {
            source_region,
            target_region,
            tenant_id,
            swarm_request_id,
            envelope_trace_id,
            sdk_schema_version: REPLAY_SCHEMA_VERSION,
            derived_hmac_key: vec![0u8; 32],
            derived_hmac_key_sig: vec![0u8; 64],
            head_hash,
            trace_rows: rows,
            body_blobs,
        }
    }

    /// **N16** : GC staging areas older than TTL.  Returns the number
    /// of areas removed.  Called periodically by background tasks
    /// in production ; explicit in tests.
    pub fn gc_stale_staging(&self) -> usize {
        let now = SystemTime::now();
        let ttl = self.staging_ttl;
        let mut to_drop = Vec::new();
        for r in self.staged.iter() {
            if now
                .duration_since(r.value().1)
                .map(|elapsed| elapsed >= ttl)
                .unwrap_or(false)
            {
                to_drop.push(*r.key());
            }
        }
        let n = to_drop.len();
        for k in to_drop {
            self.staged.remove(&k);
        }
        n
    }

    /// **N16** : Number of currently-published chains.
    pub fn published_count(&self) -> usize {
        self.published.len()
    }

    /// **N16** : Number of currently-staged chains.
    pub fn staged_count(&self) -> usize {
        self.staged.len()
    }
}

#[async_trait]
impl ChainShipmentClient for InMemoryChainShipmentClient {
    async fn fetch_and_stage(
        &self,
        source_region: RegionCode,
        target_region: RegionCode,
        tenant_id: TenantId,
        swarm_request_id: Uuid,
    ) -> Result<StagedChainHandle, ChainShipmentError> {
        let key = (source_region, tenant_id, swarm_request_id);
        let chain = self
            .published
            .get(&key)
            .ok_or(ChainShipmentError::ChainNotFound {
                region: source_region,
                swarm_request_id,
            })?
            .clone();
        // Schema version check (architecture G10 + I19 + F16).
        if chain.sdk_schema_version > REPLAY_SCHEMA_VERSION {
            return Err(ChainShipmentError::SchemaTooNew {
                source_version: chain.sdk_schema_version,
                target_max: REPLAY_SCHEMA_VERSION,
            });
        }
        let staging_id = Uuid::new_v4();
        let mut staged_chain = chain;
        staged_chain.target_region = target_region;
        self.staged
            .insert(staging_id, (staged_chain, SystemTime::now()));
        Ok(StagedChainHandle {
            staging_id,
            source_region,
            target_region,
            tenant_id,
            swarm_request_id,
        })
    }

    async fn verify_and_commit(
        &self,
        handle: &StagedChainHandle,
    ) -> Result<ShippedChain, ChainShipmentError> {
        let (chain, _staged_at) = self
            .staged
            .remove(&handle.staging_id)
            .ok_or(ChainShipmentError::StagingNotFound {
                staging_id: handle.staging_id,
            })?
            .1;
        // Head-hash verification (G4 + I18).
        if !chain.verify_head_hash() {
            return Err(ChainShipmentError::HeadHashMismatch);
        }
        Ok(chain)
    }

    async fn abort(&self, handle: &StagedChainHandle) -> Result<(), ChainShipmentError> {
        // Idempotent — removing a missing staging is OK.
        self.staged.remove(&handle.staging_id);
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_chain(
        source: RegionCode,
        target: RegionCode,
        tenant: TenantId,
    ) -> ShippedChain {
        let swarm = Uuid::new_v4();
        let env = Uuid::new_v4();
        let row = ShippedTraceRow {
            trace_id: Uuid::new_v4(),
            canonical_row_body: b"row-1-body".to_vec(),
            hmac: vec![0u8; 32],
            kms_key_arns: vec!["arn:test:eu-west-1".into()],
        };
        let body = ShippedBodyBlob {
            trace_id: row.trace_id,
            field: "full_input".into(),
            ciphertext: b"encrypted".to_vec(),
            kms_key_arn: "arn:test:eu-west-1".into(),
        };
        InMemoryChainShipmentClient::build_test_chain(
            source,
            target,
            tenant,
            swarm,
            env,
            vec![row],
            vec![body],
        )
    }

    #[test]
    fn verify_head_hash_succeeds_on_valid_chain() {
        let c = sample_chain(RegionCode::EuWest1, RegionCode::EuWest1, TenantId::new());
        assert!(c.verify_head_hash());
    }

    #[test]
    fn verify_head_hash_fails_on_corrupted_body() {
        let mut c = sample_chain(RegionCode::EuWest1, RegionCode::EuWest1, TenantId::new());
        c.trace_rows[0].canonical_row_body = b"tampered".to_vec();
        assert!(!c.verify_head_hash());
    }

    #[test]
    fn distinct_kms_arns_dedupes() {
        let tenant = TenantId::new();
        let mut c = sample_chain(RegionCode::EuWest1, RegionCode::EuWest1, tenant);
        // Add a second row referencing same arn + a new one.
        c.trace_rows.push(ShippedTraceRow {
            trace_id: Uuid::new_v4(),
            canonical_row_body: b"row-2".to_vec(),
            hmac: vec![],
            kms_key_arns: vec!["arn:test:eu-west-1".into(), "arn:test:us-east-1".into()],
        });
        // Recompute head hash.
        let mut h = Sha256::new();
        for r in &c.trace_rows {
            h.update(&r.canonical_row_body);
        }
        c.head_hash = h.finalize().to_vec();
        let arns = c.distinct_kms_arns();
        assert_eq!(arns.len(), 2);
        assert!(arns.contains(&"arn:test:eu-west-1".to_string()));
        assert!(arns.contains(&"arn:test:us-east-1".to_string()));
    }

    #[tokio::test]
    async fn fetch_unknown_chain_returns_not_found() {
        let c = InMemoryChainShipmentClient::new();
        let err = c
            .fetch_and_stage(
                RegionCode::EuWest1,
                RegionCode::UsEast1,
                TenantId::new(),
                Uuid::new_v4(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ChainShipmentError::ChainNotFound { .. }));
    }

    #[tokio::test]
    async fn fetch_published_chain_stages_correctly() {
        let c = InMemoryChainShipmentClient::new();
        let tenant = TenantId::new();
        let chain = sample_chain(RegionCode::EuWest1, RegionCode::EuWest1, tenant);
        let swarm = chain.swarm_request_id;
        c.publish(chain);
        let handle = c
            .fetch_and_stage(RegionCode::EuWest1, RegionCode::EuWest1, tenant, swarm)
            .await
            .unwrap();
        assert_eq!(handle.swarm_request_id, swarm);
        assert_eq!(c.staged_count(), 1);
    }

    #[tokio::test]
    async fn verify_and_commit_returns_chain_and_drains_staging() {
        let c = InMemoryChainShipmentClient::new();
        let tenant = TenantId::new();
        let chain = sample_chain(RegionCode::EuWest1, RegionCode::EuWest1, tenant);
        let swarm = chain.swarm_request_id;
        c.publish(chain);
        let handle = c
            .fetch_and_stage(RegionCode::EuWest1, RegionCode::EuWest1, tenant, swarm)
            .await
            .unwrap();
        let committed = c.verify_and_commit(&handle).await.unwrap();
        assert_eq!(committed.swarm_request_id, swarm);
        assert_eq!(c.staged_count(), 0);
    }

    #[tokio::test]
    async fn commit_corrupted_chain_returns_head_hash_mismatch() {
        let c = InMemoryChainShipmentClient::new();
        let tenant = TenantId::new();
        let mut chain = sample_chain(RegionCode::EuWest1, RegionCode::EuWest1, tenant);
        // Corrupt the head hash.
        chain.head_hash = vec![0xFFu8; 32];
        let swarm = chain.swarm_request_id;
        c.publish(chain);
        let handle = c
            .fetch_and_stage(RegionCode::EuWest1, RegionCode::EuWest1, tenant, swarm)
            .await
            .unwrap();
        let err = c.verify_and_commit(&handle).await.unwrap_err();
        assert!(matches!(err, ChainShipmentError::HeadHashMismatch));
    }

    #[tokio::test]
    async fn abort_is_idempotent() {
        let c = InMemoryChainShipmentClient::new();
        let handle = StagedChainHandle {
            staging_id: Uuid::new_v4(),
            source_region: RegionCode::EuWest1,
            target_region: RegionCode::EuWest1,
            tenant_id: TenantId::new(),
            swarm_request_id: Uuid::new_v4(),
        };
        // Aborting a non-existent staging is OK.
        c.abort(&handle).await.unwrap();
        c.abort(&handle).await.unwrap();
    }

    #[tokio::test]
    async fn schema_too_new_rejected_at_stage() {
        let c = InMemoryChainShipmentClient::new();
        let tenant = TenantId::new();
        let mut chain = sample_chain(RegionCode::EuWest1, RegionCode::EuWest1, tenant);
        chain.sdk_schema_version = REPLAY_SCHEMA_VERSION + 1;
        let swarm = chain.swarm_request_id;
        c.publish(chain);
        let err = c
            .fetch_and_stage(RegionCode::EuWest1, RegionCode::EuWest1, tenant, swarm)
            .await
            .unwrap_err();
        assert!(matches!(err, ChainShipmentError::SchemaTooNew { .. }));
    }

    #[tokio::test]
    async fn commit_unknown_staging_returns_not_found() {
        let c = InMemoryChainShipmentClient::new();
        let handle = StagedChainHandle {
            staging_id: Uuid::new_v4(),
            source_region: RegionCode::EuWest1,
            target_region: RegionCode::EuWest1,
            tenant_id: TenantId::new(),
            swarm_request_id: Uuid::new_v4(),
        };
        let err = c.verify_and_commit(&handle).await.unwrap_err();
        assert!(matches!(err, ChainShipmentError::StagingNotFound { .. }));
    }

    #[tokio::test]
    async fn gc_stale_staging_removes_old_entries() {
        let c = InMemoryChainShipmentClient::with_staging_ttl(Duration::from_millis(50));
        let tenant = TenantId::new();
        let chain = sample_chain(RegionCode::EuWest1, RegionCode::EuWest1, tenant);
        let swarm = chain.swarm_request_id;
        c.publish(chain);
        let _handle = c
            .fetch_and_stage(RegionCode::EuWest1, RegionCode::EuWest1, tenant, swarm)
            .await
            .unwrap();
        assert_eq!(c.staged_count(), 1);
        tokio::time::sleep(Duration::from_millis(80)).await;
        let dropped = c.gc_stale_staging();
        assert_eq!(dropped, 1);
        assert_eq!(c.staged_count(), 0);
    }

    #[test]
    fn debug_format_does_not_panic() {
        let c = InMemoryChainShipmentClient::new();
        let _ = format!("{c:?}");
    }
}
