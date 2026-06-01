//! M31 — Error envelope for `/v1/usage/*`.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

/// Usage REST error.
#[derive(Debug, thiserror::Error)]
pub enum UsageApiError {
    /// Missing or invalid bearer.
    #[error("unauthenticated")]
    Unauthenticated,

    /// Body malformed.
    #[error("invalid body: {0}")]
    InvalidBody(String),

    /// Query parameter malformed.
    #[error("invalid query: {0}")]
    InvalidQuery(String),

    /// PG / I/O / runtime error.
    #[error("internal: {0}")]
    Internal(String),

    /// Wrapped metering error.
    #[error("metering: {0}")]
    Metering(#[from] efe_metering::MeteringError),

    /// Wrapped billing error.
    #[error("billing: {0}")]
    Billing(#[from] efe_billing::BillingError),
}

impl IntoResponse for UsageApiError {
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
            Self::InvalidQuery(m) => (
                StatusCode::BAD_REQUEST,
                "invalid_query".to_string(),
                m.clone(),
            ),
            Self::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal".to_string(),
                "internal error".to_string(),
            ),
            Self::Metering(e) => (
                StatusCode::from_u16(e.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                e.error_code().to_string(),
                e.to_string(),
            ),
            Self::Billing(e) => (
                StatusCode::from_u16(e.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                e.error_code().to_string(),
                e.to_string(),
            ),
        };
        let body = serde_json::json!({"code": code, "message": msg});
        (status, Json(body)).into_response()
    }
}
