//! M30 — Error response envelope for `/v1/region/*`.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

/// Region REST error.
#[derive(Debug, thiserror::Error)]
pub enum RegionApiError {
    /// Bearer / peer-header missing or invalid.
    #[error("unauthenticated")]
    Unauthenticated,

    /// Caller lacks operator scope (failover, drill).
    #[error("forbidden")]
    Forbidden,

    /// Failover request was rate-limited.
    #[error("rate-limited")]
    RateLimited {
        /// Seconds to wait before retrying.
        retry_after_secs: u32,
    },

    /// Request body malformed.
    #[error("invalid body: {0}")]
    InvalidBody(String),

    /// PG / I/O / runtime error.
    #[error("internal: {0}")]
    Internal(String),

    /// Wrapped region-plane error.
    #[error("region error: {0}")]
    Region(#[from] efe_region::RegionError),
}

impl IntoResponse for RegionApiError {
    fn into_response(self) -> Response {
        let (status, code, msg) = match &self {
            Self::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                "unauthenticated".to_string(),
                "missing bearer or peer header".to_string(),
            ),
            Self::Forbidden => (
                StatusCode::FORBIDDEN,
                "forbidden".to_string(),
                "operator scope required".to_string(),
            ),
            Self::RateLimited { retry_after_secs } => (
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited".to_string(),
                format!("retry after {retry_after_secs}s"),
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
            Self::Region(e) => (
                StatusCode::from_u16(e.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                e.error_code().to_string(),
                e.to_string(),
            ),
        };
        let body = serde_json::json!({"code": code, "message": msg});
        (status, Json(body)).into_response()
    }
}
