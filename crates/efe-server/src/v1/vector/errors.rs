//! M32 — Error envelope for `/v1/vector/*`.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

/// Vector REST error.
#[derive(Debug, thiserror::Error)]
pub enum VectorApiError {
    /// Missing or invalid bearer.
    #[error("unauthenticated")]
    Unauthenticated,

    /// Caller lacks the `vector:write` scope (create / insert /
    /// delete / snapshot).
    #[error("forbidden")]
    Forbidden,

    /// Body malformed.
    #[error("invalid body: {0}")]
    InvalidBody(String),

    /// PG / I/O / runtime error.
    #[error("internal: {0}")]
    Internal(String),

    /// Wrapped vector-plane error.
    #[error("vector: {0}")]
    Vector(#[from] efe_vector::VectorError),

    /// Wrapped retrieval-plane error.
    #[error("retrieval: {0}")]
    Retrieval(#[from] efe_retrieval::RetrievalError),
}

impl IntoResponse for VectorApiError {
    fn into_response(self) -> Response {
        let (status, code, msg) = match &self {
            Self::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                "unauthenticated".to_string(),
                "missing or invalid bearer".to_string(),
            ),
            Self::Forbidden => (
                StatusCode::FORBIDDEN,
                "forbidden".to_string(),
                "vector:write scope required".to_string(),
            ),
            Self::InvalidBody(m) => (
                StatusCode::BAD_REQUEST,
                "invalid_body".to_string(),
                m.clone(),
            ),
            Self::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal".to_string(),
                "internal error".to_string(),
            ),
            Self::Vector(e) => (
                StatusCode::from_u16(e.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                e.error_code().to_string(),
                e.to_string(),
            ),
            Self::Retrieval(e) => (
                StatusCode::from_u16(e.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                e.error_code().to_string(),
                e.to_string(),
            ),
        };
        let body = serde_json::json!({"code": code, "message": msg});
        (status, Json(body)).into_response()
    }
}
