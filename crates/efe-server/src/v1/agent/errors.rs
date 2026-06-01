//! Error type for the `/v1/agent/*` REST gateway.
//!
//! Maps internal errors to HTTP statuses + bodies. Safety rules :
//! - Sandbox escape errors NEVER leak details to the response body.
//! - Database errors collapse to a generic 500.
//! - All `Internal` paths emit a tracing event with a correlation ID.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum AgentApiError {
    #[error("unauthenticated")]
    Unauthenticated,

    #[error("invalid bearer token")]
    InvalidToken,

    #[error("token stale : age={age_seconds}s max={max}s")]
    TokenStale { age_seconds: i64, max: i64 },

    #[error("forbidden : {0}")]
    Forbidden(String),

    #[error("not found")]
    NotFound,

    #[error("invalid input : {0}")]
    InvalidInput(String),

    #[error("failed precondition : {0}")]
    FailedPrecondition(String),

    #[error("rate limit exceeded ; retry after {retry_after_seconds}s")]
    RateLimited { retry_after_seconds: u64 },

    #[error("upstream service unavailable")]
    Unavailable,

    #[error("internal error")]
    Internal(String),

    #[error("internal error")]
    SandboxEscape, // never carries details
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
    correlation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after_seconds: Option<u64>,
}

impl IntoResponse for AgentApiError {
    fn into_response(self) -> Response {
        let correlation_id = Some(Uuid::new_v4().to_string());

        let (status, code, message, retry_after) = match &self {
            Self::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                "unauthenticated",
                "missing or invalid bearer token".to_string(),
                None,
            ),
            Self::InvalidToken => (
                StatusCode::UNAUTHORIZED,
                "invalid_token",
                "bearer token failed validation".to_string(),
                None,
            ),
            Self::TokenStale { age_seconds, max } => (
                StatusCode::UNAUTHORIZED,
                "token_stale",
                format!("token age {}s > max {}s", age_seconds, max),
                None,
            ),
            Self::Forbidden(msg) => (StatusCode::FORBIDDEN, "forbidden", msg.clone(), None),
            Self::NotFound => (
                StatusCode::NOT_FOUND,
                "not_found",
                "resource not found".to_string(),
                None,
            ),
            Self::InvalidInput(msg) => {
                (StatusCode::BAD_REQUEST, "invalid_input", msg.clone(), None)
            }
            Self::FailedPrecondition(msg) => (
                StatusCode::PRECONDITION_FAILED,
                "failed_precondition",
                msg.clone(),
                None,
            ),
            Self::RateLimited {
                retry_after_seconds,
            } => (
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "rate limit exceeded".to_string(),
                Some(*retry_after_seconds),
            ),
            Self::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "unavailable",
                "upstream service unavailable".to_string(),
                None,
            ),
            Self::SandboxEscape => {
                tracing::error!(
                    correlation_id = ?correlation_id,
                    "SECURITY P0 — sandbox escape attempt encountered ; details suppressed",
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "internal error".to_string(),
                    None,
                )
            }
            Self::Internal(detail) => {
                tracing::error!(
                    correlation_id = ?correlation_id,
                    detail = %detail,
                    "agent api internal error",
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "internal error".to_string(),
                    None,
                )
            }
        };

        let body = ErrorBody {
            code,
            message,
            correlation_id,
            retry_after_seconds: retry_after,
        };

        let mut resp = (status, Json(body)).into_response();
        if let Some(secs) = retry_after {
            resp.headers_mut().insert(
                axum::http::header::RETRY_AFTER,
                axum::http::HeaderValue::from(secs),
            );
        }
        resp
    }
}

impl From<sqlx::Error> for AgentApiError {
    fn from(e: sqlx::Error) -> Self {
        match e {
            sqlx::Error::RowNotFound => Self::NotFound,
            other => Self::Internal(other.to_string()),
        }
    }
}

impl From<serde_json::Error> for AgentApiError {
    fn from(e: serde_json::Error) -> Self {
        Self::Internal(format!("serde_json: {}", e))
    }
}
