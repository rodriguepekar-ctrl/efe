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
pub enum SubprocessorError {
    #[error("unauthenticated")]
    Unauthenticated,
    #[error("not found")]
    NotFound,
    #[error("DB error : {0}")]
    Db(String),
    #[error("invalid request : {0}")]
    Invalid(String),
}

impl IntoResponse for SubprocessorError {
    fn into_response(self) -> Response {
        let (status, code, msg) = match &self {
            Self::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                "unauthenticated",
                self.to_string(),
            ),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found", self.to_string()),
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
pub struct Subprocessor {
    pub id: Uuid,
    pub name: String,
    pub purpose: String,
    pub region: String,
    pub data_processed: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract_url: Option<String>,
    pub risk_score: i16,
    pub added_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubprocessorChange {
    pub id: Uuid,
    pub subprocessor_id: Uuid,
    pub change_type: String,
    pub diff: serde_json::Value,
    pub announced_at: DateTime<Utc>,
    pub effective_at: DateTime<Utc>,
    pub ack_required_by: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acknowledged_at: Option<DateTime<Utc>>,
}

#[async_trait]
pub trait SubprocessorHandlerState: Clone + Send + Sync + 'static {
    async fn list_active(&self) -> Result<Vec<Subprocessor>, SubprocessorError>;
    async fn list_changes(
        &self,
        tenant_id: &str,
        limit: i32,
    ) -> Result<Vec<SubprocessorChange>, SubprocessorError>;
    async fn ack_change(
        &self,
        tenant_id: &str,
        event_id: Uuid,
    ) -> Result<SubprocessorChange, SubprocessorError>;

    fn auth_tenant_from_request(&self, parts: &axum::http::request::Parts) -> Option<String> {
        parts
            .headers
            .get("x-tenant-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    }
}

/// `GET /v1/subprocessors/list` — public list (no auth needed).
pub async fn list_public<S: SubprocessorHandlerState>(
    State(state): State<S>,
) -> Result<Json<Vec<Subprocessor>>, SubprocessorError> {
    Ok(Json(state.list_active().await?))
}

#[derive(Debug, Deserialize)]
pub struct ListChangesQuery {
    #[serde(default = "default_limit")]
    pub limit: i32,
}

fn default_limit() -> i32 {
    50
}

/// `GET /v1/subprocessors/changes`
pub async fn list_changes<S: SubprocessorHandlerState>(
    State(state): State<S>,
    Query(q): Query<ListChangesQuery>,
    req: axum::extract::Request,
) -> Result<Json<Vec<SubprocessorChange>>, SubprocessorError> {
    let (parts, _) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(SubprocessorError::Unauthenticated)?;
    let lim = q.limit.clamp(1, 500);
    Ok(Json(state.list_changes(&tenant, lim).await?))
}

/// `POST /v1/subprocessors/ack/:event_id`
pub async fn ack<S: SubprocessorHandlerState>(
    State(state): State<S>,
    Path(event_id): Path<Uuid>,
    req: axum::extract::Request,
) -> Result<Json<SubprocessorChange>, SubprocessorError> {
    let (parts, _) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(SubprocessorError::Unauthenticated)?;
    Ok(Json(state.ack_change(&tenant, event_id).await?))
}

pub fn router<S: SubprocessorHandlerState>(state: S) -> axum::Router {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/subprocessors/list", get(list_public::<S>))
        .route("/subprocessors/changes", get(list_changes::<S>))
        .route("/subprocessors/ack/:event_id", post(ack::<S>))
        .with_state(state)
}
