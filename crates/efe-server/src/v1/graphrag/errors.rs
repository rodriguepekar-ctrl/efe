//! M33 — Error envelope for `/v1/graphrag/*`.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

/// GraphRAG REST error.
#[derive(Debug, thiserror::Error)]
pub enum GraphRagApiError {
    /// Missing or invalid bearer.
    #[error("unauthenticated")]
    Unauthenticated,

    /// Body malformed.
    #[error("invalid body: {0}")]
    InvalidBody(String),

    /// Internal error.
    #[error("internal: {0}")]
    Internal(String),

    /// Wrapped GraphRAG error.
    #[error("graphrag: {0}")]
    GraphRag(#[from] efe_graphrag::GraphRagError),
}

impl IntoResponse for GraphRagApiError {
    fn into_response(self) -> Response {
        let (status, code, msg) = match &self {
            Self::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                "unauthenticated".to_string(),
                "missing or invalid bearer".to_string(),
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
            Self::GraphRag(e) => (
                StatusCode::from_u16(e.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                e.error_code().to_string(),
                e.to_string(),
            ),
        };
        let body = serde_json::json!({"code": code, "message": msg});
        (status, Json(body)).into_response()
    }
}
