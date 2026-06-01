// SPDX-License-Identifier: UNLICENSED
//! # `efe-vector` — M32 vector index plane.
//!
//! This crate provides everything needed to **store, index, and
//! search** dense vector embeddings. It is the dense-retrieval half
//! of the M32 milestone ; the sparse + hybrid layer lives in the
//! companion `efe-retrieval` crate.
//!
//! ## Scope
//!
//! - **Distance metrics** : cosine, dot, L2.
//! - **Flat (brute-force) index** : golden reference, optimal for
//!   small collections (≤ 10 k docs).
//! - **HNSW index** : approximate k-NN graph for large collections.
//! - **Binary quantization** : 32× memory savings with > 0.85
//!   recall.
//! - **Collection façade** : tenant + name + dim + metric.
//! - **Snapshot / replay** : bincode → S3 mirror.
//!
//! ## Architectural invariants
//!
//! 1. **No global state.** Every collection is independent ; no
//!    cross-collection coupling.
//! 2. **Validated by ingress.** Vectors are checked for dim + NaN
//!    on every insert + query.
//! 3. **Tenant isolation.** A collection belongs to exactly one
//!    tenant ; cross-tenant search is impossible by construction.
//! 4. **Score contract.** Every public search API returns
//!    `score: f32` where **higher means more similar**.

#![warn(missing_docs)]
#![warn(rust_2018_idioms)]
#![deny(unsafe_code)]

pub mod collection;
pub mod distance;
pub mod error;
pub mod flat;
pub mod hnsw;
pub mod metrics;
pub mod quantize;
pub mod segment;
pub mod store;

pub use collection::{
    validate_collection_name, Collection, CollectionConfig, MAX_DOCS_PER_COLLECTION,
};
pub use distance::{
    cosine_normalized, cosine_similarity, dot_product, l2_distance, l2_normalize,
    squared_l2_norm, validate_vector, Distance, MAX_DIM, ZERO_NORM_EPS,
};
pub use error::{Result, VectorError};
pub use flat::{FlatIndex, SearchHit, BRUTE_FORCE_MAX_VECTORS, MAX_K};
pub use hnsw::{HnswConfig, HnswIndex};
pub use quantize::BinaryVector;
pub use segment::{
    deserialize_snapshot, serialize_snapshot, Segment, SegmentSnapshot,
    SNAPSHOT_VERSION,
};
pub use store::{DocId, DocMap, InMemoryStore, VectorDoc, VectorStore};

/// Static crate identifier.
pub const CRATE_NAME_VERSION: &str = concat!("efe-vector/", env!("CARGO_PKG_VERSION"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_string_well_formed() {
        assert!(CRATE_NAME_VERSION.starts_with("efe-vector/"));
    }

    #[test]
    fn reexports_compile() {
        let _: Distance = Distance::Cosine;
        let _: SearchHit = SearchHit { id: "x".into(), score: 1.0 };
        let _: BinaryVector = BinaryVector::from_f32(&[1.0, -1.0]).unwrap();
    }
}
