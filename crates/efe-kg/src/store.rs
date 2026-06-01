// SPDX-License-Identifier: UNLICENSED
//! M33 — `KgStore` trait + in-memory impl.
//!
//! Abstract persistence layer for per-tenant knowledge graphs.
//! Concrete `efe-server` impl lives over Postgres (`kg_entity`,
//! `kg_relation`, `kg_claim` tables).

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;

use crate::entity::Entity;
use crate::error::{KgError, Result};
use crate::graph::Graph;
use crate::relation::Relation;

/// Per-tenant graph store.
#[async_trait]
pub trait KgStore: Send + Sync {
    /// Return the graph for `tenant_id` (create empty if missing).
    async fn get_or_create(&self, tenant_id: &str) -> Result<Arc<Graph>>;

    /// Drop the entire graph for `tenant_id` (used by the M29
    /// erasure cascade). Idempotent.
    async fn drop_tenant(&self, tenant_id: &str) -> Result<()>;

    /// Persist an entity to the PG mirror. Default no-op for
    /// dev / test impls.
    async fn persist_entity(&self, _tenant_id: &str, _e: &Entity) -> Result<()> {
        Ok(())
    }

    /// Persist a relation to the PG mirror.
    async fn persist_relation(&self, _tenant_id: &str, _r: &Relation) -> Result<()> {
        Ok(())
    }

    /// Delete an entity (and touching relations) from the PG
    /// mirror.
    async fn delete_entity(&self, _tenant_id: &str, _entity_id: &str) -> Result<()> {
        Ok(())
    }

    /// Delete a relation from the PG mirror.
    async fn delete_relation(&self, _tenant_id: &str, _relation_id: &str) -> Result<()> {
        Ok(())
    }
}

/// In-memory `KgStore` implementation.
#[derive(Default, Clone)]
pub struct InMemoryStore {
    inner: Arc<RwLock<HashMap<String, Arc<Graph>>>>,
}

impl InMemoryStore {
    /// Construct empty.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the number of tenants currently tracked.
    pub fn len(&self) -> usize {
        self.inner.read().len()
    }

    /// `true` if no tenant has a graph yet.
    pub fn is_empty(&self) -> bool {
        self.inner.read().is_empty()
    }
}

#[async_trait]
impl KgStore for InMemoryStore {
    async fn get_or_create(&self, tenant_id: &str) -> Result<Arc<Graph>> {
        if tenant_id.is_empty() {
            return Err(KgError::InvalidTenant("empty".into()));
        }
        {
            let g = self.inner.read();
            if let Some(graph) = g.get(tenant_id) {
                return Ok(graph.clone());
            }
        }
        let mut g = self.inner.write();
        let entry = g
            .entry(tenant_id.to_string())
            .or_insert_with(|| Arc::new(Graph::new()));
        Ok(entry.clone())
    }

    async fn drop_tenant(&self, tenant_id: &str) -> Result<()> {
        let mut g = self.inner.write();
        g.remove(tenant_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::EntityKind;

    #[tokio::test]
    async fn get_or_create_returns_empty_for_new_tenant() {
        let s = InMemoryStore::new();
        let g = s.get_or_create("acme").await.unwrap();
        assert!(g.is_empty());
        assert_eq!(s.len(), 1);
    }

    #[tokio::test]
    async fn get_or_create_returns_same_for_existing() {
        let s = InMemoryStore::new();
        let g1 = s.get_or_create("acme").await.unwrap();
        g1.upsert_entity(Entity::new("e1", EntityKind::Person, "alice"))
            .unwrap();
        let g2 = s.get_or_create("acme").await.unwrap();
        assert_eq!(g2.entity_count(), 1);
    }

    #[tokio::test]
    async fn drop_tenant_clears_graph() {
        let s = InMemoryStore::new();
        s.get_or_create("acme").await.unwrap();
        s.drop_tenant("acme").await.unwrap();
        assert!(s.is_empty());
        // Re-create yields a fresh empty graph.
        let g = s.get_or_create("acme").await.unwrap();
        assert!(g.is_empty());
    }

    #[tokio::test]
    async fn empty_tenant_id_is_rejected() {
        let s = InMemoryStore::new();
        let e = s.get_or_create("").await.unwrap_err();
        assert!(matches!(e, KgError::InvalidTenant(_)));
    }

    #[tokio::test]
    async fn drop_tenant_idempotent() {
        let s = InMemoryStore::new();
        s.drop_tenant("acme").await.unwrap();
        s.drop_tenant("acme").await.unwrap();
    }
}
