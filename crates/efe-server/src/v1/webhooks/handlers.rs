//! Axum handlers for `/v1/webhooks/*`.
//!
//! State is abstracted via the [`WebhookHandlerState`] trait so tests
//! can swap a fake. The production state lives in `efe-server::main`
//! and is wired through [`crate::v1::webhooks::router`].

use super::delivery::{CircuitState, DeliveryOutcome};
use super::signing;
use super::validation::{validate_webhook_url, WebhookUrlError};
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

/// Trait the state object must implement.
#[async_trait]
pub trait WebhookHandlerState: Clone + Send + Sync + 'static {
    /// Insert a new subscription. Returns the persisted record.
    async fn create_subscription(
        &self,
        tenant_id: &str,
        url: &str,
        secret_hash: &str,
        topics: &[String],
    ) -> Result<SubscriptionRecord, WebhookError>;

    async fn get_subscription(
        &self,
        tenant_id: &str,
        id: Uuid,
    ) -> Result<Option<SubscriptionRecord>, WebhookError>;

    async fn list_subscriptions(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<SubscriptionRecord>, WebhookError>;

    async fn update_subscription(
        &self,
        tenant_id: &str,
        id: Uuid,
        patch: SubscriptionPatch,
    ) -> Result<SubscriptionRecord, WebhookError>;

    async fn delete_subscription(&self, tenant_id: &str, id: Uuid) -> Result<(), WebhookError>;

    async fn list_deliveries(
        &self,
        tenant_id: &str,
        sub_id: Uuid,
        limit: i32,
    ) -> Result<Vec<DeliveryRecord>, WebhookError>;

    /// Enqueue a replay job for events since `since_ts` matching topics.
    async fn enqueue_replay(
        &self,
        tenant_id: &str,
        sub_id: Uuid,
        since: DateTime<Utc>,
        topics: &[String],
    ) -> Result<u64, WebhookError>;

    /// Verify ownership via TXT record check (or mark verified if dev).
    async fn verify_subscription(
        &self,
        tenant_id: &str,
        id: Uuid,
    ) -> Result<SubscriptionRecord, WebhookError>;

    /// Tenant scoping : pulls the JWT-derived tenant_id from the request
    /// (the auth middleware injected it as `x-tenant-id`).
    fn auth_tenant_from_request(&self, parts: &axum::http::request::Parts) -> Option<String> {
        parts
            .headers
            .get("x-tenant-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SubscriptionRecord {
    pub id: Uuid,
    pub tenant_id: String,
    pub url: String,
    pub topics: Vec<String>,
    pub active: bool,
    pub verified: bool,
    pub circuit_state: CircuitState,
    pub consecutive_fails: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_delivery_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct SubscriptionPatch {
    pub url: Option<String>,
    pub topics: Option<Vec<String>>,
    pub active: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeliveryRecord {
    pub id: Uuid,
    pub subscription_id: Uuid,
    pub event_id: String,
    pub topic: String,
    pub attempt_n: u8,
    pub attempted_at: DateTime<Utc>,
    pub response_status: Option<i16>,
    pub response_body_head: Option<String>,
    pub latency_ms: Option<i32>,
    pub outcome: DeliveryOutcome,
    pub next_retry_at: Option<DateTime<Utc>>,
}

#[derive(Debug, thiserror::Error)]
pub enum WebhookError {
    #[error("invalid URL : {0}")]
    InvalidUrl(WebhookUrlError),
    #[error("invalid topic '{0}' (not in PUBLIC_SUBSCRIBABLE_TOPICS)")]
    InvalidTopic(String),
    #[error("subscription not found")]
    NotFound,
    #[error("tenant scope mismatch")]
    Forbidden,
    #[error("unauthenticated")]
    Unauthenticated,
    #[error("DB error : {0}")]
    Db(String),
    #[error("verification failed : {0}")]
    Verification(String),
    #[error("validation failed : {0}")]
    Validation(String),
}

impl IntoResponse for WebhookError {
    fn into_response(self) -> Response {
        let (status, code, msg) = match &self {
            Self::InvalidUrl(_) => (StatusCode::BAD_REQUEST, "invalid_url", self.to_string()),
            Self::InvalidTopic(_) => (StatusCode::BAD_REQUEST, "invalid_topic", self.to_string()),
            Self::NotFound => (
                StatusCode::NOT_FOUND,
                "not_found",
                "subscription not found".into(),
            ),
            Self::Forbidden => (
                StatusCode::FORBIDDEN,
                "forbidden",
                "tenant scope mismatch".into(),
            ),
            Self::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                "unauthenticated",
                self.to_string(),
            ),
            Self::Verification(_) => (
                StatusCode::FAILED_DEPENDENCY,
                "verification_failed",
                self.to_string(),
            ),
            Self::Validation(_) => (StatusCode::BAD_REQUEST, "invalid_input", self.to_string()),
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

#[derive(Debug, Deserialize)]
pub struct CreateRequest {
    pub url: String,
    pub topics: Vec<String>,
    #[serde(default = "default_active")]
    pub active: bool,
}

fn default_active() -> bool {
    true
}

#[derive(Debug, Serialize)]
pub struct CreateResponse {
    /// Subscription record.
    pub subscription: SubscriptionRecord,
    /// One-time view of the generated secret. Customer MUST persist it.
    /// Subsequent reads return only the SHA256 hash.
    pub secret: String,
}

/// `POST /v1/webhooks` — create + return the secret once.
pub async fn create<S: WebhookHandlerState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<CreateResponse>, WebhookError> {
    let (parts, body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(WebhookError::Unauthenticated)?;
    let bytes = axum::body::to_bytes(body, 64 * 1024)
        .await
        .map_err(|e| WebhookError::Validation(format!("body read : {}", e)))?;
    let payload: CreateRequest = serde_json::from_slice(&bytes)
        .map_err(|e| WebhookError::Validation(format!("body parse : {}", e)))?;

    // Validate URL.
    validate_webhook_url(&payload.url).map_err(WebhookError::InvalidUrl)?;

    // Validate topics.
    for t in &payload.topics {
        if !efe_events::topics::is_public_subscribable(t) {
            return Err(WebhookError::InvalidTopic(t.clone()));
        }
    }

    // Generate secret + hash.
    let (secret_raw, secret_hash) = signing::crypto_secret_gen();
    let secret_b64 = hex::encode(&secret_raw);

    let record = state
        .create_subscription(&tenant, &payload.url, &secret_hash, &payload.topics)
        .await?;

    Ok(Json(CreateResponse {
        subscription: record,
        secret: secret_b64,
    }))
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub include_inactive: bool,
}

/// `GET /v1/webhooks`
pub async fn list<S: WebhookHandlerState>(
    State(state): State<S>,
    Query(_q): Query<ListQuery>,
    req: axum::extract::Request,
) -> Result<Json<Vec<SubscriptionRecord>>, WebhookError> {
    let (parts, _) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(WebhookError::Unauthenticated)?;
    let subs = state.list_subscriptions(&tenant).await?;
    Ok(Json(subs))
}

/// `GET /v1/webhooks/:id`
pub async fn get_one<S: WebhookHandlerState>(
    State(state): State<S>,
    Path(id): Path<Uuid>,
    req: axum::extract::Request,
) -> Result<Json<SubscriptionRecord>, WebhookError> {
    let (parts, _) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(WebhookError::Unauthenticated)?;
    match state.get_subscription(&tenant, id).await? {
        Some(r) => Ok(Json(r)),
        None => Err(WebhookError::NotFound),
    }
}

/// `PATCH /v1/webhooks/:id`
pub async fn update<S: WebhookHandlerState>(
    State(state): State<S>,
    Path(id): Path<Uuid>,
    req: axum::extract::Request,
) -> Result<Json<SubscriptionRecord>, WebhookError> {
    let (parts, body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(WebhookError::Unauthenticated)?;
    let bytes = axum::body::to_bytes(body, 64 * 1024)
        .await
        .map_err(|e| WebhookError::Validation(format!("body read : {}", e)))?;
    let patch: SubscriptionPatch = serde_json::from_slice(&bytes)
        .map_err(|e| WebhookError::Validation(format!("body parse : {}", e)))?;
    if let Some(u) = patch.url.as_deref() {
        validate_webhook_url(u).map_err(WebhookError::InvalidUrl)?;
    }
    if let Some(ts) = patch.topics.as_ref() {
        for t in ts {
            if !efe_events::topics::is_public_subscribable(t) {
                return Err(WebhookError::InvalidTopic(t.clone()));
            }
        }
    }
    let rec = state.update_subscription(&tenant, id, patch).await?;
    Ok(Json(rec))
}

/// `DELETE /v1/webhooks/:id`
pub async fn delete_one<S: WebhookHandlerState>(
    State(state): State<S>,
    Path(id): Path<Uuid>,
    req: axum::extract::Request,
) -> Result<StatusCode, WebhookError> {
    let (parts, _) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(WebhookError::Unauthenticated)?;
    state.delete_subscription(&tenant, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct ReplayRequest {
    pub since: DateTime<Utc>,
    #[serde(default)]
    pub topics: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ReplayResponse {
    pub events_enqueued: u64,
}

/// `POST /v1/webhooks/:id/replay`
pub async fn replay<S: WebhookHandlerState>(
    State(state): State<S>,
    Path(id): Path<Uuid>,
    req: axum::extract::Request,
) -> Result<Json<ReplayResponse>, WebhookError> {
    let (parts, body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(WebhookError::Unauthenticated)?;
    let bytes = axum::body::to_bytes(body, 16 * 1024)
        .await
        .map_err(|e| WebhookError::Validation(format!("body read : {}", e)))?;
    let payload: ReplayRequest = serde_json::from_slice(&bytes)
        .map_err(|e| WebhookError::Validation(format!("body parse : {}", e)))?;
    let n = state
        .enqueue_replay(&tenant, id, payload.since, &payload.topics)
        .await?;
    Ok(Json(ReplayResponse { events_enqueued: n }))
}

#[derive(Debug, Deserialize)]
pub struct ListDeliveriesQuery {
    #[serde(default = "default_delivery_limit")]
    pub limit: i32,
}

fn default_delivery_limit() -> i32 {
    100
}

/// `GET /v1/webhooks/:id/deliveries?limit=100`
pub async fn list_deliveries<S: WebhookHandlerState>(
    State(state): State<S>,
    Path(id): Path<Uuid>,
    Query(q): Query<ListDeliveriesQuery>,
    req: axum::extract::Request,
) -> Result<Json<Vec<DeliveryRecord>>, WebhookError> {
    let (parts, _) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(WebhookError::Unauthenticated)?;
    let lim = q.limit.clamp(1, 1000);
    let rows = state.list_deliveries(&tenant, id, lim).await?;
    Ok(Json(rows))
}

/// `POST /v1/webhooks/:id/verify`
pub async fn verify<S: WebhookHandlerState>(
    State(state): State<S>,
    Path(id): Path<Uuid>,
    req: axum::extract::Request,
) -> Result<Json<SubscriptionRecord>, WebhookError> {
    let (parts, _) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(WebhookError::Unauthenticated)?;
    let rec = state.verify_subscription(&tenant, id).await?;
    Ok(Json(rec))
}
