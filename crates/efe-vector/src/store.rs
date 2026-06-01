// SPDX-License-Identifier: UNLICENSED
//! M32 — Vector storage abstraction.
//!
//! The `VectorStore` trait abstracts the **per-document** storage
//! layer. The HNSW + flat indices live on top of a `VectorStore`
//! and treat it as the source-of-truth for `(vector, payload)`
//! pairs.
//!
//! In production the concrete impl lives in `efe-server` (Postgres-
//! backed mirror table `vector_doc`). For tests + dev this crate
//! ships [`InMemoryStore`].

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::error::{Result, VectorError};

/// Document identifier — ULID-shaped (26 chars). The caller assigns
/// the id (clients want predictable upsert semantics) ; the crate
/// validates length + character set.
pub type DocId = String;

/// One document stored in a collection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorDoc {
    /// Document identifier.
    pub id: DocId,
    /// Vector embedding (length must match the collection's dim).
    pub vector: Vec<f32>,
    /// Optional original text (for hybrid BM25 retrieval).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Arbitrary JSON payload (used for filters).
    #[serde(default)]
    pub payload: serde_json::Value,
}

impl VectorDoc {
    /// Construct from the minimal set of fields.
    pub fn new(id: impl Into<DocId>, vector: Vec<f32>) -> Self {
        Self {
            id: id.into(),
            vector,
            text: None,
            payload: serde_json::Value::Null,
        }
    }

    /// Builder-style text setter.
    #[must_use]
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    /// Builder-style payload setter.
    #[must_use]
    pub fn with_payload(mut self, payload: serde_json::Value) -> Self {
        self.payload = payload;
        self
    }
}

/// Abstract vector storage.
#[async_trait]
pub trait VectorStore: Send + Sync {
    /// Upsert one document. Existing rows with the same `id` are
    /// replaced.
    async fn upsert(&self, doc: VectorDoc) -> Result<()>;

    /// Read one document by id.
    async fn get(&self, id: &str) -> Result<VectorDoc>;

    /// Delete one document. Returns `Ok(())` even if the doc was
    /// already absent (idempotent).
    async fn delete(&self, id: &str) -> Result<()>;

    /// Number of documents currently stored.
    async fn cardinality(&self) -> Result<u64>;

    /// Iterate every document (for HNSW rebuilds + snapshots).
    async fn iter_all(&self) -> Result<Vec<VectorDoc>>;
}

// ─────────────────────────── IN-MEMORY ───────────────────────────

/// Type alias for the in-memory document map.
pub type DocMap = HashMap<DocId, VectorDoc>;

/// Thread-safe in-memory `VectorStore` for tests + dev.
#[derive(Default, Clone)]
pub struct InMemoryStore {
    inner: Arc<RwLock<DocMap>>,
}

impl InMemoryStore {
    /// Construct a fresh empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot the entire state (for test assertions).
    pub fn snapshot(&self) -> DocMap {
        self.inner.read().clone()
    }
}

#[async_trait]
impl VectorStore for InMemoryStore {
    async fn upsert(&self, doc: VectorDoc) -> Result<()> {
        if doc.id.is_empty() {
            return Err(VectorError::InvalidVector {
                reason: "doc id is empty".into(),
            });
        }
        let mut g = self.inner.write();
        g.insert(doc.id.clone(), doc);
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<VectorDoc> {
        let g = self.inner.read();
        g.get(id)
            .cloned()
            .ok_or_else(|| VectorError::DocumentNotFound(id.to_string()))
    }

    async fn delete(&self, id: &str) -> Result<()> {
        let mut g = self.inner.write();
        g.remove(id);
        Ok(())
    }

    async fn cardinality(&self) -> Result<u64> {
        Ok(self.inner.read().len() as u64)
    }

    async fn iter_all(&self) -> Result<Vec<VectorDoc>> {
        Ok(self.inner.read().values().cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(id: &str, v: Vec<f32>) -> VectorDoc {
        VectorDoc::new(id, v)
    }

    #[tokio::test]
    async fn upsert_get_roundtrip() {
        let s = InMemoryStore::new();
        s.upsert(doc("d1", vec![1.0, 2.0, 3.0])).await.unwrap();
        let got = s.get("d1").await.unwrap();
        assert_eq!(got.vector, vec![1.0, 2.0, 3.0]);
    }

    #[tokio::test]
    async fn upsert_replaces_existing() {
        let s = InMemoryStore::new();
        s.upsert(doc("d1", vec![1.0; 3])).await.unwrap();
        s.upsert(doc("d1", vec![9.0; 3])).await.unwrap();
        let got = s.get("d1").await.unwrap();
        assert_eq!(got.vector, vec![9.0; 3]);
    }

    #[tokio::test]
    async fn upsert_rejects_empty_id() {
        let s = InMemoryStore::new();
        let e = s.upsert(doc("", vec![1.0; 3])).await.unwrap_err();
        assert!(matches!(e, VectorError::InvalidVector { .. }));
    }

    #[tokio::test]
    async fn get_missing_returns_not_found() {
        let s = InMemoryStore::new();
        let e = s.get("missing").await.unwrap_err();
        assert!(matches!(e, VectorError::DocumentNotFound(_)));
        assert_eq!(e.error_code(), "vector.doc_not_found");
    }

    #[tokio::test]
    async fn delete_is_idempotent() {
        let s = InMemoryStore::new();
        s.delete("never_existed").await.unwrap();
        s.upsert(doc("d1", vec![1.0; 3])).await.unwrap();
        s.delete("d1").await.unwrap();
        s.delete("d1").await.unwrap(); // double-delete OK
        assert_eq!(s.cardinality().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn iter_all_returns_every_doc() {
        let s = InMemoryStore::new();
        s.upsert(doc("d1", vec![1.0; 3])).await.unwrap();
        s.upsert(doc("d2", vec![2.0; 3])).await.unwrap();
        s.upsert(doc("d3", vec![3.0; 3])).await.unwrap();
        let mut all = s.iter_all().await.unwrap();
        all.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].id, "d1");
        assert_eq!(all[2].id, "d3");
    }

    #[tokio::test]
    async fn cardinality_tracks_size() {
        let s = InMemoryStore::new();
        assert_eq!(s.cardinality().await.unwrap(), 0);
        s.upsert(doc("d1", vec![1.0; 3])).await.unwrap();
        assert_eq!(s.cardinality().await.unwrap(), 1);
        s.upsert(doc("d2", vec![1.0; 3])).await.unwrap();
        assert_eq!(s.cardinality().await.unwrap(), 2);
    }

    #[test]
    fn builders_set_optional_fields() {
        let d = VectorDoc::new("x", vec![1.0])
            .with_text("hello")
            .with_payload(serde_json::json!({"foo": "bar"}));
        assert_eq!(d.text.as_deref(), Some("hello"));
        assert_eq!(d.payload, serde_json::json!({"foo": "bar"}));
    }
}
