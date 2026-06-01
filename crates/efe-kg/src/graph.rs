// SPDX-License-Identifier: UNLICENSED
//! M33 — Adjacency-list graph.
//!
//! In-process undirected-but-typed graph backed by two HashMaps :
//!
//! - `entities` : id → `Entity`.
//! - `out_edges` : subject_id → Vec<(predicate, object_id, relation_id)>.
//! - `in_edges`  : object_id  → Vec<(predicate, subject_id, relation_id)>.
//!
//! Designed for ≤ 10 M nodes + 50 M edges per tenant. Above that
//! threshold tenants should switch to a graph database (Neo4j /
//! Memgraph) — out of scope for M33.

use std::collections::HashMap;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::entity::Entity;
use crate::error::{KgError, Result};
use crate::relation::{Predicate, Relation};

/// Adjacency entry : `(predicate, neighbor_entity_id, relation_id)`.
pub type AdjEntry = (Predicate, String, String);

/// One per-tenant graph.
#[derive(Default)]
pub struct Graph {
    inner: RwLock<GraphInner>,
}

impl std::fmt::Debug for Graph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Graph")
            .field("entities", &self.entity_count())
            .field("relations", &self.relation_count())
            .finish()
    }
}

#[derive(Default)]
struct GraphInner {
    entities: HashMap<String, Entity>,
    out_edges: HashMap<String, Vec<AdjEntry>>,
    in_edges: HashMap<String, Vec<AdjEntry>>,
    relations: HashMap<String, Relation>,
}

/// Snapshot envelope serialized to bincode + S3.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphSnapshot {
    /// Snapshot format version.
    pub version: u32,
    /// Entities in stable order.
    pub entities: Vec<Entity>,
    /// Relations in stable order.
    pub relations: Vec<Relation>,
}

/// Current snapshot format version.
pub const GRAPH_SNAPSHOT_VERSION: u32 = 1;

impl Graph {
    /// Construct a fresh empty graph.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of entities.
    #[must_use]
    pub fn entity_count(&self) -> usize {
        self.inner.read().entities.len()
    }

    /// Number of relations.
    #[must_use]
    pub fn relation_count(&self) -> usize {
        self.inner.read().relations.len()
    }

    /// `true` if no entities are stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.read().entities.is_empty()
    }

    /// Insert (or upsert) an entity. Returns the new entity count.
    pub fn upsert_entity(&self, e: Entity) -> Result<usize> {
        e.validate()?;
        let mut g = self.inner.write();
        g.entities.insert(e.id.clone(), e);
        Ok(g.entities.len())
    }

    /// Read an entity by id.
    pub fn get_entity(&self, id: &str) -> Result<Entity> {
        let g = self.inner.read();
        g.entities
            .get(id)
            .cloned()
            .ok_or_else(|| KgError::EntityNotFound(id.to_string()))
    }

    /// Delete an entity by id. Idempotent. Also drops every edge
    /// touching this entity.
    pub fn delete_entity(&self, id: &str) -> Result<()> {
        let mut g = self.inner.write();
        if g.entities.remove(id).is_some() {
            // Drop every edge touching this entity.
            let touching_rel_ids: Vec<String> = g
                .relations
                .values()
                .filter(|r| r.subject_id == id || r.object_id == id)
                .map(|r| r.id.clone())
                .collect();
            for rid in &touching_rel_ids {
                g.relations.remove(rid);
            }
            // Drop from adjacency maps.
            g.out_edges.remove(id);
            g.in_edges.remove(id);
            // Also remove tombstoned entries from other adj lists.
            for v in g.out_edges.values_mut() {
                v.retain(|(_, neighbor, _)| neighbor != id);
            }
            for v in g.in_edges.values_mut() {
                v.retain(|(_, neighbor, _)| neighbor != id);
            }
        }
        Ok(())
    }

    /// Insert (or upsert) a relation. Both endpoints MUST exist.
    pub fn upsert_relation(&self, r: Relation) -> Result<usize> {
        r.validate()?;
        let mut g = self.inner.write();
        if !g.entities.contains_key(&r.subject_id) {
            return Err(KgError::EntityNotFound(r.subject_id.clone()));
        }
        if !g.entities.contains_key(&r.object_id) {
            return Err(KgError::EntityNotFound(r.object_id.clone()));
        }
        // Replace any prior relation with the same id.
        if let Some(prev) = g.relations.remove(&r.id) {
            if let Some(v) = g.out_edges.get_mut(&prev.subject_id) {
                v.retain(|(_, _, rid)| rid != &prev.id);
            }
            if let Some(v) = g.in_edges.get_mut(&prev.object_id) {
                v.retain(|(_, _, rid)| rid != &prev.id);
            }
        }
        g.out_edges
            .entry(r.subject_id.clone())
            .or_default()
            .push((r.predicate.clone(), r.object_id.clone(), r.id.clone()));
        g.in_edges
            .entry(r.object_id.clone())
            .or_default()
            .push((r.predicate.clone(), r.subject_id.clone(), r.id.clone()));
        g.relations.insert(r.id.clone(), r);
        Ok(g.relations.len())
    }

    /// Read a relation by id.
    pub fn get_relation(&self, id: &str) -> Result<Relation> {
        let g = self.inner.read();
        g.relations
            .get(id)
            .cloned()
            .ok_or_else(|| KgError::RelationNotFound(id.to_string()))
    }

    /// Delete a relation by id. Idempotent.
    pub fn delete_relation(&self, id: &str) -> Result<()> {
        let mut g = self.inner.write();
        if let Some(r) = g.relations.remove(id) {
            if let Some(v) = g.out_edges.get_mut(&r.subject_id) {
                v.retain(|(_, _, rid)| rid != &r.id);
            }
            if let Some(v) = g.in_edges.get_mut(&r.object_id) {
                v.retain(|(_, _, rid)| rid != &r.id);
            }
        }
        Ok(())
    }

    /// Outgoing edges from a node. **Clones** the underlying Vec.
    /// Prefer [`Self::for_each_out_edge`] on the hot path (BFS /
    /// DFS / shortest-path) to avoid the clone.
    pub fn out_edges(&self, id: &str) -> Vec<AdjEntry> {
        let g = self.inner.read();
        g.out_edges.get(id).cloned().unwrap_or_default()
    }

    /// Incoming edges to a node. Same clone trade-off as
    /// [`Self::out_edges`].
    pub fn in_edges(&self, id: &str) -> Vec<AdjEntry> {
        let g = self.inner.read();
        g.in_edges.get(id).cloned().unwrap_or_default()
    }

    /// **M33 perf (G1)** : iterate outgoing edges under a single
    /// read-lock acquisition without cloning the Vec. The callback
    /// is invoked once per edge while the lock is held — it MUST
    /// NOT re-enter the graph (would deadlock the parking_lot
    /// `RwLock`).
    ///
    /// BFS / DFS / shortest-path use this path to save N clones on
    /// large traversals (N = visited nodes ; typical 10² – 10⁴).
    pub fn for_each_out_edge<F>(&self, id: &str, mut f: F)
    where
        F: FnMut(&AdjEntry),
    {
        let g = self.inner.read();
        if let Some(edges) = g.out_edges.get(id) {
            for e in edges {
                f(e);
            }
        }
    }

    /// Same as [`Self::for_each_out_edge`] for incoming edges.
    pub fn for_each_in_edge<F>(&self, id: &str, mut f: F)
    where
        F: FnMut(&AdjEntry),
    {
        let g = self.inner.read();
        if let Some(edges) = g.in_edges.get(id) {
            for e in edges {
                f(e);
            }
        }
    }

    /// **M33 perf (G1)** : O(1) out-degree lookup.
    pub fn out_degree(&self, id: &str) -> usize {
        self.inner
            .read()
            .out_edges
            .get(id)
            .map(|v| v.len())
            .unwrap_or(0)
    }

    /// **M33 perf (G1)** : O(1) in-degree lookup.
    pub fn in_degree(&self, id: &str) -> usize {
        self.inner
            .read()
            .in_edges
            .get(id)
            .map(|v| v.len())
            .unwrap_or(0)
    }

    /// All entities (for snapshot).
    pub fn entities(&self) -> Vec<Entity> {
        let g = self.inner.read();
        let mut out: Vec<Entity> = g.entities.values().cloned().collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// All relations (for snapshot).
    pub fn relations(&self) -> Vec<Relation> {
        let g = self.inner.read();
        let mut out: Vec<Relation> = g.relations.values().cloned().collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// **M33 perf (G4)** : bulk upsert of entities under a single
    /// write-lock acquisition. Amortizes the lock cost over N
    /// inserts. Returns the new entity count.
    pub fn bulk_upsert_entities(&self, entities: Vec<Entity>) -> Result<usize> {
        for e in &entities {
            e.validate()?;
        }
        let mut g = self.inner.write();
        // Pre-reserve to skip rehashes during bulk insert.
        g.entities.reserve(entities.len());
        for e in entities {
            g.entities.insert(e.id.clone(), e);
        }
        Ok(g.entities.len())
    }

    /// **M33 perf (G4)** : bulk upsert of relations under a single
    /// write-lock acquisition. Validates ahead of the lock so a
    /// malformed entry doesn't half-apply.
    pub fn bulk_upsert_relations(&self, relations: Vec<Relation>) -> Result<usize> {
        for r in &relations {
            r.validate()?;
        }
        let mut g = self.inner.write();
        g.relations.reserve(relations.len());
        // Pre-check endpoints exist (cheap with the lock held).
        for r in &relations {
            if !g.entities.contains_key(&r.subject_id) {
                return Err(KgError::EntityNotFound(r.subject_id.clone()));
            }
            if !g.entities.contains_key(&r.object_id) {
                return Err(KgError::EntityNotFound(r.object_id.clone()));
            }
        }
        for r in relations {
            // Replace any prior relation with the same id (same
            // bookkeeping as upsert_relation but inlined).
            if let Some(prev) = g.relations.remove(&r.id) {
                if let Some(v) = g.out_edges.get_mut(&prev.subject_id) {
                    v.retain(|(_, _, rid)| rid != &prev.id);
                }
                if let Some(v) = g.in_edges.get_mut(&prev.object_id) {
                    v.retain(|(_, _, rid)| rid != &prev.id);
                }
            }
            g.out_edges
                .entry(r.subject_id.clone())
                .or_default()
                .push((r.predicate.clone(), r.object_id.clone(), r.id.clone()));
            g.in_edges
                .entry(r.object_id.clone())
                .or_default()
                .push((r.predicate.clone(), r.subject_id.clone(), r.id.clone()));
            g.relations.insert(r.id.clone(), r);
        }
        Ok(g.relations.len())
    }

    /// Serialize a snapshot envelope.
    pub fn to_snapshot(&self) -> GraphSnapshot {
        GraphSnapshot {
            version: GRAPH_SNAPSHOT_VERSION,
            entities: self.entities(),
            relations: self.relations(),
        }
    }

    /// Restore from a snapshot envelope.
    pub fn from_snapshot(snap: &GraphSnapshot) -> Result<Self> {
        if snap.version != GRAPH_SNAPSHOT_VERSION {
            return Err(KgError::Internal {
                reason: format!(
                    "snapshot version {} unsupported (expected {GRAPH_SNAPSHOT_VERSION})",
                    snap.version
                ),
            });
        }
        let g = Self::new();
        for e in &snap.entities {
            g.upsert_entity(e.clone())?;
        }
        for r in &snap.relations {
            g.upsert_relation(r.clone())?;
        }
        Ok(g)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::EntityKind;

    fn ent(id: &str, kind: EntityKind, name: &str) -> Entity {
        Entity::new(id, kind, name)
    }

    fn rel(id: &str, subject: &str, predicate: &str, object: &str) -> Relation {
        Relation::new(
            id,
            subject,
            Predicate::new(predicate).unwrap(),
            object,
            0.95,
        )
    }

    #[test]
    fn upsert_entity_increments_count() {
        let g = Graph::new();
        assert!(g.is_empty());
        g.upsert_entity(ent("e1", EntityKind::Person, "alice")).unwrap();
        assert_eq!(g.entity_count(), 1);
        g.upsert_entity(ent("e2", EntityKind::Org, "acme")).unwrap();
        assert_eq!(g.entity_count(), 2);
    }

    #[test]
    fn upsert_relation_requires_both_endpoints() {
        let g = Graph::new();
        g.upsert_entity(ent("e1", EntityKind::Person, "alice")).unwrap();
        let e = g.upsert_relation(rel("r1", "e1", "works_at", "missing"))
            .unwrap_err();
        assert!(matches!(e, KgError::EntityNotFound(_)));
    }

    #[test]
    fn upsert_relation_creates_bidirectional_adjacency() {
        let g = Graph::new();
        g.upsert_entity(ent("e1", EntityKind::Person, "alice")).unwrap();
        g.upsert_entity(ent("e2", EntityKind::Org, "acme")).unwrap();
        g.upsert_relation(rel("r1", "e1", "works_at", "e2")).unwrap();
        let out = g.out_edges("e1");
        let inc = g.in_edges("e2");
        assert_eq!(out.len(), 1);
        assert_eq!(inc.len(), 1);
        assert_eq!(out[0].1, "e2");
        assert_eq!(inc[0].1, "e1");
    }

    #[test]
    fn delete_entity_drops_touching_relations() {
        let g = Graph::new();
        g.upsert_entity(ent("e1", EntityKind::Person, "alice")).unwrap();
        g.upsert_entity(ent("e2", EntityKind::Org, "acme")).unwrap();
        g.upsert_entity(ent("e3", EntityKind::Org, "ben")).unwrap();
        g.upsert_relation(rel("r1", "e1", "works_at", "e2")).unwrap();
        g.upsert_relation(rel("r2", "e1", "works_at", "e3")).unwrap();
        g.delete_entity("e1").unwrap();
        assert_eq!(g.relation_count(), 0);
        assert!(g.out_edges("e1").is_empty());
        assert!(g.in_edges("e2").is_empty());
    }

    #[test]
    fn delete_relation_removes_only_target_edge() {
        let g = Graph::new();
        g.upsert_entity(ent("e1", EntityKind::Person, "alice")).unwrap();
        g.upsert_entity(ent("e2", EntityKind::Org, "acme")).unwrap();
        g.upsert_relation(rel("r1", "e1", "works_at", "e2")).unwrap();
        g.upsert_relation(rel("r2", "e1", "founded", "e2")).unwrap();
        g.delete_relation("r1").unwrap();
        assert_eq!(g.relation_count(), 1);
        // r2 still around.
        g.get_relation("r2").unwrap();
        // r1 gone.
        assert!(g.get_relation("r1").is_err());
    }

    #[test]
    fn snapshot_roundtrip_preserves_state() {
        let g = Graph::new();
        g.upsert_entity(ent("e1", EntityKind::Person, "alice")).unwrap();
        g.upsert_entity(ent("e2", EntityKind::Org, "acme")).unwrap();
        g.upsert_relation(rel("r1", "e1", "works_at", "e2")).unwrap();
        let snap = g.to_snapshot();
        // Round-trip through JSON.
        let bytes = serde_json::to_vec(&snap).unwrap();
        let snap2: GraphSnapshot = serde_json::from_slice(&bytes).unwrap();
        let g2 = Graph::from_snapshot(&snap2).unwrap();
        assert_eq!(g2.entity_count(), 2);
        assert_eq!(g2.relation_count(), 1);
        let out = g2.out_edges("e1");
        assert_eq!(out[0].1, "e2");
    }

    #[test]
    fn from_snapshot_rejects_wrong_version() {
        let snap = GraphSnapshot {
            version: 999,
            entities: vec![],
            relations: vec![],
        };
        assert!(Graph::from_snapshot(&snap).is_err());
    }

    #[test]
    fn duplicate_relation_id_is_replaced() {
        let g = Graph::new();
        g.upsert_entity(ent("e1", EntityKind::Person, "alice")).unwrap();
        g.upsert_entity(ent("e2", EntityKind::Org, "acme")).unwrap();
        g.upsert_relation(rel("r1", "e1", "works_at", "e2")).unwrap();
        g.upsert_relation(rel("r1", "e1", "founded", "e2")).unwrap();
        assert_eq!(g.relation_count(), 1);
        let r = g.get_relation("r1").unwrap();
        assert_eq!(r.predicate.as_str(), "founded");
    }

    #[test]
    fn for_each_out_edge_matches_out_edges_vec() {
        // The callback variant must visit edges in the same order
        // as the cloning variant.
        let g = Graph::new();
        g.upsert_entity(ent("a", EntityKind::Person, "a")).unwrap();
        g.upsert_entity(ent("b", EntityKind::Org, "b")).unwrap();
        g.upsert_entity(ent("c", EntityKind::Org, "c")).unwrap();
        g.upsert_relation(rel("r1", "a", "works_at", "b")).unwrap();
        g.upsert_relation(rel("r2", "a", "founded", "c")).unwrap();
        let cloned = g.out_edges("a");
        let mut callback: Vec<AdjEntry> = Vec::new();
        g.for_each_out_edge("a", |e| callback.push(e.clone()));
        assert_eq!(cloned, callback);
    }

    #[test]
    fn out_degree_in_degree_match_lens() {
        let g = Graph::new();
        g.upsert_entity(ent("a", EntityKind::Person, "a")).unwrap();
        g.upsert_entity(ent("b", EntityKind::Org, "b")).unwrap();
        g.upsert_entity(ent("c", EntityKind::Org, "c")).unwrap();
        g.upsert_relation(rel("r1", "a", "works_at", "b")).unwrap();
        g.upsert_relation(rel("r2", "a", "founded", "c")).unwrap();
        assert_eq!(g.out_degree("a"), 2);
        assert_eq!(g.in_degree("b"), 1);
        assert_eq!(g.in_degree("c"), 1);
        assert_eq!(g.out_degree("missing"), 0);
    }

    #[test]
    fn bulk_upsert_entities_inserts_all() {
        let g = Graph::new();
        let n = g
            .bulk_upsert_entities(vec![
                ent("a", EntityKind::Person, "a"),
                ent("b", EntityKind::Org, "b"),
                ent("c", EntityKind::Org, "c"),
            ])
            .unwrap();
        assert_eq!(n, 3);
        assert_eq!(g.entity_count(), 3);
    }

    #[test]
    fn bulk_upsert_relations_fails_atomically_on_missing_endpoint() {
        let g = Graph::new();
        g.bulk_upsert_entities(vec![
            ent("a", EntityKind::Person, "a"),
            ent("b", EntityKind::Org, "b"),
        ])
        .unwrap();
        let result = g.bulk_upsert_relations(vec![
            rel("r1", "a", "works_at", "b"),
            rel("r2", "a", "works_at", "missing"), // bad endpoint
        ]);
        assert!(result.is_err());
        // r1 must NOT have been applied.
        assert_eq!(g.relation_count(), 0);
    }

    #[test]
    fn bulk_upsert_relations_applies_all_on_success() {
        let g = Graph::new();
        g.bulk_upsert_entities(vec![
            ent("a", EntityKind::Person, "a"),
            ent("b", EntityKind::Org, "b"),
            ent("c", EntityKind::Org, "c"),
        ])
        .unwrap();
        let n = g
            .bulk_upsert_relations(vec![
                rel("r1", "a", "works_at", "b"),
                rel("r2", "a", "founded", "c"),
            ])
            .unwrap();
        assert_eq!(n, 2);
        assert_eq!(g.relation_count(), 2);
    }
}
