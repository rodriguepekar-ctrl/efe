//! M33 — Error envelope for `/v1/kg/*`.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

/// KG REST error.
#[derive(Debug, thiserror::Error)]
pub enum KgApiError {
    /// Missing or invalid bearer.
    #[error("unauthenticated")]
    Unauthenticated,

    /// Caller lacks `kg:write` scope.
    #[error("forbidden")]
    Forbidden,

    /// Body malformed.
    #[error("invalid body: {0}")]
    InvalidBody(String),

    /// Internal error.
    #[error("internal: {0}")]
    Internal(String),

    /// Wrapped KG-plane error.
    #[error("kg: {0}")]
    Kg(#[from] efe_kg::KgError),
}

impl IntoResponse for KgApiError {
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
                "kg:write scope required".to_string(),
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
            Self::Kg(e) => (
                StatusCode::from_u16(e.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                e.error_code().to_string(),
                e.to_string(),
            ),
        };
        let body = serde_json::json!({"code": code, "message": msg});
        (status, Json(body)).into_response()
    }
}
