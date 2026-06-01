//! `POST /v1/tenants/:slug/export` + `GET /v1/exports/:id` + download.

use super::state::PortabilityState;
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
pub enum PortabilityError {
    #[error("unauthenticated")]
    Unauthenticated,
    #[error("forbidden")]
    Forbidden,
    #[error("export not found")]
    NotFound,
    #[error("export not ready (current state : {0})")]
    NotReady(String),
    #[error("export expired")]
    Expired,
    #[error("invalid request : {0}")]
    Invalid(String),
    #[error("DB error : {0}")]
    Db(String),
}

impl IntoResponse for PortabilityError {
    fn into_response(self) -> Response {
        let (status, code, msg) = match &self {
            Self::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                "unauthenticated",
                self.to_string(),
            ),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden", self.to_string()),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found", self.to_string()),
            Self::NotReady(_) => (StatusCode::CONFLICT, "not_ready", self.to_string()),
            Self::Expired => (StatusCode::GONE, "expired", self.to_string()),
            Self::Invalid(_) => (StatusCode::BAD_REQUEST, "invalid_input", self.to_string()),
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
pub struct ExportRecord {
    pub id: Uuid,
    pub tenant_id: String,
    pub status: PortabilityState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_pgp_sig: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<i64>,
    pub submitted_by: String,
    pub submitted_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct SubmitRequest {
    /// Optional date-range filter to keep the bundle small.
    #[serde(default)]
    pub since: Option<DateTime<Utc>>,
    #[serde(default)]
    pub until: Option<DateTime<Utc>>,
    /// Topics to include : null = all.
    #[serde(default)]
    pub include_topics: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct DownloadResponse {
    pub presigned_url: String,
    pub expires_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_sha256: Option<String>,
}

#[async_trait]
pub trait PortabilityHandlerState: Clone + Send + Sync + 'static {
    async fn submit_export(
        &self,
        tenant_id: &str,
        submitted_by: &str,
        req: SubmitRequest,
    ) -> Result<ExportRecord, PortabilityError>;

    async fn get_export(
        &self,
        tenant_id: &str,
        id: Uuid,
    ) -> Result<Option<ExportRecord>, PortabilityError>;

    async fn build_download_url(
        &self,
        tenant_id: &str,
        id: Uuid,
    ) -> Result<DownloadResponse, PortabilityError>;

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

/// `POST /v1/tenants/:slug/export`
pub async fn submit<S: PortabilityHandlerState>(
    State(state): State<S>,
    Path(slug): Path<String>,
    req: axum::extract::Request,
) -> Result<Json<ExportRecord>, PortabilityError> {
    let (parts, body) = req.into_parts();
    let caller_tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(PortabilityError::Unauthenticated)?;
    if caller_tenant != slug {
        return Err(PortabilityError::Forbidden);
    }
    let sub = state
        .auth_subject_from_request(&parts)
        .unwrap_or_else(|| "anonymous".to_string());
    let bytes = axum::body::to_bytes(body, 16 * 1024)
        .await
        .map_err(|e| PortabilityError::Invalid(format!("body read : {}", e)))?;
    let payload: SubmitRequest = if bytes.is_empty() {
        SubmitRequest {
            since: None,
            until: None,
            include_topics: None,
        }
    } else {
        serde_json::from_slice(&bytes)
            .map_err(|e| PortabilityError::Invalid(format!("body parse : {}", e)))?
    };
    if let (Some(s), Some(u)) = (payload.since, payload.until) {
        if s > u {
            return Err(PortabilityError::Invalid("since > until".into()));
        }
    }
    let rec = state.submit_export(&slug, &sub, payload).await?;
    Ok(Json(rec))
}

/// `GET /v1/exports/:id`
pub async fn get_status<S: PortabilityHandlerState>(
    State(state): State<S>,
    Path(id): Path<Uuid>,
    req: axum::extract::Request,
) -> Result<Json<ExportRecord>, PortabilityError> {
    let (parts, _) = req.into_parts();
    let caller_tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(PortabilityError::Unauthenticated)?;
    state
        .get_export(&caller_tenant, id)
        .await?
        .map(Json)
        .ok_or(PortabilityError::NotFound)
}

/// `GET /v1/exports/:id/download`
pub async fn download<S: PortabilityHandlerState>(
    State(state): State<S>,
    Path(id): Path<Uuid>,
    req: axum::extract::Request,
) -> Result<Json<DownloadResponse>, PortabilityError> {
    let (parts, _) = req.into_parts();
    let caller_tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(PortabilityError::Unauthenticated)?;
    let url = state.build_download_url(&caller_tenant, id).await?;
    Ok(Json(url))
}

pub fn router<S: PortabilityHandlerState>(state: S) -> axum::Router {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/tenants/:slug/export", post(submit::<S>))
        .route("/exports/:id", get(get_status::<S>))
        .route("/exports/:id/download", get(download::<S>))
        .with_state(state)
}
