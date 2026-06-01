//! `DELETE /v1/tenants/:slug` + cancel + status.
//!
//! Two-step flow :
//! 1. First DELETE call → `202 Accepted` + email with MFA token.
//! 2. DELETE with `?mfa=<token>` → enters 7-day grace.
//!
//! During grace, the tenant remains operational but every write is
//! locked at the application layer (`tenant_feature.deletion_state =
//! 'grace'`). After grace expires, the cascade worker runs.

use super::state::ErasureState;
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
pub enum ErasureError {
    #[error("unauthenticated")]
    Unauthenticated,
    #[error("forbidden")]
    Forbidden,
    #[error("MFA verification failed")]
    MfaFailed,
    #[error("erasure not pending or already terminal")]
    NotInValidState,
    #[error("erasure not found")]
    NotFound,
    #[error("invalid request : {0}")]
    Invalid(String),
    #[error("DB error : {0}")]
    Db(String),
}

impl IntoResponse for ErasureError {
    fn into_response(self) -> Response {
        let (status, code, msg) = match &self {
            Self::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                "unauthenticated",
                self.to_string(),
            ),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden", self.to_string()),
            Self::MfaFailed => (StatusCode::UNAUTHORIZED, "mfa_failed", self.to_string()),
            Self::NotInValidState => (StatusCode::CONFLICT, "invalid_state", self.to_string()),
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
pub struct ErasureRecord {
    pub id: Uuid,
    pub tenant_id: String,
    pub state: ErasureState,
    pub requested_by: String,
    pub requested_at: DateTime<Utc>,
    pub grace_until: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_audit_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DeleteQuery {
    /// One-time MFA token from the verification email.
    pub mfa: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeleteAcceptedResponse {
    pub erasure_id: Uuid,
    pub state: ErasureState,
    pub message: String,
}

#[async_trait]
pub trait ErasureHandlerState: Clone + Send + Sync + 'static {
    /// Initiate a fresh erasure request : send MFA email + insert
    /// `erasure_request` row.
    async fn initiate_erasure(
        &self,
        tenant_id: &str,
        requested_by: &str,
    ) -> Result<ErasureRecord, ErasureError>;

    /// Verify the MFA token and transition mfa_pending → grace.
    /// `grace_until` = now + 7d.
    async fn verify_mfa(
        &self,
        tenant_id: &str,
        mfa_token: &str,
    ) -> Result<ErasureRecord, ErasureError>;

    /// Cancel a grace-state erasure.
    async fn cancel_erasure(&self, tenant_id: &str) -> Result<ErasureRecord, ErasureError>;

    /// Status lookup.
    async fn get_erasure(&self, tenant_id: &str) -> Result<Option<ErasureRecord>, ErasureError>;

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

/// `DELETE /v1/tenants/:slug?mfa=<token?>` — initiate OR confirm.
pub async fn delete_tenant<S: ErasureHandlerState>(
    State(state): State<S>,
    Path(slug): Path<String>,
    Query(q): Query<DeleteQuery>,
    req: axum::extract::Request,
) -> Result<(StatusCode, Json<DeleteAcceptedResponse>), ErasureError> {
    let (parts, _) = req.into_parts();
    let caller_tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(ErasureError::Unauthenticated)?;
    if caller_tenant != slug {
        return Err(ErasureError::Forbidden);
    }
    let sub = state
        .auth_subject_from_request(&parts)
        .unwrap_or_else(|| "anonymous".to_string());

    if let Some(token) = q.mfa.as_deref() {
        let rec = state.verify_mfa(&slug, token).await?;
        return Ok((
            StatusCode::OK,
            Json(DeleteAcceptedResponse {
                erasure_id: rec.id,
                state: rec.state,
                message:
                    "MFA verified ; tenant now in 7-day grace. Cancel within window if needed."
                        .into(),
            }),
        ));
    }

    let rec = state.initiate_erasure(&slug, &sub).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(DeleteAcceptedResponse {
            erasure_id: rec.id,
            state: rec.state,
            message: "MFA verification email sent to DPO. Confirm with ?mfa=<token>.".into(),
        }),
    ))
}

/// `POST /v1/tenants/:slug/cancel-deletion`
pub async fn cancel_deletion<S: ErasureHandlerState>(
    State(state): State<S>,
    Path(slug): Path<String>,
    req: axum::extract::Request,
) -> Result<Json<ErasureRecord>, ErasureError> {
    let (parts, _) = req.into_parts();
    let caller_tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(ErasureError::Unauthenticated)?;
    if caller_tenant != slug {
        return Err(ErasureError::Forbidden);
    }
    let rec = state.cancel_erasure(&slug).await?;
    Ok(Json(rec))
}

/// `GET /v1/tenants/:slug/deletion-status`
pub async fn get_status<S: ErasureHandlerState>(
    State(state): State<S>,
    Path(slug): Path<String>,
    req: axum::extract::Request,
) -> Result<Json<ErasureRecord>, ErasureError> {
    let (parts, _) = req.into_parts();
    let caller_tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(ErasureError::Unauthenticated)?;
    if caller_tenant != slug {
        return Err(ErasureError::Forbidden);
    }
    state
        .get_erasure(&slug)
        .await?
        .map(Json)
        .ok_or(ErasureError::NotFound)
}

pub fn router<S: ErasureHandlerState>(state: S) -> axum::Router {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/tenants/:slug", axum::routing::delete(delete_tenant::<S>))
        .route("/tenants/:slug/cancel-deletion", post(cancel_deletion::<S>))
        .route("/tenants/:slug/deletion-status", get(get_status::<S>))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_machine_consistency() {
        assert!(ErasureState::MfaPending.can_transition_to(ErasureState::Grace));
        assert!(ErasureState::Grace.can_transition_to(ErasureState::InProgress));
        assert!(!ErasureState::Done.can_transition_to(ErasureState::Grace));
    }
}
