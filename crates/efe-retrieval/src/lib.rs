// SPDX-License-Identifier: UNLICENSED
//! # `efe-retrieval` — M32 hybrid retrieval plane.
//!
//! Pairs a dense vector index ([`efe_vector::Collection`]) with a
//! sparse BM25 inverted index, then fuses the two result lists via
//! Reciprocal Rank Fusion (RRF) or a weighted score merge.
//!
//! ## Modules
//!
//! - [`bm25`]    — Inverted-index BM25 scorer.
//! - [`sparse`]  — Sparse vector representation (token → weight).
//! - [`scoring`] — Score normalization (min-max, z-score).
//! - [`rrf`]     — Reciprocal Rank Fusion + weighted merge.
//! - [`hybrid`]  — Orchestrator combining dense + sparse.
//! - [`error`]   — Typed errors.

#![warn(missing_docs)]
#![warn(rust_2018_idioms)]
#![deny(unsafe_code)]

pub mod bm25;
pub mod error;
pub mod hybrid;
pub mod rrf;
pub mod scoring;
pub mod sparse;

pub use bm25::{Bm25Index, Bm25Hit};
pub use error::{Result, RetrievalError};
pub use hybrid::{HybridConfig, HybridMode, HybridSearcher};
pub use rrf::{rrf_fuse, weighted_fuse, FusedHit, DEFAULT_RRF_K};
pub use scoring::{minmax_normalize, zscore_normalize};
pub use sparse::SparseVector;

/// Static crate identifier.
pub const CRATE_NAME_VERSION: &str = concat!("efe-retrieval/", env!("CARGO_PKG_VERSION"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reexports_compile() {
        let _: HybridMode = HybridMode::Dense;
    }
}
