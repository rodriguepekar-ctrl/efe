// SPDX-License-Identifier: UNLICENSED
//! M33 — Multi-hop GraphRAG executor.
//!
//! Wires together :
//!
//! - [`efe_vector::Collection`] for dense search.
//! - [`efe_kg::Graph`] for traversal.
//! - [`crate::fusion::fuse_scores`] for ranking.
//!
//! `MultiHopExecutor` takes a `QueryPlan` produced by the planner
//! and runs it end-to-end.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use efe_kg::{bfs, Graph, TraversalFilter};
use efe_vector::Collection;

use crate::error::{GraphRagError, Result};
use crate::fusion::{fuse_scores, GraphRagHit, DEFAULT_ALPHA, DEFAULT_DECAY};
use crate::planner::{PlanStep, QueryPlan};

/// Request envelope for [`MultiHopExecutor::execute`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphRagRequest {
    /// Query vector for the dense seed step.
    pub query_vector: Vec<f32>,
    /// Optional explicit graph-seed entity ids (added to the seeds
    /// derived from dense hits).
    #[serde(default)]
    pub explicit_seed_entity_ids: Vec<String>,
    /// Traversal filter (predicates, skip list).
    #[serde(default)]
    pub filter: TraversalFilter,
    /// Fusion alpha (defaults to `DEFAULT_ALPHA`).
    #[serde(default = "default_alpha")]
    pub alpha: f32,
    /// Decay rate (defaults to `DEFAULT_DECAY`).
    #[serde(default = "default_decay")]
    pub decay_rate: f32,
}

fn default_alpha() -> f32 {
    DEFAULT_ALPHA
}
fn default_decay() -> f32 {
    DEFAULT_DECAY
}

/// Multi-hop executor.
pub struct MultiHopExecutor {
    dense: Arc<Collection>,
    graph: Arc<Graph>,
}

impl MultiHopExecutor {
    /// Construct from a dense collection + a per-tenant graph.
    pub fn new(dense: Arc<Collection>, graph: Arc<Graph>) -> Self {
        Self { dense, graph }
    }

    /// Run the plan end-to-end.
    pub async fn execute(
        &self,
        plan: &QueryPlan,
        req: &GraphRagRequest,
    ) -> Result<Vec<GraphRagHit>> {
        plan.validate()?;
        if req.query_vector.is_empty() && req.explicit_seed_entity_ids.is_empty() {
            return Err(GraphRagError::EmptyQuery);
        }

        let mut dense_pairs: Vec<(String, f32)> = Vec::new();
        let mut graph_pairs: Vec<(String, usize)> = Vec::new();
        let mut final_top_k: usize = 10;

        for step in &plan.steps {
            match step {
                PlanStep::DenseSeed { top_k } => {
                    if !req.query_vector.is_empty() {
                        let hits = self.dense.search(&req.query_vector, *top_k)?;
                        dense_pairs = hits.into_iter().map(|h| (h.id, h.score)).collect();
                    }
                }
                PlanStep::GraphExpand {
                    max_hops,
                    max_results,
                } => {
                    // Build seed set : dense hit ids ∪ explicit seeds.
                    let mut seeds: Vec<String> = dense_pairs.iter().map(|(id, _)| id.clone()).collect();
                    seeds.extend(req.explicit_seed_entity_ids.iter().cloned());
                    seeds.sort();
                    seeds.dedup();

                    // Run BFS from each seed (ignoring missing
                    // entities — the dense hit may refer to a doc,
                    // not an entity).
                    let mut all_hops: Vec<(String, usize)> = Vec::new();
                    for s in &seeds {
                        match bfs(self.graph.as_ref(), s, *max_hops, &req.filter, *max_results) {
                            Ok(nodes) => {
                                for n in nodes {
                                    all_hops.push((n.entity_id, n.hop_distance));
                                }
                            }
                            Err(efe_kg::KgError::EntityNotFound(_)) => continue,
                            Err(e) => return Err(e.into()),
                        }
                    }
                    graph_pairs = all_hops;
                }
                PlanStep::Rank { top_k } => {
                    final_top_k = *top_k;
                }
            }
        }

        let hits = fuse_scores(
            &dense_pairs,
            &graph_pairs,
            req.alpha,
            req.decay_rate,
            final_top_k,
        )?;
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_kg::{Entity, EntityKind, Predicate, Relation};
    use efe_vector::{CollectionConfig, Distance, HnswConfig, InMemoryStore, VectorDoc};

    fn make_collection() -> Arc<Collection> {
        let cfg = CollectionConfig {
            tenant_id: "acme".into(),
            name: "tickets".into(),
            embedding_model_name: "efe-classifier-768".into(),
            dim: 3,
            metric: Distance::Cosine,
            hnsw_cfg: HnswConfig::default(),
            max_docs: 100,
            force_hnsw: false,
        };
        Arc::new(Collection::new(cfg, Arc::new(InMemoryStore::new())).unwrap())
    }

    fn make_graph() -> Arc<Graph> {
        let g = Graph::new();
        g.upsert_entity(Entity::new("a", EntityKind::Person, "alice")).unwrap();
        g.upsert_entity(Entity::new("b", EntityKind::Org, "acme")).unwrap();
        g.upsert_entity(Entity::new("c", EntityKind::Org, "beta")).unwrap();
        g.upsert_relation(Relation::new(
            "r1",
            "a",
            Predicate::new("works_at").unwrap(),
            "b",
            0.95,
        ))
        .unwrap();
        g.upsert_relation(Relation::new(
            "r2",
            "b",
            Predicate::new("subsidiary_of").unwrap(),
            "c",
            0.95,
        ))
        .unwrap();
        Arc::new(g)
    }

    #[tokio::test]
    async fn execute_dense_only_when_no_seeds_match_graph() {
        let dense = make_collection();
        dense
            .upsert(VectorDoc::new("doc-x", vec![1.0, 0.0, 0.0]))
            .await
            .unwrap();
        let graph = make_graph();
        let exec = MultiHopExecutor::new(dense, graph);
        let plan = crate::planner::plan_query(5, 2).unwrap();
        let req = GraphRagRequest {
            query_vector: vec![1.0, 0.0, 0.0],
            explicit_seed_entity_ids: vec![],
            filter: TraversalFilter::default(),
            alpha: 0.7,
            decay_rate: 0.5,
        };
        let out = exec.execute(&plan, &req).await.unwrap();
        // Dense hit `doc-x` is not in the graph ⇒ no graph expansion ;
        // result is just the dense hit.
        assert_eq!(out[0].id, "doc-x");
        assert!(out[0].dense_origin);
        assert!(!out[0].graph_origin);
    }

    #[tokio::test]
    async fn execute_expands_graph_from_explicit_seed() {
        let dense = make_collection();
        let graph = make_graph();
        let exec = MultiHopExecutor::new(dense, graph);
        let plan = crate::planner::plan_query(10, 2).unwrap();
        let req = GraphRagRequest {
            query_vector: vec![1.0, 0.0, 0.0],
            explicit_seed_entity_ids: vec!["a".into()],
            filter: TraversalFilter::default(),
            alpha: 0.5,
            decay_rate: 0.5,
        };
        let out = exec.execute(&plan, &req).await.unwrap();
        let ids: std::collections::HashSet<_> = out.iter().map(|h| h.id.clone()).collect();
        // BFS from `a` reaches a, b, c.
        assert!(ids.contains("a"));
        assert!(ids.contains("b"));
        assert!(ids.contains("c"));
    }

    #[tokio::test]
    async fn execute_empty_query_rejected() {
        let dense = make_collection();
        let graph = make_graph();
        let exec = MultiHopExecutor::new(dense, graph);
        let plan = crate::planner::plan_query(5, 2).unwrap();
        let req = GraphRagRequest {
            query_vector: vec![],
            explicit_seed_entity_ids: vec![],
            filter: TraversalFilter::default(),
            alpha: 0.7,
            decay_rate: 0.5,
        };
        let e = exec.execute(&plan, &req).await.unwrap_err();
        assert!(matches!(e, GraphRagError::EmptyQuery));
    }

    #[tokio::test]
    async fn execute_seed_a_at_alpha_zero_ranks_a_first() {
        let dense = make_collection();
        let graph = make_graph();
        let exec = MultiHopExecutor::new(dense, graph);
        let plan = crate::planner::plan_query(5, 2).unwrap();
        let req = GraphRagRequest {
            query_vector: vec![1.0, 0.0, 0.0],
            explicit_seed_entity_ids: vec!["a".into()],
            filter: TraversalFilter::default(),
            alpha: 0.0,
            decay_rate: 0.5,
        };
        let out = exec.execute(&plan, &req).await.unwrap();
        // alpha = 0 ⇒ graph dominates ; `a` is hop 0 ⇒ highest.
        assert_eq!(out[0].id, "a");
    }
}
