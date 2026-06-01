//! M31 — Error envelope for `/v1/billing/*`.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

/// Billing REST error.
#[derive(Debug, thiserror::Error)]
pub enum BillingApiError {
    /// Missing or invalid bearer.
    #[error("unauthenticated")]
    Unauthenticated,

    /// Caller lacks billing scope.
    #[error("forbidden")]
    Forbidden,

    /// Body malformed.
    #[error("invalid body: {0}")]
    InvalidBody(String),

    /// Wrapped billing error.
    #[error("billing: {0}")]
    Billing(#[from] efe_billing::BillingError),

    /// PG / I/O / runtime error.
    #[error("internal: {0}")]
    Internal(String),
}

impl IntoResponse for BillingApiError {
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
                "billing scope required".to_string(),
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
