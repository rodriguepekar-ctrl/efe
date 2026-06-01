//! `POST /v1/byok/rotate` + status + history + cancel.

use super::state::RotationState;
use async_trait::async_trait;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum ByokError {
    #[error("unauthenticated")]
    Unauthenticated,
    #[error("forbidden")]
    Forbidden,
    #[error("rotation already in progress")]
    AlreadyInProgress,
    #[error("rotation cannot be cancelled in state {0}")]
    NotCancellable(String),
    #[error("rotation not found")]
    NotFound,
    #[error("invalid request : {0}")]
    Invalid(String),
    #[error("KMS error : {0}")]
    Kms(String),
    #[error("DB error : {0}")]
    Db(String),
}

impl IntoResponse for ByokError {
    fn into_response(self) -> Response {
        let (status, code, msg) = match &self {
            Self::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                "unauthenticated",
                self.to_string(),
            ),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden", self.to_string()),
            Self::AlreadyInProgress => (
                StatusCode::CONFLICT,
                "already_in_progress",
                self.to_string(),
            ),
            Self::NotCancellable(_) => (StatusCode::CONFLICT, "not_cancellable", self.to_string()),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found", self.to_string()),
            Self::Invalid(_) => (StatusCode::BAD_REQUEST, "invalid_input", self.to_string()),
            Self::Kms(_) | Self::Db(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "internal error".into(),
            ),
        };
        let body = serde_json::json!({"code": code, "message": msg});
        (status, Json(body)).into_response()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RotationRecord {
    pub id: Uuid,
    pub tenant_id: String,
    pub state: RotationState,
    pub old_key_fpr: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_key_fpr: Option<String>,
    pub rows_reencrypted: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RotateRequest {
    /// PEM-encoded new public key (customer-controlled).
    pub new_key_pem: String,
    /// Optional human-readable reason for the audit trail.
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub current_key_fpr: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation_in_progress: Option<RotationRecord>,
}

#[async_trait]
pub trait ByokHandlerState: Clone + Send + Sync + 'static {
    async fn trigger_rotation(
        &self,
        tenant_id: &str,
        triggered_by: &str,
        new_key_pem: &str,
        reason: Option<&str>,
    ) -> Result<RotationRecord, ByokError>;

    async fn get_status(&self, tenant_id: &str) -> Result<StatusResponse, ByokError>;

    async fn list_rotations(
        &self,
        tenant_id: &str,
        limit: i32,
    ) -> Result<Vec<RotationRecord>, ByokError>;

    async fn cancel_rotation(&self, tenant_id: &str, id: Uuid)
        -> Result<RotationRecord, ByokError>;

    fn auth_tenant_from_request(&self, parts: &axum::http::request::Parts) -> Option<String> {
        parts
            .headers
            .get("x-tenant-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    }
    fn auth_subject_from_request(&self, parts: &axum::http::request::Parts) -> Option<String> {
        parts
            .headers
            .get("x-token-sub")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    }
}

/// `POST /v1/byok/rotate`
pub async fn rotate<S: ByokHandlerState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<RotationRecord>, ByokError> {
    let (parts, body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(ByokError::Unauthenticated)?;
    let sub = state
        .auth_subject_from_request(&parts)
        .unwrap_or_else(|| "anonymous".to_string());
    let bytes = axum::body::to_bytes(body, 64 * 1024)
        .await
        .map_err(|e| ByokError::Invalid(format!("body read : {}", e)))?;
    let payload: RotateRequest = serde_json::from_slice(&bytes)
        .map_err(|e| ByokError::Invalid(format!("body parse : {}", e)))?;
    if !payload.new_key_pem.contains("BEGIN PUBLIC KEY")
        && !payload.new_key_pem.contains("BEGIN PGP PUBLIC KEY")
    {
        return Err(ByokError::Invalid(
            "new_key_pem must be a PEM-encoded public key".into(),
        ));
    }
    let rec = state
        .trigger_rotation(
            &tenant,
            &sub,
            &payload.new_key_pem,
            payload.reason.as_deref(),
        )
        .await?;
    Ok(Json(rec))
}

/// `GET /v1/byok/status`
pub async fn status<S: ByokHandlerState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<StatusResponse>, ByokError> {
    let (parts, _) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(ByokError::Unauthenticated)?;
    Ok(Json(state.get_status(&tenant).await?))
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default = "default_limit")]
    pub limit: i32,
}

fn default_limit() -> i32 {
    10
}

/// `GET /v1/byok/rotations`
pub async fn list<S: ByokHandlerState>(
    State(state): State<S>,
    Query(q): Query<ListQuery>,
    req: axum::extract::Request,
) -> Result<Json<Vec<RotationRecord>>, ByokError> {
    let (parts, _) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(ByokError::Unauthenticated)?;
    let lim = q.limit.clamp(1, 100);
    Ok(Json(state.list_rotations(&tenant, lim).await?))
}

/// `POST /v1/byok/rotations/:id/cancel`
pub async fn cancel<S: ByokHandlerState>(
    State(state): State<S>,
    Path(id): Path<Uuid>,
    req: axum::extract::Request,
) -> Result<Json<RotationRecord>, ByokError> {
    let (parts, _) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(ByokError::Unauthenticated)?;
    Ok(Json(state.cancel_rotation(&tenant, id).await?))
}

pub fn router<S: ByokHandlerState>(state: S) -> axum::Router {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/byok/rotate", post(rotate::<S>))
        .route("/byok/status", get(status::<S>))
        .route("/byok/rotations", get(list::<S>))
        .route("/byok/rotations/:id/cancel", post(cancel::<S>))
        .with_state(state)
}
