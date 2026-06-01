//! **Sprint 7 — N9** : Persistence trait for sealed epoch roots.
//!
//! # Why a separate store ?
//!
//! The Sprint 6 [`crate::audit::chain_store::ChainStore`] is optimised
//! for **append-per-trace** workloads (millions of rows, indexed by
//! `tenant_id` + `inserted_at`).  The Merkle epoch sealer writes far
//! less frequently — one row per epoch (~ one row per ≥ 1 024 traces),
//! but it needs separate queries :
//!
//! - "Give me the latest sealed epoch_id for tenant T" (so the next
//!   sealing knows where to start).
//! - "Give me the epoch that contains trace_id X" (so the auditor can
//!   build the inclusion proof for X).
//! - "Give me the linear sequence of epoch roots from epoch_id A to B"
//!   (for cross-epoch chain verification — each root links to the
//!   previous via `previous_epoch_root`).
//!
//! Co-mingling these access patterns in a single table would either
//! bloat the per-trace queries or slow the epoch queries.  Splitting
//! into a dedicated [`EpochRootStore`] is the audit-grade discipline.
//!
//! # Phase 4 wiring
//!
//! Production deploys [`PostgresEpochRootStore`] backed by table
//! `epoch_roots` (migration v008).  Sprint 7 ships the in-memory impl
//! [`InMemoryEpochRootStore`] which is sufficient for the smoke tests
//! and for the seal harness reference impl.

use async_trait::async_trait;
use dashmap::DashMap;
use efe_nano_sdk::TenantId;
use std::sync::Arc;

use crate::audit::merkle::epoch::EpochRoot;
use crate::error::ChainStoreError;

// ─────────────────────────────────────────────────────────────────────
//  Trait
// ─────────────────────────────────────────────────────────────────────

/// Persistent storage for sealed [`EpochRoot`]s.
///
/// All methods are tenant-scoped (per-tenant chain of epoch roots).
/// Cross-tenant queries are not exposed — sealing is per-tenant by design.
#[async_trait]
pub trait EpochRootStore: Send + Sync + 'static {
    /// Append a new sealed epoch root.
    ///
    /// Implementations MUST enforce monotonic `epoch_id` per-tenant
    /// (the `i`-th epoch for tenant T must have `epoch_id == i + 1`,
    /// 1-indexed).  Out-of-order appends ⇒
    /// [`ChainStoreError::Constraint`].
    async fn append(&self, root: EpochRoot) -> Result<(), ChainStoreError>;

    /// Latest sealed epoch for `tenant`.  `None` if the tenant has no
    /// sealed epochs yet.
    async fn latest_for_tenant(&self, tenant: TenantId)
        -> Result<Option<EpochRoot>, ChainStoreError>;

    /// Fetch a specific epoch by `epoch_id` within `tenant`.
    async fn get_by_epoch_id(
        &self,
        tenant: TenantId,
        epoch_id: u64,
    ) -> Result<Option<EpochRoot>, ChainStoreError>;

    /// Return every sealed epoch for `tenant`, ordered by ascending
    /// `epoch_id`.  Used by the linear-chain verifier (each root
    /// commits to the previous one).
    async fn chain_for_tenant(
        &self,
        tenant: TenantId,
    ) -> Result<Vec<EpochRoot>, ChainStoreError>;
}

// ─────────────────────────────────────────────────────────────────────
//  InMemoryEpochRootStore
// ─────────────────────────────────────────────────────────────────────

/// Sprint 7 default impl : DashMap-backed in-memory store.
///
/// Suitable for unit tests, integration tests, and single-process
/// Phase-3 deployments.  Production deploys Postgres via Phase 4.
#[derive(Debug, Default, Clone)]
pub struct InMemoryEpochRootStore {
    /// `tenant_id → ordered vec of epoch roots (by epoch_id)`.
    inner: Arc<DashMap<TenantId, Vec<EpochRoot>>>,
}

impl InMemoryEpochRootStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
        }
    }

    /// Total count of sealed epochs across all tenants — used by the
    /// observability metrics.
    pub fn total_sealed_count(&self) -> usize {
        self.inner.iter().map(|kv| kv.value().len()).sum()
    }

    /// **Test-only** backdoor : append an epoch row WITHOUT the
    /// monotonic-id + linkback validation that `append` enforces.
    ///
    /// Used by integration tests that need to simulate a compromised
    /// epoch store (e.g. "the operator was bribed and rewrote epoch 2's
    /// previous_epoch_root").  Production code MUST NOT call this.
    ///
    /// Gated to `cfg(test)` + the `test-utils` feature so it cannot
    /// be invoked from a release binary.
    #[cfg(test)]
    pub fn force_append_for_test(&self, root: EpochRoot) {
        self.inner
            .entry(root.tenant_id)
            .or_default()
            .push(root);
    }
}

#[async_trait]
impl EpochRootStore for InMemoryEpochRootStore {
    async fn append(&self, root: EpochRoot) -> Result<(), ChainStoreError> {
        let mut entry = self.inner.entry(root.tenant_id).or_default();
        let expected_id = entry.value().len() as u64 + 1;
        if root.epoch_id != expected_id {
            return Err(ChainStoreError::Constraint(format!(
                "non-monotonic epoch_id : got {}, expected {} for tenant {}",
                root.epoch_id, expected_id, root.tenant_id
            )));
        }
        // Sprint 7 — N9 : verify that the linkback to the previous
        // epoch root matches what's actually stored.  This catches a
        // malicious sealer that tries to "branch" the epoch chain.
        let prev_expected: Option<[u8; 32]> = entry.value().last().map(|r| r.root_hash);
        if root.previous_epoch_root != prev_expected {
            return Err(ChainStoreError::Constraint(format!(
                "previous_epoch_root linkback mismatch for tenant {} epoch {}",
                root.tenant_id, root.epoch_id
            )));
        }
        entry.value_mut().push(root);
        Ok(())
    }

    async fn latest_for_tenant(
        &self,
        tenant: TenantId,
    ) -> Result<Option<EpochRoot>, ChainStoreError> {
        Ok(self.inner.get(&tenant).and_then(|v| v.last().cloned()))
    }

    async fn get_by_epoch_id(
        &self,
        tenant: TenantId,
        epoch_id: u64,
    ) -> Result<Option<EpochRoot>, ChainStoreError> {
        if epoch_id == 0 {
            return Ok(None);
        }
        Ok(self
            .inner
            .get(&tenant)
            .and_then(|v| v.get((epoch_id - 1) as usize).cloned()))
    }

    async fn chain_for_tenant(
        &self,
        tenant: TenantId,
    ) -> Result<Vec<EpochRoot>, ChainStoreError> {
        Ok(self
            .inner
            .get(&tenant)
            .map(|v| v.clone())
            .unwrap_or_default())
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::merkle::epoch::EpochRoot;
    use chrono::Utc;
    use uuid::Uuid;

    fn epoch(tenant: TenantId, id: u64, prev: Option<[u8; 32]>, root: u8) -> EpochRoot {
        EpochRoot {
            tenant_id: tenant,
            epoch_id: id,
            start_trace_id: Uuid::new_v4(),
            end_trace_id: Uuid::new_v4(),
            leaf_count: 1024,
            root_hash: [root; 32],
            previous_epoch_root: prev,
            sealed_at: Utc::now(),
            signature: vec![1, 2, 3],
        }
    }

    #[tokio::test]
    async fn append_then_latest() {
        let store = InMemoryEpochRootStore::new();
        let t = TenantId::system();
        let r = epoch(t, 1, None, 0xaa);
        store.append(r.clone()).await.unwrap();
        assert_eq!(store.latest_for_tenant(t).await.unwrap().unwrap().epoch_id, 1);
    }

    #[tokio::test]
    async fn append_rejects_non_monotonic_epoch_id() {
        let store = InMemoryEpochRootStore::new();
        let t = TenantId::system();
        store.append(epoch(t, 1, None, 0xaa)).await.unwrap();
        // Skipping epoch 2 and going to 3 ⇒ rejected.
        let err = store
            .append(epoch(t, 3, Some([0xaa; 32]), 0xcc))
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("non-monotonic"));
    }

    #[tokio::test]
    async fn append_rejects_wrong_previous_linkback() {
        let store = InMemoryEpochRootStore::new();
        let t = TenantId::system();
        store.append(epoch(t, 1, None, 0xaa)).await.unwrap();
        // Epoch 2 must link back to [0xaa; 32], not [0xff; 32].
        let err = store
            .append(epoch(t, 2, Some([0xff; 32]), 0xbb))
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("linkback"));
    }

    #[tokio::test]
    async fn append_rejects_missing_previous_when_chain_exists() {
        let store = InMemoryEpochRootStore::new();
        let t = TenantId::system();
        store.append(epoch(t, 1, None, 0xaa)).await.unwrap();
        // Epoch 2 forgetting to set previous → rejected.
        let err = store.append(epoch(t, 2, None, 0xbb)).await.unwrap_err();
        assert!(err.to_string().contains("linkback"));
    }

    #[tokio::test]
    async fn append_rejects_previous_when_no_chain() {
        let store = InMemoryEpochRootStore::new();
        let t = TenantId::system();
        // First epoch must have previous == None.
        let err = store
            .append(epoch(t, 1, Some([0x42; 32]), 0xaa))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("linkback"));
    }

    #[tokio::test]
    async fn chain_for_tenant_returns_ordered() {
        let store = InMemoryEpochRootStore::new();
        let t = TenantId::system();
        store.append(epoch(t, 1, None, 0x11)).await.unwrap();
        store
            .append(epoch(t, 2, Some([0x11; 32]), 0x22))
            .await
            .unwrap();
        store
            .append(epoch(t, 3, Some([0x22; 32]), 0x33))
            .await
            .unwrap();
        let chain = store.chain_for_tenant(t).await.unwrap();
        assert_eq!(chain.len(), 3);
        assert_eq!(chain[0].epoch_id, 1);
        assert_eq!(chain[1].epoch_id, 2);
        assert_eq!(chain[2].epoch_id, 3);
    }

    #[tokio::test]
    async fn get_by_epoch_id_works() {
        let store = InMemoryEpochRootStore::new();
        let t = TenantId::system();
        store.append(epoch(t, 1, None, 0x11)).await.unwrap();
        store
            .append(epoch(t, 2, Some([0x11; 32]), 0x22))
            .await
            .unwrap();
        let got = store.get_by_epoch_id(t, 2).await.unwrap().unwrap();
        assert_eq!(got.root_hash, [0x22; 32]);
        assert!(store.get_by_epoch_id(t, 99).await.unwrap().is_none());
        // epoch_id == 0 is reserved for "no chain yet".
        assert!(store.get_by_epoch_id(t, 0).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn cross_tenant_isolation() {
        let store = InMemoryEpochRootStore::new();
        let t1 = TenantId::new();
        let t2 = TenantId::new();
        store.append(epoch(t1, 1, None, 0xa1)).await.unwrap();
        store.append(epoch(t2, 1, None, 0xb1)).await.unwrap();
        // Each tenant has its own epoch_id sequence.
        assert_eq!(
            store.latest_for_tenant(t1).await.unwrap().unwrap().root_hash,
            [0xa1; 32]
        );
        assert_eq!(
            store.latest_for_tenant(t2).await.unwrap().unwrap().root_hash,
            [0xb1; 32]
        );
    }

    #[tokio::test]
    async fn total_sealed_count() {
        let store = InMemoryEpochRootStore::new();
        let t1 = TenantId::new();
        let t2 = TenantId::new();
        store.append(epoch(t1, 1, None, 0x01)).await.unwrap();
        store
            .append(epoch(t1, 2, Some([0x01; 32]), 0x02))
            .await
            .unwrap();
        store.append(epoch(t2, 1, None, 0x03)).await.unwrap();
        assert_eq!(store.total_sealed_count(), 3);
    }
}
