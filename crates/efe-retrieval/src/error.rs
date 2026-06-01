// SPDX-License-Identifier: UNLICENSED
//! M32 — Retrieval error taxonomy.

use thiserror::Error;

/// Canonical retrieval-plane error type.
#[derive(Debug, Error)]
pub enum RetrievalError {
    /// BM25 query string was empty / whitespace-only.
    #[error("empty query")]
    EmptyQuery,

    /// Token id out of range (sparse vector validation).
    #[error("token id out of range: {0}")]
    TokenIdOutOfRange(u32),

    /// RRF received an empty input list.
    #[error("rrf requires at least one input run")]
    RrfNoRuns,

    /// Wrapped vector-plane error (dense path).
    #[error("dense: {0}")]
    Dense(#[from] efe_vector::VectorError),

    /// Catch-all.
    #[error("internal: {reason}")]
    Internal {
        /// Sanitized reason.
        reason: String,
    },
}

impl RetrievalError {
    /// Stable string identifier.
    #[must_use]
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::EmptyQuery => "retrieval.empty_query",
            Self::TokenIdOutOfRange(_) => "retrieval.token_id_oor",
            Self::RrfNoRuns => "retrieval.rrf_no_runs",
            Self::Dense(_) => "retrieval.dense",
            Self::Internal { .. } => "retrieval.internal",
        }
    }

    /// HTTP status code.
    #[must_use]
    pub fn http_status(&self) -> u16 {
        match self {
            Self::EmptyQuery | Self::TokenIdOutOfRange(_) | Self::RrfNoRuns => 422,
            Self::Dense(e) => e.http_status(),
            Self::Internal { .. } => 500,
        }
    }
}

/// Result alias.
pub type Result<T> = std::result::Result<T, RetrievalError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_stable() {
        assert_eq!(RetrievalError::EmptyQuery.error_code(), "retrieval.empty_query");
        assert_eq!(RetrievalError::RrfNoRuns.error_code(), "retrieval.rrf_no_runs");
    }

    #[test]
    fn http_mapping_uses_dense_status() {
        let e = RetrievalError::Dense(efe_vector::VectorError::CollectionNotFound("x".into()));
        assert_eq!(e.http_status(), 404);
    }
}
