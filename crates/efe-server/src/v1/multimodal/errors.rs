//! M30 — Error response envelope for `/v1/multimodal/*`.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

/// Multimodal REST error. Carries the canonical
/// `efe_multimodal::MultimodalError` and any handler-local variant
/// (auth, JSON decode, etc.).
#[derive(Debug, thiserror::Error)]
pub enum MultimodalApiError {
    /// Bearer token missing or invalid.
    #[error("unauthenticated")]
    Unauthenticated,

    /// Caller is authenticated but lacks the multimodal scope.
    #[error("forbidden")]
    Forbidden,

    /// Request body could not be decoded.
    #[error("invalid body: {0}")]
    InvalidBody(String),

    /// Tenant policy not found in PG. Should never happen in steady
    /// state ; surfaced as 404.
    #[error("tenant policy not found: {0}")]
    PolicyNotFound(String),

    /// PG / I/O / runtime error.
    #[error("internal error: {0}")]
    Internal(String),

    /// Wrapped multimodal-plane error.
    #[error("multimodal error: {0}")]
    Multimodal(#[from] efe_multimodal::MultimodalError),
}

impl IntoResponse for MultimodalApiError {
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
                "multimodal scope not granted".to_string(),
            ),
            Self::InvalidBody(m) => (
                StatusCode::BAD_REQUEST,
                "invalid_body".to_string(),
                m.clone(),
            ),
            Self::PolicyNotFound(t) => (
                StatusCode::NOT_FOUND,
                "policy_not_found".to_string(),
                format!("no multimodal policy for tenant {t}"),
            ),
            Self::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal".to_string(),
                "internal error".to_string(),
            ),
            Self::Multimodal(e) => (
                StatusCode::from_u16(e.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                e.error_code().to_string(),
                e.to_string(),
            ),
        };
        let body = serde_json::json!({"code": code, "message": msg});
        (status, Json(body)).into_response()
    }
}
