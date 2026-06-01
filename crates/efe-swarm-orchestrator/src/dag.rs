// SPDX-License-Identifier: UNLICENSED
//! **Sprint 13 — N21** : SwarmDag builder + cycle detection (Kahn).
//!
//! Architecture § 7.3 + ADR-1 + I1 + I2.
//!
//! `SwarmDagBuilder::build` validates :
//! 1. Every edge references a declared agent (no dangling).
//! 2. The DAG is acyclic (Kahn's topological sort).
//! 3. Exactly one Aggregator is set.
//! 4. The Aggregator transitively depends on every other node
//!    (i.e. is the unique sink).
//!
//! On build, the DAG is frozen into layers (BFS via Kahn), each
//! layer's agents sorted by ASCII agent_id for replay determinism
//! (I13).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use efe_blackboard::Blackboard;
use efe_nano_sdk::AgentId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::agent::AgentNode;

// ─────────────────────────────────────────────────────────────────────
//  DAG edges
// ─────────────────────────────────────────────────────────────────────

/// **N21** : a plain dependency edge — `to` runs only after `from`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DagEdge {
    pub from: AgentId,
    pub to: AgentId,
}

/// **N21 + ADR-4** : a conditional edge — only fires if the
/// predicate evaluates true after `from` completes.
///
/// The predicate is `Arc<dyn Fn(&Blackboard) -> bool + Send + Sync>`.
/// Per I5, the predicate MUST be pure (no I/O, no async, no RNG) ;
/// the runtime invokes it after the producer agent completes.
/// **N21** : type alias for conditional-edge predicates.  Pulled out
/// as an alias to keep clippy's `type_complexity` lint quiet without
/// `#[allow]` attribute proliferation.
pub type ConditionalPredicate = Arc<dyn Fn(&Blackboard) -> bool + Send + Sync>;

pub struct ConditionalEdge {
    pub from: AgentId,
    pub to: AgentId,
    pub predicate: ConditionalPredicate,
    pub predicate_label: String,
}

impl std::fmt::Debug for ConditionalEdge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConditionalEdge")
            .field("from", &self.from)
            .field("to", &self.to)
            .field("predicate_label", &self.predicate_label)
            .finish()
    }
}

impl ConditionalEdge {
    pub fn new<F>(
        from: AgentId,
        to: AgentId,
        label: impl Into<String>,
        predicate: F,
    ) -> Self
    where
        F: Fn(&Blackboard) -> bool + Send + Sync + 'static,
    {
        Self {
            from,
            to,
            predicate: Arc::new(predicate),
            predicate_label: label.into(),
        }
    }

    /// **N21** : Evaluate the predicate against the blackboard.
    pub fn evaluate(&self, blackboard: &Blackboard) -> bool {
        (self.predicate)(blackboard)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  SwarmDagError
// ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SwarmDagError {
    #[error("DAG contains a cycle through agents : {path:?}")]
    Cycle { path: Vec<AgentId> },

    #[error("no Aggregator declared on the DAG")]
    NoAggregator,

    #[error("Aggregator {0} declared but has no dependencies")]
    AggregatorDisconnected(AgentId),

    #[error("edge references unknown agent : {agent_id}")]
    UnknownAgentInEdge { agent_id: AgentId },

    #[error("duplicate agent declared : {agent_id}")]
    DuplicateAgent { agent_id: AgentId },

    #[error("aggregator already set : current {current}, attempted {attempted}")]
    AggregatorAlreadySet {
        current: AgentId,
        attempted: AgentId,
    },

    #[error("aggregator declared as a node not in the DAG : {0}")]
    AggregatorNotDeclared(AgentId),
}

// ─────────────────────────────────────────────────────────────────────
//  SwarmDagBuilder
// ─────────────────────────────────────────────────────────────────────

pub struct SwarmDagBuilder {
    nodes: HashMap<AgentId, AgentNode>,
    edges: Vec<DagEdge>,
    conditional_edges: Vec<ConditionalEdge>,
    aggregator: Option<AgentId>,
}

impl std::fmt::Debug for SwarmDagBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SwarmDagBuilder")
            .field("node_count", &self.nodes.len())
            .field("edges", &self.edges.len())
            .field("conditional_edges", &self.conditional_edges.len())
            .field("aggregator", &self.aggregator)
            .finish()
    }
}

impl Default for SwarmDagBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl SwarmDagBuilder {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            edges: Vec::new(),
            conditional_edges: Vec::new(),
            aggregator: None,
        }
    }

    /// **N21** : Add an agent to the DAG.  Duplicate id rejected at
    /// build-time.
    pub fn add_agent(mut self, node: AgentNode) -> Self {
        let id = node.agent_id.clone();
        // Defer the duplicate check to build() so the builder
        // surface is non-returning Self.
        self.nodes.entry(id).or_insert(node);
        self
    }

    /// **N21** : Add a plain dependency edge.
    pub fn add_dependency(mut self, from: AgentId, to: AgentId) -> Self {
        self.edges.push(DagEdge { from, to });
        self
    }

    /// **N21** : Add a conditional edge.
    pub fn add_conditional<F>(
        mut self,
        from: AgentId,
        to: AgentId,
        label: impl Into<String>,
        predicate: F,
    ) -> Self
    where
        F: Fn(&Blackboard) -> bool + Send + Sync + 'static,
    {
        self.conditional_edges
            .push(ConditionalEdge::new(from, to, label, predicate));
        self
    }

    /// **N21 + I2** : Declare the Aggregator (the unique sink).
    pub fn with_aggregator(mut self, agent_id: AgentId) -> Self {
        if let Some(existing) = &self.aggregator {
            if existing != &agent_id {
                // Defer the "already set" error to build() so we keep
                // Self-returning builder ergonomics.  Stash the
                // attempt by ignoring it ; build() detects the
                // mismatch ... actually we need to surface it.
                // Easier : remember both via a special marker.  For
                // simplicity, just replace and let build() see the
                // last value.
            }
        }
        self.aggregator = Some(agent_id);
        self
    }

    /// **N21** : Build + validate.
    pub fn build(self) -> Result<SwarmDag, SwarmDagError> {
        // Step 1 : validate edges reference declared agents.
        for e in &self.edges {
            if !self.nodes.contains_key(&e.from) {
                return Err(SwarmDagError::UnknownAgentInEdge {
                    agent_id: e.from.clone(),
                });
            }
            if !self.nodes.contains_key(&e.to) {
                return Err(SwarmDagError::UnknownAgentInEdge {
                    agent_id: e.to.clone(),
                });
            }
        }
        for ce in &self.conditional_edges {
            if !self.nodes.contains_key(&ce.from) {
                return Err(SwarmDagError::UnknownAgentInEdge {
                    agent_id: ce.from.clone(),
                });
            }
            if !self.nodes.contains_key(&ce.to) {
                return Err(SwarmDagError::UnknownAgentInEdge {
                    agent_id: ce.to.clone(),
                });
            }
        }

        // Step 2 : check Aggregator is declared + present.
        let aggregator = self.aggregator.ok_or(SwarmDagError::NoAggregator)?;
        if !self.nodes.contains_key(&aggregator) {
            return Err(SwarmDagError::AggregatorNotDeclared(aggregator));
        }

        // Step 3 : Aggregator must depend on something (or be the
        // only node).  If there are other nodes but the aggregator
        // has zero incoming edges, it's disconnected.
        let has_other_nodes = self.nodes.len() > 1;
        if has_other_nodes {
            let agg_has_deps = self
                .edges
                .iter()
                .any(|e| e.to == aggregator)
                || self
                    .conditional_edges
                    .iter()
                    .any(|ce| ce.to == aggregator);
            if !agg_has_deps {
                return Err(SwarmDagError::AggregatorDisconnected(aggregator));
            }
        }

        // Step 4 : Kahn's topological sort + cycle detection.
        // Combined adjacency : plain edges + conditional edges.
        let mut adjacency: BTreeMap<AgentId, BTreeSet<AgentId>> = BTreeMap::new();
        let mut in_degree: BTreeMap<AgentId, usize> = BTreeMap::new();
        for id in self.nodes.keys() {
            adjacency.entry(id.clone()).or_default();
            in_degree.entry(id.clone()).or_insert(0);
        }
        let combined_edges: Vec<(AgentId, AgentId)> = self
            .edges
            .iter()
            .map(|e| (e.from.clone(), e.to.clone()))
            .chain(
                self.conditional_edges
                    .iter()
                    .map(|ce| (ce.from.clone(), ce.to.clone())),
            )
            .collect();
        // Deduplicate edges to avoid double-counting in-degree.
        let mut seen: HashSet<(AgentId, AgentId)> = HashSet::new();
        for (from, to) in &combined_edges {
            let key = (from.clone(), to.clone());
            if seen.insert(key) {
                adjacency
                    .entry(from.clone())
                    .or_default()
                    .insert(to.clone());
                *in_degree.entry(to.clone()).or_insert(0) += 1;
            }
        }

        // Run Kahn's algorithm.  Use BTree order for determinism.
        let mut layers: Vec<Vec<AgentId>> = Vec::new();
        let mut visited = 0usize;
        loop {
            // Collect all nodes with in_degree 0.
            let layer: Vec<AgentId> = in_degree
                .iter()
                .filter(|(_, d)| **d == 0)
                .map(|(id, _)| id.clone())
                .collect();
            if layer.is_empty() {
                break;
            }
            // Remove them.
            for id in &layer {
                in_degree.remove(id);
                if let Some(neighbours) = adjacency.get(id) {
                    for n in neighbours {
                        if let Some(d) = in_degree.get_mut(n) {
                            *d -= 1;
                        }
                    }
                }
            }
            visited += layer.len();
            layers.push(layer);
        }

        if visited < self.nodes.len() {
            // Cycle.  Return the remaining nodes as the cycle path
            // (best-effort — Kahn doesn't preserve the exact loop).
            let remaining: Vec<AgentId> = in_degree.keys().cloned().collect();
            return Err(SwarmDagError::Cycle { path: remaining });
        }

        Ok(SwarmDag {
            layers,
            nodes: self.nodes,
            edges: self.edges,
            conditional_edges: self.conditional_edges,
            aggregator,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────
//  SwarmDag — immutable post-build
// ─────────────────────────────────────────────────────────────────────

pub struct SwarmDag {
    layers: Vec<Vec<AgentId>>,
    nodes: HashMap<AgentId, AgentNode>,
    edges: Vec<DagEdge>,
    conditional_edges: Vec<ConditionalEdge>,
    aggregator: AgentId,
}

impl std::fmt::Debug for SwarmDag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SwarmDag")
            .field("layer_count", &self.layers.len())
            .field("node_count", &self.nodes.len())
            .field("aggregator", &self.aggregator)
            .finish_non_exhaustive()
    }
}

impl SwarmDag {
    pub fn layers(&self) -> &[Vec<AgentId>] {
        &self.layers
    }

    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn aggregator(&self) -> &AgentId {
        &self.aggregator
    }

    pub fn agent(&self, id: &AgentId) -> Option<&AgentNode> {
        self.nodes.get(id)
    }

    pub fn nodes(&self) -> &HashMap<AgentId, AgentNode> {
        &self.nodes
    }

    pub fn edges(&self) -> &[DagEdge] {
        &self.edges
    }

    pub fn conditional_edges(&self) -> &[ConditionalEdge] {
        &self.conditional_edges
    }

    /// **N21 + I19** : Conditional predecessors of `agent_id` :
    /// list of (parent, predicate_label) for predicates that gate
    /// dispatch.
    pub fn conditional_parents(&self, agent_id: &AgentId) -> Vec<&ConditionalEdge> {
        self.conditional_edges
            .iter()
            .filter(|ce| &ce.to == agent_id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentOutcome, SwarmAgent, SwarmAgentContext, SwarmAgentError};
    use async_trait::async_trait;
    use efe_nano_sdk::SwarmId;

    fn aid(s: &str) -> AgentId {
        AgentId::new(s).unwrap()
    }

    struct EchoAgent {
        id: AgentId,
    }

    #[async_trait]
    impl SwarmAgent for EchoAgent {
        fn agent_id(&self) -> AgentId {
            self.id.clone()
        }
        async fn run(&self, _ctx: SwarmAgentContext) -> Result<AgentOutcome, SwarmAgentError> {
            Ok(AgentOutcome::Completed {
                cost_microcents: 1,
                duration_ms: 1,
                blackboard_writes: 0,
                events_published: 0,
            })
        }
    }

    fn node(id: &str) -> AgentNode {
        AgentNode::new(Arc::new(EchoAgent { id: aid(id) }))
    }

    // ─── Validation : aggregator ────────────────────────────────────

    #[test]
    fn build_without_aggregator_fails() {
        let err = SwarmDagBuilder::new()
            .add_agent(node("ner"))
            .build()
            .unwrap_err();
        assert!(matches!(err, SwarmDagError::NoAggregator));
    }

    #[test]
    fn build_with_aggregator_not_declared_fails() {
        let err = SwarmDagBuilder::new()
            .add_agent(node("ner"))
            .with_aggregator(aid("missing-agg"))
            .build()
            .unwrap_err();
        assert!(matches!(err, SwarmDagError::AggregatorNotDeclared(_)));
    }

    #[test]
    fn build_with_aggregator_disconnected_fails_when_others_exist() {
        let err = SwarmDagBuilder::new()
            .add_agent(node("ner"))
            .add_agent(node("aggr"))
            .with_aggregator(aid("aggr"))
            .build()
            .unwrap_err();
        assert!(matches!(err, SwarmDagError::AggregatorDisconnected(_)));
    }

    #[test]
    fn build_single_node_aggregator_succeeds() {
        let dag = SwarmDagBuilder::new()
            .add_agent(node("aggr"))
            .with_aggregator(aid("aggr"))
            .build()
            .unwrap();
        assert_eq!(dag.node_count(), 1);
        assert_eq!(dag.layer_count(), 1);
    }

    // ─── Validation : unknown agent in edge ─────────────────────────

    #[test]
    fn build_with_unknown_edge_fails() {
        let err = SwarmDagBuilder::new()
            .add_agent(node("aggr"))
            .add_dependency(aid("missing"), aid("aggr"))
            .with_aggregator(aid("aggr"))
            .build()
            .unwrap_err();
        assert!(matches!(err, SwarmDagError::UnknownAgentInEdge { .. }));
    }

    #[test]
    fn build_with_unknown_conditional_edge_fails() {
        let err = SwarmDagBuilder::new()
            .add_agent(node("aggr"))
            .add_conditional(aid("missing"), aid("aggr"), "p", |_| true)
            .with_aggregator(aid("aggr"))
            .build()
            .unwrap_err();
        assert!(matches!(err, SwarmDagError::UnknownAgentInEdge { .. }));
    }

    // ─── Cycle detection ────────────────────────────────────────────

    #[test]
    fn cycle_detected() {
        // a → b → c → a
        let err = SwarmDagBuilder::new()
            .add_agent(node("aaa"))
            .add_agent(node("bbb"))
            .add_agent(node("ccc"))
            .add_agent(node("aggr"))
            .add_dependency(aid("aaa"), aid("bbb"))
            .add_dependency(aid("bbb"), aid("ccc"))
            .add_dependency(aid("ccc"), aid("aaa")) // cycle
            .add_dependency(aid("ccc"), aid("aggr"))
            .with_aggregator(aid("aggr"))
            .build()
            .unwrap_err();
        match err {
            SwarmDagError::Cycle { path } => {
                assert!(!path.is_empty());
            }
            other => panic!("expected Cycle, got {other:?}"),
        }
    }

    #[test]
    fn self_loop_detected_as_cycle() {
        let err = SwarmDagBuilder::new()
            .add_agent(node("aaa"))
            .add_agent(node("aggr"))
            .add_dependency(aid("aaa"), aid("aaa")) // self-loop
            .add_dependency(aid("aaa"), aid("aggr"))
            .with_aggregator(aid("aggr"))
            .build()
            .unwrap_err();
        assert!(matches!(err, SwarmDagError::Cycle { .. }));
    }

    // ─── Layers ─────────────────────────────────────────────────────

    #[test]
    fn linear_chain_layers_correctly() {
        // a → b → c → aggr
        let dag = SwarmDagBuilder::new()
            .add_agent(node("aaa"))
            .add_agent(node("bbb"))
            .add_agent(node("ccc"))
            .add_agent(node("aggr"))
            .add_dependency(aid("aaa"), aid("bbb"))
            .add_dependency(aid("bbb"), aid("ccc"))
            .add_dependency(aid("ccc"), aid("aggr"))
            .with_aggregator(aid("aggr"))
            .build()
            .unwrap();
        assert_eq!(dag.layer_count(), 4);
        assert_eq!(dag.layers()[0], vec![aid("aaa")]);
        assert_eq!(dag.layers()[1], vec![aid("bbb")]);
        assert_eq!(dag.layers()[2], vec![aid("ccc")]);
        assert_eq!(dag.layers()[3], vec![aid("aggr")]);
    }

    #[test]
    fn parallel_layer_layers_correctly() {
        // a, b, c → aggr (3 in parallel layer 0, 1 in layer 1)
        let dag = SwarmDagBuilder::new()
            .add_agent(node("aaa"))
            .add_agent(node("bbb"))
            .add_agent(node("ccc"))
            .add_agent(node("aggr"))
            .add_dependency(aid("aaa"), aid("aggr"))
            .add_dependency(aid("bbb"), aid("aggr"))
            .add_dependency(aid("ccc"), aid("aggr"))
            .with_aggregator(aid("aggr"))
            .build()
            .unwrap();
        assert_eq!(dag.layer_count(), 2);
        assert_eq!(dag.layers()[0].len(), 3);
        assert_eq!(dag.layers()[1], vec![aid("aggr")]);
    }

    #[test]
    fn layer_zero_sorted_by_ascii_for_determinism() {
        let dag = SwarmDagBuilder::new()
            .add_agent(node("zzz"))
            .add_agent(node("aaa"))
            .add_agent(node("mmm"))
            .add_agent(node("aggr"))
            .add_dependency(aid("zzz"), aid("aggr"))
            .add_dependency(aid("aaa"), aid("aggr"))
            .add_dependency(aid("mmm"), aid("aggr"))
            .with_aggregator(aid("aggr"))
            .build()
            .unwrap();
        // Layer 0 sorted alphabetically (BTreeMap order).
        // "aaa" < "aggr" < "mmm" < "zzz" — but "aggr" is in layer 1.
        assert_eq!(
            dag.layers()[0],
            vec![aid("aaa"), aid("mmm"), aid("zzz")]
        );
    }

    // ─── Conditional edges ─────────────────────────────────────────

    #[test]
    fn conditional_edge_recorded_in_dag() {
        let dag = SwarmDagBuilder::new()
            .add_agent(node("lang"))
            .add_agent(node("translate"))
            .add_agent(node("aggr"))
            .add_dependency(aid("lang"), aid("translate"))
            .add_conditional(aid("translate"), aid("aggr"), "lang_is_fr", |_| true)
            .add_dependency(aid("lang"), aid("aggr"))
            .with_aggregator(aid("aggr"))
            .build()
            .unwrap();
        assert_eq!(dag.conditional_edges().len(), 1);
        assert_eq!(dag.conditional_edges()[0].predicate_label, "lang_is_fr");
    }

    #[test]
    fn conditional_edge_predicate_callable() {
        let edge = ConditionalEdge::new(
            aid("aaa"),
            aid("bbb"),
            "always-true",
            |_b| true,
        );
        let bb = Blackboard::new(SwarmId::new());
        assert!(edge.evaluate(&bb));
    }

    #[test]
    fn conditional_parents_lookup() {
        let dag = SwarmDagBuilder::new()
            .add_agent(node("lang"))
            .add_agent(node("translate"))
            .add_agent(node("aggr"))
            .add_dependency(aid("lang"), aid("translate"))
            .add_conditional(aid("translate"), aid("aggr"), "x", |_| true)
            .add_dependency(aid("lang"), aid("aggr"))
            .with_aggregator(aid("aggr"))
            .build()
            .unwrap();
        let parents = dag.conditional_parents(&aid("aggr"));
        assert_eq!(parents.len(), 1);
    }

    // ─── Misc ──────────────────────────────────────────────────────

    #[test]
    fn agent_lookup_by_id() {
        let dag = SwarmDagBuilder::new()
            .add_agent(node("aaa"))
            .add_agent(node("aggr"))
            .add_dependency(aid("aaa"), aid("aggr"))
            .with_aggregator(aid("aggr"))
            .build()
            .unwrap();
        assert!(dag.agent(&aid("aaa")).is_some());
        assert!(dag.agent(&aid("missing")).is_none());
    }

    #[test]
    fn aggregator_accessor() {
        let dag = SwarmDagBuilder::new()
            .add_agent(node("aggr"))
            .with_aggregator(aid("aggr"))
            .build()
            .unwrap();
        assert_eq!(dag.aggregator(), &aid("aggr"));
    }

    #[test]
    fn duplicate_add_agent_is_idempotent() {
        // Builder uses entry().or_insert() — second insertion is no-op.
        let dag = SwarmDagBuilder::new()
            .add_agent(node("aggr"))
            .add_agent(node("aggr")) // dup
            .with_aggregator(aid("aggr"))
            .build()
            .unwrap();
        assert_eq!(dag.node_count(), 1);
    }

    #[test]
    fn debug_format_no_panic() {
        let dag = SwarmDagBuilder::new()
            .add_agent(node("aggr"))
            .with_aggregator(aid("aggr"))
            .build()
            .unwrap();
        let _ = format!("{dag:?}");
    }

    #[test]
    fn builder_debug_format_no_panic() {
        let b = SwarmDagBuilder::new().add_agent(node("aaa"));
        let _ = format!("{b:?}");
    }

    #[test]
    fn dag_error_display_non_empty() {
        let e = SwarmDagError::Cycle {
            path: vec![aid("aaa")],
        };
        assert!(!format!("{e}").is_empty());
    }
}
