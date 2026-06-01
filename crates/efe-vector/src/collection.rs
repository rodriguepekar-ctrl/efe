// SPDX-License-Identifier: UNLICENSED
//! M32 — Collection façade.
//!
//! A `Collection` ties together :
//!
//! - One [`Segment`] (the in-RAM HNSW + flat index).
//! - One [`VectorStore`] (the source-of-truth for doc bodies).
//! - Per-collection config : name, tenant, dim, metric, caps.
//!
//! The collection is the unit of CRUD ops at the REST layer. Its
//! contract :
//!
//! - Insert  → upsert in store + upsert in segment.
//! - Delete  → drop in store + tombstone in segment.
//! - Search  → segment.search ; the store is consulted only for
//!   payload + filter post-processing.
//! - Snapshot → segment.to_snapshot + collect doc bodies from
//!   store (HNSW backend) ; serialize.

use std::sync::Arc;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::distance::{validate_vector, Distance};
use crate::error::{Result, VectorError};
use crate::flat::SearchHit;
use crate::hnsw::HnswConfig;
use crate::segment::{Segment, SegmentSnapshot, SNAPSHOT_VERSION};
use crate::store::{InMemoryStore, VectorDoc, VectorStore};

/// Default max cardinality per collection (cf. ADR-M32-008).
pub const MAX_DOCS_PER_COLLECTION: u64 = 10_000_000;

/// Validate the collection slug against `[a-z0-9_-]{1,32}`.
pub fn validate_collection_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 32 {
        return Err(VectorError::InvalidCollectionName(name.into()));
    }
    if !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        return Err(VectorError::InvalidCollectionName(name.into()));
    }
    Ok(())
}

/// Per-collection configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionConfig {
    /// Tenant identifier.
    pub tenant_id: String,
    /// Collection slug.
    pub name: String,
    /// Embedding-model name (FK to the `embedding_model` table).
    pub embedding_model_name: String,
    /// Vector dimension.
    pub dim: usize,
    /// Distance metric.
    pub metric: Distance,
    /// HNSW tuning (only used when backend = hnsw).
    #[serde(default)]
    pub hnsw_cfg: HnswConfig,
    /// Per-collection cardinality cap.
    #[serde(default = "default_max_docs")]
    pub max_docs: u64,
    /// `true` ⇒ start with HNSW. `false` ⇒ start with flat,
    /// auto-promote at BRUTE_FORCE_MAX_VECTORS.
    #[serde(default)]
    pub force_hnsw: bool,
}

fn default_max_docs() -> u64 {
    MAX_DOCS_PER_COLLECTION
}

impl CollectionConfig {
    /// Validate the config end-to-end.
    pub fn validate(&self) -> Result<()> {
        validate_collection_name(&self.name)?;
        if self.dim == 0 || self.dim > crate::distance::MAX_DIM {
            return Err(VectorError::DimensionTooLarge {
                observed: self.dim,
                cap: crate::distance::MAX_DIM,
            });
        }
        if self.max_docs == 0 {
            return Err(VectorError::Internal {
                reason: "max_docs must be > 0".into(),
            });
        }
        Ok(())
    }
}

/// One collection : seg + store + config.
pub struct Collection {
    cfg: CollectionConfig,
    segment: RwLock<Segment>,
    store: Arc<dyn VectorStore>,
}

impl Collection {
    /// Construct a new collection.
    pub fn new(cfg: CollectionConfig, store: Arc<dyn VectorStore>) -> Result<Self> {
        cfg.validate()?;
        let segment = if cfg.force_hnsw {
            Segment::new_hnsw(cfg.dim, cfg.metric, cfg.hnsw_cfg)
        } else {
            Segment::new_flat(cfg.dim, cfg.metric)
        };
        Ok(Self {
            cfg,
            segment: RwLock::new(segment),
            store,
        })
    }

    /// Convenience constructor with [`InMemoryStore`].
    pub fn in_memory(cfg: CollectionConfig) -> Result<Self> {
        Self::new(cfg, Arc::new(InMemoryStore::new()))
    }

    /// Borrow the config.
    #[must_use]
    pub fn config(&self) -> &CollectionConfig {
        &self.cfg
    }

    /// Live doc cardinality (from the store, authoritative).
    pub async fn cardinality(&self) -> Result<u64> {
        self.store.cardinality().await
    }

    /// Insert / upsert. Enforces dim + cap.
    pub async fn upsert(&self, doc: VectorDoc) -> Result<()> {
        validate_vector(&doc.vector, self.cfg.dim)?;
        let current = self.store.cardinality().await?;
        // Only enforce cap when this is a fresh insert.
        let exists = self.store.get(&doc.id).await.is_ok();
        if !exists && current >= self.cfg.max_docs {
            return Err(VectorError::CollectionFull {
                observed: current,
                cap: self.cfg.max_docs,
            });
        }
        self.store.upsert(doc.clone()).await?;
        let mut seg = self.segment.write();
        seg.insert(doc)?;
        Ok(())
    }

    /// Delete by id. Idempotent.
    pub async fn delete(&self, id: &str) -> Result<()> {
        self.store.delete(id).await?;
        let mut seg = self.segment.write();
        seg.delete(id)?;
        Ok(())
    }

    /// Search top-k.
    pub fn search(&self, query: &[f32], k: usize) -> Result<Vec<SearchHit>> {
        validate_vector(query, self.cfg.dim)?;
        let seg = self.segment.read();
        seg.search(query, k)
    }

    /// Read a document body by id (for hybrid retrieval / payload
    /// hydration).
    pub async fn get(&self, id: &str) -> Result<VectorDoc> {
        self.store.get(id).await
    }

    /// Serialize a snapshot. The doc bodies are always pulled from
    /// the store (authoritative) so the snapshot is correct even
    /// when the segment is HNSW (which doesn't track docs locally).
    ///
    /// **M32 audit fix** : the segment read-lock is dropped BEFORE
    /// the `iter_all()` await, otherwise the parking_lot guard
    /// would straddle a tokio yield point. Clippy
    /// `await_holding_lock` flagged this — and beyond the lint,
    /// holding the lock through an async call risks deadlock if
    /// the underlying store re-enters the segment (e.g. M32.1 will
    /// add a PG-backed store that may issue its own queries).
    pub async fn snapshot(&self) -> Result<SegmentSnapshot> {
        let mut snap = {
            // Scoped block : guard dropped at the closing brace.
            let seg = self.segment.read();
            seg.to_snapshot()
        };
        // Override the docs from the authoritative store. The
        // segment lock is no longer held while we await.
        snap.docs = self.store.iter_all().await?;
        snap.version = SNAPSHOT_VERSION;
        Ok(snap)
    }

    /// Replay a snapshot. **Wipes** the current state.
    pub async fn restore(
        cfg: CollectionConfig,
        store: Arc<dyn VectorStore>,
        snap: &SegmentSnapshot,
    ) -> Result<Self> {
        let coll = Self::new(cfg, store)?;
        for d in &snap.docs {
            coll.upsert(d.clone()).await?;
        }
        Ok(coll)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(name: &str, dim: usize) -> CollectionConfig {
        CollectionConfig {
            tenant_id: "acme".into(),
            name: name.into(),
            embedding_model_name: "efe-classifier-768".into(),
            dim,
            metric: Distance::Cosine,
            hnsw_cfg: HnswConfig::default(),
            max_docs: 100,
            force_hnsw: false,
        }
    }

    #[test]
    fn collection_name_validation() {
        validate_collection_name("acme-tickets").unwrap();
        validate_collection_name("ABC").unwrap_err();
        validate_collection_name("").unwrap_err();
        validate_collection_name("with space").unwrap_err();
        validate_collection_name(&"x".repeat(33)).unwrap_err();
    }

    #[tokio::test]
    async fn upsert_search_roundtrip() {
        let c = Collection::in_memory(cfg("tickets", 3)).unwrap();
        c.upsert(VectorDoc::new("a", vec![1.0, 0.0, 0.0])).await.unwrap();
        c.upsert(VectorDoc::new("b", vec![0.0, 1.0, 0.0])).await.unwrap();
        let out = c.search(&[1.0, 0.0, 0.0], 1).unwrap();
        assert_eq!(out[0].id, "a");
    }

    #[tokio::test]
    async fn upsert_rejects_dim_mismatch() {
        let c = Collection::in_memory(cfg("tickets", 3)).unwrap();
        let e = c
            .upsert(VectorDoc::new("a", vec![1.0, 2.0]))
            .await
            .unwrap_err();
        assert!(matches!(e, VectorError::DimensionMismatch { .. }));
    }

    #[tokio::test]
    async fn upsert_enforces_max_docs_cap() {
        let mut config = cfg("tickets", 3);
        config.max_docs = 2;
        let c = Collection::in_memory(config).unwrap();
        c.upsert(VectorDoc::new("a", vec![1.0, 0.0, 0.0])).await.unwrap();
        c.upsert(VectorDoc::new("b", vec![0.0, 1.0, 0.0])).await.unwrap();
        let e = c
            .upsert(VectorDoc::new("c", vec![0.0, 0.0, 1.0]))
            .await
            .unwrap_err();
        assert!(matches!(e, VectorError::CollectionFull { .. }));
    }

    #[tokio::test]
    async fn upsert_does_not_count_against_cap_on_replace() {
        let mut config = cfg("tickets", 3);
        config.max_docs = 2;
        let c = Collection::in_memory(config).unwrap();
        c.upsert(VectorDoc::new("a", vec![1.0, 0.0, 0.0])).await.unwrap();
        c.upsert(VectorDoc::new("b", vec![0.0, 1.0, 0.0])).await.unwrap();
        // Re-upsert of existing id is OK even at cap.
        c.upsert(VectorDoc::new("a", vec![0.5, 0.5, 0.0])).await.unwrap();
    }

    #[tokio::test]
    async fn delete_is_idempotent() {
        let c = Collection::in_memory(cfg("tickets", 3)).unwrap();
        c.delete("never_existed").await.unwrap();
        c.upsert(VectorDoc::new("a", vec![1.0, 0.0, 0.0])).await.unwrap();
        c.delete("a").await.unwrap();
        c.delete("a").await.unwrap();
        assert_eq!(c.cardinality().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn snapshot_includes_all_docs() {
        let c = Collection::in_memory(cfg("tickets", 3)).unwrap();
        c.upsert(VectorDoc::new("a", vec![1.0, 0.0, 0.0])).await.unwrap();
        c.upsert(VectorDoc::new("b", vec![0.0, 1.0, 0.0])).await.unwrap();
        let snap = c.snapshot().await.unwrap();
        assert_eq!(snap.docs.len(), 2);
        assert_eq!(snap.dim, 3);
    }

    #[tokio::test]
    async fn restore_rebuilds_searchable_collection() {
        let c = Collection::in_memory(cfg("tickets", 3)).unwrap();
        c.upsert(VectorDoc::new("a", vec![1.0, 0.0, 0.0])).await.unwrap();
        c.upsert(VectorDoc::new("b", vec![0.0, 1.0, 0.0])).await.unwrap();
        let snap = c.snapshot().await.unwrap();
        let store2 = Arc::new(InMemoryStore::new());
        let c2 = Collection::restore(c.config().clone(), store2, &snap).await.unwrap();
        let out = c2.search(&[1.0, 0.0, 0.0], 1).unwrap();
        assert_eq!(out[0].id, "a");
    }

    #[tokio::test]
    async fn force_hnsw_uses_hnsw_backend_from_start() {
        let mut config = cfg("tickets", 3);
        config.force_hnsw = true;
        let c = Collection::new(config, Arc::new(InMemoryStore::new())).unwrap();
        c.upsert(VectorDoc::new("a", vec![1.0, 0.0, 0.0])).await.unwrap();
        let snap = c.snapshot().await.unwrap();
        assert_eq!(snap.backend, "hnsw");
    }
}
