//! Axum handlers for `/v1/batch/*`.
//!
//! Same pattern as the webhooks handler : state abstracted via a trait
//! so tests can inject a fake DB + queue.

use super::job::{BatchJob, BatchStatus, InputFormat};
use super::s3::{check_against_allowlist, parse_s3_url, S3UrlError};
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
pub enum BatchError {
    #[error("invalid request : {0}")]
    Invalid(String),
    #[error("invalid input URL : {0}")]
    InvalidInputUrl(S3UrlError),
    #[error("invalid output URL : {0}")]
    InvalidOutputUrl(S3UrlError),
    #[error("job not found")]
    NotFound,
    #[error("unauthenticated")]
    Unauthenticated,
    #[error("forbidden")]
    Forbidden,
    #[error("DB error : {0}")]
    Db(String),
    #[error("queue error : {0}")]
    Queue(String),
}

impl IntoResponse for BatchError {
    fn into_response(self) -> Response {
        let (status, code, msg) = match &self {
            Self::Invalid(_) | Self::InvalidInputUrl(_) | Self::InvalidOutputUrl(_) => {
                (StatusCode::BAD_REQUEST, "invalid_input", self.to_string())
            }
            Self::NotFound => (
                StatusCode::NOT_FOUND,
                "not_found",
                "batch job not found".into(),
            ),
            Self::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                "unauthenticated",
                self.to_string(),
            ),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden", self.to_string()),
            Self::Db(_) | Self::Queue(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "internal error".into(),
            ),
        };
        let body = serde_json::json!({"code": code, "message": msg});
        (status, Json(body)).into_response()
    }
}

#[derive(Debug, Deserialize)]
pub struct SubmitRequest {
    pub tenant_id: String,
    pub catalog_id: String,
    pub input_url: String,
    #[serde(default = "default_input_format")]
    pub input_format: InputFormat,
    #[serde(default = "default_input_column")]
    pub input_column: String,
    pub output_url: String,
    #[serde(default = "default_input_format")]
    pub output_format: InputFormat,
    #[serde(default)]
    pub callback_webhook_id: Option<Uuid>,
    #[serde(default)]
    pub max_cost_usd: Option<f64>,
    #[serde(default = "default_mode")]
    pub mode: i16,
}

fn default_input_format() -> InputFormat {
    InputFormat::Parquet
}

fn default_input_column() -> String {
    "text".into()
}

fn default_mode() -> i16 {
    1
}

#[derive(Debug, Serialize)]
pub struct SubmitResponse {
    pub job: BatchJob,
}

/// Trait the state object must satisfy.
#[async_trait]
pub trait BatchHandlerState: Clone + Send + Sync + 'static {
    async fn submit(&self, tenant_id: &str, req: SubmitRequest) -> Result<BatchJob, BatchError>;
    async fn get(&self, tenant_id: &str, id: Uuid) -> Result<Option<BatchJob>, BatchError>;
    async fn cancel(&self, tenant_id: &str, id: Uuid) -> Result<BatchJob, BatchError>;
    async fn list(
        &self,
        tenant_id: &str,
        status: Option<BatchStatus>,
        limit: i32,
    ) -> Result<Vec<BatchJob>, BatchError>;
    async fn result_url(&self, tenant_id: &str, id: Uuid) -> Result<String, BatchError>;

    /// Tenant S3 bucket allow-list for this tenant. Loaded from PG
    /// `tenant_s3_allowlist`. Cached per-request.
    async fn allowed_buckets(&self, tenant_id: &str) -> Result<Vec<String>, BatchError>;

    fn auth_tenant_from_request(&self, parts: &axum::http::request::Parts) -> Option<String> {
        parts
            .headers
            .get("x-tenant-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    }
}

/// `POST /v1/batch/classify`
pub async fn submit<S: BatchHandlerState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<SubmitResponse>, BatchError> {
    let (parts, body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(BatchError::Unauthenticated)?;

    let bytes = axum::body::to_bytes(body, 64 * 1024)
        .await
        .map_err(|e| BatchError::Invalid(format!("body read : {}", e)))?;
    let body: SubmitRequest = serde_json::from_slice(&bytes)
        .map_err(|e| BatchError::Invalid(format!("body parse : {}", e)))?;

    if body.tenant_id != tenant {
        return Err(BatchError::Forbidden);
    }

    // Validate URLs.
    let input = parse_s3_url(&body.input_url).map_err(BatchError::InvalidInputUrl)?;
    let output = parse_s3_url(&body.output_url).map_err(BatchError::InvalidOutputUrl)?;
    let allow = state.allowed_buckets(&tenant).await?;
    check_against_allowlist(&input, &allow).map_err(BatchError::InvalidInputUrl)?;
    check_against_allowlist(&output, &allow).map_err(BatchError::InvalidOutputUrl)?;

    let job = state.submit(&tenant, body).await?;
    Ok(Json(SubmitResponse { job }))
}

/// `GET /v1/batch/:id`
pub async fn get_one<S: BatchHandlerState>(
    State(state): State<S>,
    Path(id): Path<Uuid>,
    req: axum::extract::Request,
) -> Result<Json<BatchJob>, BatchError> {
    let (parts, _) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(BatchError::Unauthenticated)?;
    let j = state.get(&tenant, id).await?.ok_or(BatchError::NotFound)?;
    Ok(Json(j))
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub status: Option<BatchStatus>,
    #[serde(default = "default_limit")]
    pub limit: i32,
}

fn default_limit() -> i32 {
    50
}

/// `GET /v1/batch`
pub async fn list<S: BatchHandlerState>(
    State(state): State<S>,
    Query(q): Query<ListQuery>,
    req: axum::extract::Request,
) -> Result<Json<Vec<BatchJob>>, BatchError> {
    let (parts, _) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(BatchError::Unauthenticated)?;
    let jobs = state.list(&tenant, q.status, q.limit.clamp(1, 500)).await?;
    Ok(Json(jobs))
}

/// `DELETE /v1/batch/:id` — cancel.
pub async fn cancel<S: BatchHandlerState>(
    State(state): State<S>,
    Path(id): Path<Uuid>,
    req: axum::extract::Request,
) -> Result<Json<BatchJob>, BatchError> {
    let (parts, _) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(BatchError::Unauthenticated)?;
    let j = state.cancel(&tenant, id).await?;
    Ok(Json(j))
}

#[derive(Debug, Serialize)]
pub struct ResultResponse {
    pub presigned_url: String,
    pub expires_at: DateTime<Utc>,
}

/// `GET /v1/batch/:id/result` — presigned S3 URL for the output.
pub async fn result<S: BatchHandlerState>(
    State(state): State<S>,
    Path(id): Path<Uuid>,
    req: axum::extract::Request,
) -> Result<Json<ResultResponse>, BatchError> {
    let (parts, _) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(BatchError::Unauthenticated)?;
    let url = state.result_url(&tenant, id).await?;
    Ok(Json(ResultResponse {
        presigned_url: url,
        expires_at: Utc::now() + chrono::Duration::hours(1),
    }))
}

/// Build the axum sub-router.
pub fn router<S>(state: S) -> axum::Router
where
    S: BatchHandlerState,
{
    use axum::routing::{delete, get, post};
    axum::Router::new()
        .route("/batch/classify", post(submit::<S>))
        .route("/batch", get(list::<S>))
        .route("/batch/:id", get(get_one::<S>).delete(cancel::<S>))
        .route("/batch/:id/result", get(result::<S>))
        .with_state(state)
}
