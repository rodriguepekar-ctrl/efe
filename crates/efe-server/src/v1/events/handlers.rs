//! `GET /v1/events*` handlers : paginated read of the customer's
//! recent event log.

use async_trait::async_trait;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum EventsError {
    #[error("unauthenticated")]
    Unauthenticated,
    #[error("invalid query : {0}")]
    InvalidQuery(String),
    #[error("DB error : {0}")]
    Db(String),
}

impl IntoResponse for EventsError {
    fn into_response(self) -> Response {
        let (status, code, msg) = match &self {
            Self::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                "unauthenticated",
                "missing or invalid bearer".into(),
            ),
            Self::InvalidQuery(_) => (StatusCode::BAD_REQUEST, "invalid_query", self.to_string()),
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
pub struct EventLogEntry {
    pub id: Uuid,
    pub tenant_id: String,
    pub topic: String,
    pub occurred_at: DateTime<Utc>,
    pub schema_version: i16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct EventLogPage {
    pub events: Vec<EventLogEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_page_token: Option<String>,
}

#[async_trait]
pub trait EventsHandlerState: Clone + Send + Sync + 'static {
    /// Page-by-page read.
    async fn list_events(
        &self,
        tenant_id: &str,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
        topic: Option<&str>,
        limit: i32,
        page_token: Option<&str>,
    ) -> Result<EventLogPage, EventsError>;

    fn auth_tenant_from_request(&self, parts: &axum::http::request::Parts) -> Option<String> {
        parts
            .headers
            .get("x-tenant-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    }
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub since: Option<DateTime<Utc>>,
    #[serde(default)]
    pub until: Option<DateTime<Utc>>,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: i32,
    #[serde(default)]
    pub page_token: Option<String>,
}

fn default_limit() -> i32 {
    100
}

/// `GET /v1/events`
pub async fn list<S: EventsHandlerState>(
    State(state): State<S>,
    Query(q): Query<ListQuery>,
    req: axum::extract::Request,
) -> Result<Json<EventLogPage>, EventsError> {
    let (parts, _) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(EventsError::Unauthenticated)?;
    let limit = q.limit.clamp(1, 1000);
    if let (Some(s), Some(u)) = (q.since, q.until) {
        if s > u {
            return Err(EventsError::InvalidQuery("since must be ≤ until".into()));
        }
    }
    let page = state
        .list_events(
            &tenant,
            q.since,
            q.until,
            q.topic.as_deref(),
            limit,
            q.page_token.as_deref(),
        )
        .await?;
    Ok(Json(page))
}

/// Build the router.
pub fn router<S>(state: S) -> axum::Router
where
    S: EventsHandlerState,
{
    use axum::routing::get;
    axum::Router::new()
        .route("/events", get(list::<S>))
        .route("/events/stream", get(super::sse::sse_stream::<S>))
        .with_state(state)
}
