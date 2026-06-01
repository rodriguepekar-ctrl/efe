use super::sink::{validate_url, SinkKind, SinkStatus, SinkUrlError};
use async_trait::async_trait;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum AuditExportError {
    #[error("unauthenticated")]
    Unauthenticated,
    #[error("forbidden")]
    Forbidden,
    #[error("invalid URL : {0}")]
    InvalidUrl(SinkUrlError),
    #[error("sink not found")]
    NotFound,
    #[error("invalid request : {0}")]
    Invalid(String),
    #[error("DB error : {0}")]
    Db(String),
}

impl IntoResponse for AuditExportError {
    fn into_response(self) -> Response {
        let (status, code, msg) = match &self {
            Self::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                "unauthenticated",
                self.to_string(),
            ),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden", self.to_string()),
            Self::InvalidUrl(_) | Self::Invalid(_) => {
                (StatusCode::BAD_REQUEST, "invalid_input", self.to_string())
            }
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found", self.to_string()),
            Self::Db(_) => (
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
pub struct SinkRecord {
    pub id: Uuid,
    pub tenant_id: String,
    pub kind: SinkKind,
    pub url: String,
    pub status: SinkStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_export_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub kind: SinkKind,
    pub url: String,
    /// Customer's PGP public key for delivering the per-tenant HMAC key.
    #[serde(default)]
    pub per_tenant_key_pgp_pub: Option<String>,
    /// Live tap mode : push events as they happen (in addition to
    /// daily rollup). Default false to minimize cost.
    #[serde(default)]
    pub live_tap: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegisterResponse {
    pub sink: SinkRecord,
    /// One-time HMAC key for chain verification, PGP-encrypted with the
    /// customer's pub key. Only present on first registration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pgp_encrypted_hmac_key: Option<String>,
}

#[async_trait]
pub trait AuditExportHandlerState: Clone + Send + Sync + 'static {
    async fn register_sink(
        &self,
        tenant_id: &str,
        registered_by: &str,
        req: RegisterRequest,
    ) -> Result<RegisterResponse, AuditExportError>;

    async fn list_sinks(&self, tenant_id: &str) -> Result<Vec<SinkRecord>, AuditExportError>;

    async fn delete_sink(&self, tenant_id: &str, id: Uuid) -> Result<(), AuditExportError>;

    async fn verify_sink(&self, tenant_id: &str, id: Uuid) -> Result<SinkRecord, AuditExportError>;

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

/// `POST /v1/audit/export-sink`
pub async fn register<S: AuditExportHandlerState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<RegisterResponse>, AuditExportError> {
    let (parts, body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(AuditExportError::Unauthenticated)?;
    let sub = state
        .auth_subject_from_request(&parts)
        .unwrap_or_else(|| "anonymous".to_string());
    let bytes = axum::body::to_bytes(body, 256 * 1024)
        .await
        .map_err(|e| AuditExportError::Invalid(format!("body read : {}", e)))?;
    let payload: RegisterRequest = serde_json::from_slice(&bytes)
        .map_err(|e| AuditExportError::Invalid(format!("body parse : {}", e)))?;
    validate_url(payload.kind, &payload.url).map_err(AuditExportError::InvalidUrl)?;
    let resp = state.register_sink(&tenant, &sub, payload).await?;
    Ok(Json(resp))
}

/// `GET /v1/audit/export-sink`
pub async fn list<S: AuditExportHandlerState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<Vec<SinkRecord>>, AuditExportError> {
    let (parts, _) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(AuditExportError::Unauthenticated)?;
    Ok(Json(state.list_sinks(&tenant).await?))
}

/// `DELETE /v1/audit/export-sink/:id`
pub async fn delete<S: AuditExportHandlerState>(
    State(state): State<S>,
    Path(id): Path<Uuid>,
    req: axum::extract::Request,
) -> Result<StatusCode, AuditExportError> {
    let (parts, _) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(AuditExportError::Unauthenticated)?;
    state.delete_sink(&tenant, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /v1/audit/export-sink/:id/verify`
pub async fn verify<S: AuditExportHandlerState>(
    State(state): State<S>,
    Path(id): Path<Uuid>,
    req: axum::extract::Request,
) -> Result<Json<SinkRecord>, AuditExportError> {
    let (parts, _) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(AuditExportError::Unauthenticated)?;
    Ok(Json(state.verify_sink(&tenant, id).await?))
}

pub fn router<S: AuditExportHandlerState>(state: S) -> axum::Router {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/audit/export-sink", post(register::<S>).get(list::<S>))
        .route("/audit/export-sink/:id", axum::routing::delete(delete::<S>))
        .route("/audit/export-sink/:id/verify", post(verify::<S>))
        .with_state(state)
}
