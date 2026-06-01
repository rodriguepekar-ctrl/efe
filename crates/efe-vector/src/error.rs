// SPDX-License-Identifier: UNLICENSED
//! M32 — Vector plane error taxonomy.
//!
//! Stable string identifiers map 1:1 to HTTP status codes consumed
//! by `v1/vector/*` REST handlers.

use thiserror::Error;

/// Canonical vector-plane error type.
#[derive(Debug, Error)]
pub enum VectorError {
    /// Vector dimension does not match the collection's configured
    /// dim.
    #[error("dimension mismatch: vector dim {observed}, collection dim {expected}")]
    DimensionMismatch {
        /// Dim of the incoming vector.
        observed: usize,
        /// Configured collection dim.
        expected: usize,
    },

    /// Vector dimension exceeds the hard cap (`MAX_DIM`).
    #[error("dimension too large: {observed} > cap {cap}")]
    DimensionTooLarge {
        /// Dim of the incoming vector.
        observed: usize,
        /// Hard cap.
        cap: usize,
    },

    /// k (top-k) is out of range.
    #[error("k out of range: {observed}, expected 1..={max}")]
    KOutOfRange {
        /// Observed k.
        observed: usize,
        /// Max allowed.
        max: usize,
    },

    /// Collection slug does not match `[a-z0-9_-]{1,32}`.
    #[error("invalid collection name: {0}")]
    InvalidCollectionName(String),

    /// Collection not found.
    #[error("collection not found: {0}")]
    CollectionNotFound(String),

    /// Collection already exists.
    #[error("collection already exists: {0}")]
    CollectionAlreadyExists(String),

    /// Collection is full (cardinality cap reached).
    #[error("collection full: {observed} >= cap {cap}")]
    CollectionFull {
        /// Observed cardinality.
        observed: u64,
        /// Configured cap.
        cap: u64,
    },

    /// Document not found.
    #[error("document not found: {0}")]
    DocumentNotFound(String),

    /// Vector payload is NaN / infinite.
    #[error("invalid vector: {reason}")]
    InvalidVector {
        /// Sanitized reason.
        reason: String,
    },

    /// Filter expression failed to parse / type-check.
    #[error("invalid filter: {0}")]
    InvalidFilter(String),

    /// Persistence layer error.
    #[error("storage error: {reason}")]
    Storage {
        /// Sanitized reason.
        reason: String,
    },

    /// HNSW build / query error.
    #[error("hnsw error: {reason}")]
    Hnsw {
        /// Sanitized reason.
        reason: String,
    },

    /// Bincode serialization failure (snapshot).
    #[error("snapshot serde: {reason}")]
    SnapshotSerde {
        /// Sanitized reason.
        reason: String,
    },

    /// Catch-all internal error.
    #[error("internal: {reason}")]
    Internal {
        /// Sanitized reason.
        reason: String,
    },
}

impl VectorError {
    /// Stable string identifier consumed by the REST layer.
    #[must_use]
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::DimensionMismatch { .. } => "vector.dim_mismatch",
            Self::DimensionTooLarge { .. } => "vector.dim_too_large",
            Self::KOutOfRange { .. } => "vector.k_out_of_range",
            Self::InvalidCollectionName(_) => "vector.invalid_collection_name",
            Self::CollectionNotFound(_) => "vector.collection_not_found",
            Self::CollectionAlreadyExists(_) => "vector.collection_exists",
            Self::CollectionFull { .. } => "vector.collection_full",
            Self::DocumentNotFound(_) => "vector.doc_not_found",
            Self::InvalidVector { .. } => "vector.invalid_vector",
            Self::InvalidFilter(_) => "vector.invalid_filter",
            Self::Storage { .. } => "vector.storage",
            Self::Hnsw { .. } => "vector.hnsw",
            Self::SnapshotSerde { .. } => "vector.snapshot_serde",
            Self::Internal { .. } => "vector.internal",
        }
    }

    /// Recommended HTTP status code.
    #[must_use]
    pub fn http_status(&self) -> u16 {
        match self {
            Self::DimensionMismatch { .. }
            | Self::KOutOfRange { .. }
            | Self::InvalidVector { .. }
            | Self::InvalidFilter(_) => 422,
            Self::DimensionTooLarge { .. } => 413,
            Self::InvalidCollectionName(_) => 400,
            Self::CollectionNotFound(_) | Self::DocumentNotFound(_) => 404,
            Self::CollectionAlreadyExists(_) => 409,
            Self::CollectionFull { .. } => 409,
            Self::Storage { .. } | Self::SnapshotSerde { .. } => 503,
            Self::Hnsw { .. } | Self::Internal { .. } => 500,
        }
    }

    /// `true` iff retrying the operation may succeed.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Storage { .. } | Self::SnapshotSerde { .. })
    }
}

/// Result alias for the crate.
pub type Result<T> = std::result::Result<T, VectorError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_stable() {
        assert_eq!(
            VectorError::DimensionMismatch { observed: 1, expected: 0 }.error_code(),
            "vector.dim_mismatch"
        );
        assert_eq!(
            VectorError::CollectionNotFound("x".into()).error_code(),
            "vector.collection_not_found"
        );
    }

    #[test]
    fn http_status_mapping_sane() {
        assert_eq!(
            VectorError::DimensionMismatch { observed: 1, expected: 0 }.http_status(),
            422
        );
        assert_eq!(
            VectorError::CollectionNotFound("x".into()).http_status(),
            404
        );
        assert_eq!(
            VectorError::CollectionAlreadyExists("x".into()).http_status(),
            409
        );
        assert_eq!(
            VectorError::Storage { reason: "x".into() }.http_status(),
            503
        );
    }

    #[test]
    fn retryable_set_minimal() {
        assert!(VectorError::Storage { reason: "x".into() }.is_retryable());
        assert!(!VectorError::DimensionMismatch { observed: 1, expected: 0 }.is_retryable());
    }
}
