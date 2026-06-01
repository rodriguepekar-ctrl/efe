// SPDX-License-Identifier: UNLICENSED
//! M33 — KG plane error taxonomy.

use thiserror::Error;

/// Canonical KG-plane error type.
#[derive(Debug, Error)]
pub enum KgError {
    /// Tenant identifier failed validation.
    #[error("invalid tenant: {0}")]
    InvalidTenant(String),

    /// Entity / relation kind unknown to the predicate catalog.
    #[error("unknown predicate: {0}")]
    UnknownPredicate(String),

    /// Entity name failed validation (empty or > MAX_NAME_LEN).
    #[error("invalid entity name: {reason}")]
    InvalidEntityName {
        /// Sanitized reason.
        reason: String,
    },

    /// Entity not found.
    #[error("entity not found: {0}")]
    EntityNotFound(String),

    /// Relation not found.
    #[error("relation not found: {0}")]
    RelationNotFound(String),

    /// Entity already exists.
    #[error("entity already exists: {0}")]
    EntityAlreadyExists(String),

    /// Traversal exceeded the configured caps.
    #[error("traversal cap exceeded: {observed} > cap {cap}")]
    TraversalCapExceeded {
        /// Observed hops or fan-out.
        observed: usize,
        /// Configured cap.
        cap: usize,
    },

    /// Graph cycle detected (informational).
    #[error("graph cycle at entity: {0}")]
    CycleDetected(String),

    /// NER extractor returned an error or invalid payload.
    #[error("ner extractor: {reason}")]
    NerExtractor {
        /// Sanitized reason.
        reason: String,
    },

    /// HTTP transport error talking to the extractor.
    #[error("ner transport: {source}")]
    NerTransport {
        /// Underlying error.
        #[source]
        source: reqwest::Error,
    },

    /// NER extractor request timed out.
    #[error("ner timeout after {elapsed_ms} ms")]
    NerTimeout {
        /// Elapsed wall-clock milliseconds.
        elapsed_ms: u64,
    },

    /// Input text exceeded the cap.
    #[error("input too large: {observed} bytes > cap {cap}")]
    InputTooLarge {
        /// Observed bytes.
        observed: usize,
        /// Configured cap.
        cap: usize,
    },

    /// Storage layer error.
    #[error("storage: {reason}")]
    Storage {
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

impl KgError {
    /// Stable string identifier.
    #[must_use]
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::InvalidTenant(_) => "kg.invalid_tenant",
            Self::UnknownPredicate(_) => "kg.unknown_predicate",
            Self::InvalidEntityName { .. } => "kg.invalid_entity_name",
            Self::EntityNotFound(_) => "kg.entity_not_found",
            Self::RelationNotFound(_) => "kg.relation_not_found",
            Self::EntityAlreadyExists(_) => "kg.entity_exists",
            Self::TraversalCapExceeded { .. } => "kg.traversal_cap",
            Self::CycleDetected(_) => "kg.cycle_detected",
            Self::NerExtractor { .. } => "kg.ner_extractor",
            Self::NerTransport { .. } => "kg.ner_transport",
            Self::NerTimeout { .. } => "kg.ner_timeout",
            Self::InputTooLarge { .. } => "kg.input_too_large",
            Self::Storage { .. } => "kg.storage",
            Self::Internal { .. } => "kg.internal",
        }
    }

    /// HTTP status code.
    #[must_use]
    pub fn http_status(&self) -> u16 {
        match self {
            Self::InvalidTenant(_)
            | Self::UnknownPredicate(_)
            | Self::InvalidEntityName { .. }
            | Self::TraversalCapExceeded { .. } => 422,
            Self::InputTooLarge { .. } => 413,
            Self::EntityNotFound(_) | Self::RelationNotFound(_) => 404,
            Self::EntityAlreadyExists(_) => 409,
            Self::CycleDetected(_) => 200, // informational
            Self::NerExtractor { .. } => 502,
            Self::NerTransport { .. } => 503,
            Self::NerTimeout { .. } => 504,
            Self::Storage { .. } => 503,
            Self::Internal { .. } => 500,
        }
    }

    /// `true` iff retrying the operation may succeed.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::NerTransport { .. } | Self::NerTimeout { .. } | Self::Storage { .. }
        )
    }
}

/// Result alias.
pub type Result<T> = std::result::Result<T, KgError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_stable() {
        assert_eq!(
            KgError::EntityNotFound("x".into()).error_code(),
            "kg.entity_not_found"
        );
        assert_eq!(
            KgError::UnknownPredicate("x".into()).error_code(),
            "kg.unknown_predicate"
        );
    }

    #[test]
    fn http_mapping_sane() {
        assert_eq!(KgError::EntityNotFound("x".into()).http_status(), 404);
        assert_eq!(KgError::EntityAlreadyExists("x".into()).http_status(), 409);
        assert_eq!(KgError::InputTooLarge { observed: 1, cap: 0 }.http_status(), 413);
        assert_eq!(KgError::NerTimeout { elapsed_ms: 1 }.http_status(), 504);
    }

    #[test]
    fn retryable_set_minimal() {
        assert!(KgError::NerTimeout { elapsed_ms: 1 }.is_retryable());
        assert!(!KgError::EntityNotFound("x".into()).is_retryable());
    }
}
