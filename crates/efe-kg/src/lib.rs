// SPDX-License-Identifier: UNLICENSED
//! # `efe-kg` — M33 knowledge-graph plane.
//!
//! Pure-Rust crate providing :
//!
//! - Entity model with 6 NER labels.
//! - Relation model + base + custom predicate catalog.
//! - Claim → relation promotion logic.
//! - Adjacency-list graph + BFS / DFS / shortest path.
//! - Entity resolution (Levenshtein + cosine).
//! - NER + Relation extractor traits + in-memory test impls.
//! - Per-tenant `KgStore` trait + in-memory impl.
//!
//! ## Architectural invariants
//!
//! 1. **No global state.** Each tenant has an independent graph.
//! 2. **Validated on every insert.** Names, predicates, and
//!    confidences are clamped or rejected at ingress.
//! 3. **Bounded traversal.** Hops ≤ `MAX_HOPS` ; fan-out ≤
//!    `MAX_FANOUT_PER_HOP`.

#![warn(missing_docs)]
#![warn(rust_2018_idioms)]
#![deny(unsafe_code)]

pub mod claim;
pub mod dedup;
pub mod entity;
pub mod error;
pub mod extractor;
pub mod graph;
pub mod metrics;
pub mod relation;
pub mod store;
pub mod traversal;

pub use claim::{
    promote_claims_to_relations, Claim, CLAIM_PROMOTE_CONFIDENCE, CLAIM_PROMOTE_MIN_SUPPORT,
};
pub use dedup::{classify, cosine, levenshtein_capped, DedupVerdict, COSINE_THRESHOLD, MAX_LEV_DISTANCE};
pub use entity::{canonicalize_name, validate_name, Entity, EntityKind, MAX_ENTITY_NAME_LEN};
pub use error::{KgError, Result};
pub use extractor::{
    EntitySpan, InMemoryNer, InMemoryRe, NerExtractor, RelationExtractor, RelationHypothesis,
    RelationRequest, MAX_INPUT_BYTES,
};
pub use graph::{AdjEntry, Graph, GraphSnapshot, GRAPH_SNAPSHOT_VERSION};
pub use relation::{
    validate_predicate, Predicate, Relation, BASE_PREDICATES, MAX_PREDICATE_LEN,
};
pub use store::{InMemoryStore, KgStore};
pub use traversal::{
    bfs, dfs, shortest_path, TraversalFilter, TraversalNode, MAX_FANOUT_PER_HOP, MAX_HOPS,
};

/// Static crate identifier.
pub const CRATE_NAME_VERSION: &str = concat!("efe-kg/", env!("CARGO_PKG_VERSION"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_string_well_formed() {
        assert!(CRATE_NAME_VERSION.starts_with("efe-kg/"));
    }

    #[test]
    fn reexports_compile() {
        let _: EntityKind = EntityKind::Person;
        let _: Predicate = Predicate::new("works_at").unwrap();
        let _: DedupVerdict = DedupVerdict::Merge;
    }
}
