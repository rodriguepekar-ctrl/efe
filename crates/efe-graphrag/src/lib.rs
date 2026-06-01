// SPDX-License-Identifier: UNLICENSED
//! # `efe-graphrag` — M33 GraphRAG plane.
//!
//! Orchestrates **dense vector search** ([`efe_vector::Collection`])
//! with **graph traversal** ([`efe_kg::Graph`]) to deliver
//! multi-hop retrieval.
//!
//! Flow :
//!
//! 1. `vector_search(query)` → dense candidates.
//! 2. `entities_mentioned_in(candidates)` → graph seeds.
//! 3. `bfs(seeds, max_hops)` → neighbourhood.
//! 4. `fuse(dense_scores, graph_distances)` → ranked output.

#![warn(missing_docs)]
#![warn(rust_2018_idioms)]
#![deny(unsafe_code)]

pub mod error;
pub mod fusion;
pub mod multi_hop;
pub mod planner;
pub mod scoring;

pub use error::{GraphRagError, Result};
pub use fusion::{fuse_scores, GraphRagHit, DEFAULT_ALPHA, DEFAULT_DECAY};
pub use multi_hop::MultiHopExecutor;
pub use planner::{plan_query, PlanStep, QueryPlan};
pub use scoring::{graph_decay, normalize_dense_scores};

/// Static crate identifier.
pub const CRATE_NAME_VERSION: &str = concat!("efe-graphrag/", env!("CARGO_PKG_VERSION"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_string_well_formed() {
        assert!(CRATE_NAME_VERSION.starts_with("efe-graphrag/"));
    }
}
