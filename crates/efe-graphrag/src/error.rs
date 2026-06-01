// SPDX-License-Identifier: UNLICENSED
//! M33 — GraphRAG plane error taxonomy.

use thiserror::Error;

/// Canonical GraphRAG error type.
#[derive(Debug, Error)]
pub enum GraphRagError {
    /// Empty query — neither query_text nor query_vector supplied.
    #[error("empty query")]
    EmptyQuery,

    /// Plan length exceeded the configured cap.
    #[error("plan too long: {observed} > cap {cap}")]
    PlanTooLong {
        /// Observed plan length.
        observed: usize,
        /// Configured cap.
        cap: usize,
    },

    /// Alpha out of [0, 1] range.
    #[error("invalid alpha: {0}")]
    InvalidAlpha(f32),

    /// Wrapped vector-plane error.
    #[error("vector: {0}")]
    Vector(#[from] efe_vector::VectorError),

    /// Wrapped retrieval-plane error.
    #[error("retrieval: {0}")]
    Retrieval(#[from] efe_retrieval::RetrievalError),

    /// Wrapped KG-plane error.
    #[error("kg: {0}")]
    Kg(#[from] efe_kg::KgError),

    /// Catch-all.
    #[error("internal: {reason}")]
    Internal {
        /// Sanitized reason.
        reason: String,
    },
}

impl GraphRagError {
    /// Stable string identifier.
    #[must_use]
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::EmptyQuery => "graphrag.empty_query",
            Self::PlanTooLong { .. } => "graphrag.plan_too_long",
            Self::InvalidAlpha(_) => "graphrag.invalid_alpha",
            Self::Vector(_) => "graphrag.vector",
            Self::Retrieval(_) => "graphrag.retrieval",
            Self::Kg(_) => "graphrag.kg",
            Self::Internal { .. } => "graphrag.internal",
        }
    }

    /// HTTP status code.
    #[must_use]
    pub fn http_status(&self) -> u16 {
        match self {
            Self::EmptyQuery | Self::PlanTooLong { .. } | Self::InvalidAlpha(_) => 422,
            Self::Vector(e) => e.http_status(),
            Self::Retrieval(e) => e.http_status(),
            Self::Kg(e) => e.http_status(),
            Self::Internal { .. } => 500,
        }
    }
}

/// Result alias.
pub type Result<T> = std::result::Result<T, GraphRagError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_stable() {
        assert_eq!(GraphRagError::EmptyQuery.error_code(), "graphrag.empty_query");
    }

    #[test]
    fn http_inherits_from_wrapped() {
        let e = GraphRagError::Kg(efe_kg::KgError::EntityNotFound("x".into()));
        assert_eq!(e.http_status(), 404);
    }
}
