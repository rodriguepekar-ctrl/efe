// SPDX-License-Identifier: UNLICENSED
//! **Sprint 14 — N22** : N18 publisher endpoints — § 9.4.
//!
//! 5 endpoints :
//! - `POST /v1/publishers` register
//! - `GET  /v1/publishers/{id}` lookup
//! - `POST /v1/publishers/{id}/rotate-key`
//! - `POST /v1/publishers/{id}/revoke`
//! - `POST /v1/publishers/{id}/promote` (tier promote)

use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Json,
};
use efe_nano_sdk::PublisherId;
use efe_publisher_identity::{
    PromoteRequest, PublisherApiError, RegisterRequest, RegisterResponse,
    RevokeRequest, RotateKeyRequest, RotateKeyResponse,
};

use crate::auth::layer::{operator_only_or_403, Claims};
use crate::error::ApiError;
use crate::state::AppState;

// ─────────────────────────────────────────────────────────────────────
//  Error mapping
// ─────────────────────────────────────────────────────────────────────

fn map_publisher_err(e: PublisherApiError) -> ApiError {
    match e {
        PublisherApiError::NotFound { .. } => ApiError::NotFound("publisher".into()),
        PublisherApiError::Conflict { name, tier } => {
            ApiError::Conflict(format!("display name {name:?} already used at tier {tier:?}"))
        }
        PublisherApiError::BadRequest { reason } => ApiError::BadRequest(reason),
        PublisherApiError::KycRefused { verdict } => ApiError::Unprocessable(verdict),
        PublisherApiError::Forbidden { reason } => ApiError::BadRequest(reason),
        PublisherApiError::Gone => ApiError::NotFound("publisher revoked".into()),
        PublisherApiError::Backend { message } => ApiError::DependencyUnavailable(message),
    }
}

// ─────────────────────────────────────────────────────────────────────
//  POST /v1/publishers — register
// ─────────────────────────────────────────────────────────────────────

pub async fn register_publisher_handler(
    State(state): State<AppState>,
    Claims(claims): Claims,
    Json(req): Json<RegisterRequest>,
) -> Result<impl IntoResponse, ApiError> {
    // Registering requires operator scope (KYC + sanctions check is
    // sensitive).  Sprint 15 may add publisher-self-onboarding via
    // OAuth.
    operator_only_or_403(&claims)?;
    let resp: RegisterResponse = state
        .publisher_api
        .register(req)
        .await
        .map_err(map_publisher_err)?;
    Ok(Json(resp))
}

// ─────────────────────────────────────────────────────────────────────
//  GET /v1/publishers/{id}
// ─────────────────────────────────────────────────────────────────────

pub async fn lookup_publisher_handler(
    State(state): State<AppState>,
    Claims(_claims): Claims,
    Path(publisher_id): Path<uuid::Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let identity = state
        .publisher_api
        .lookup(PublisherId::from_uuid(publisher_id))
        .await
        .map_err(map_publisher_err)?;
    Ok(Json(identity))
}

// ─────────────────────────────────────────────────────────────────────
//  POST /v1/publishers/{id}/rotate-key
// ─────────────────────────────────────────────────────────────────────

pub async fn rotate_key_handler(
    State(state): State<AppState>,
    Claims(_claims): Claims,
    Path(publisher_id): Path<uuid::Uuid>,
    Json(req): Json<RotateKeyRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let pid = PublisherId::from_uuid(publisher_id);
    let resp: RotateKeyResponse = state
        .publisher_api
        .rotate_key(pid, req)
        .await
        .map_err(map_publisher_err)?;
    // **N24 + G12** : rotate-key flips `active_key` ; the cached
    // identity now holds a stale `active_key.public_key_pem`.
    // Evict so the next dispatch loads the rotated key.
    if let Some(runner) = state.compliance.as_ref() {
        runner.evict_publisher(pid);
    }
    Ok(Json(resp))
}

// ─────────────────────────────────────────────────────────────────────
//  POST /v1/publishers/{id}/revoke
// ─────────────────────────────────────────────────────────────────────

pub async fn revoke_publisher_handler(
    State(state): State<AppState>,
    Claims(claims): Claims,
    Path(publisher_id): Path<uuid::Uuid>,
    Json(req): Json<RevokeRequest>,
) -> Result<impl IntoResponse, ApiError> {
    operator_only_or_403(&claims)?;
    let pid = PublisherId::from_uuid(publisher_id);
    let cascade = state
        .publisher_api
        .revoke(pid, req)
        .await
        .map_err(map_publisher_err)?;
    // **N24 + G11 + G12 + Workflow E** : evict the wired
    // compliance cache so the next swarm dispatch picks up the
    // new status (Revoked) and fails closed via the
    // PublisherStatusGate.  Without this the cache could hold a
    // stale Active identity for up to 60 s (the cache TTL).
    if let Some(runner) = state.compliance.as_ref() {
        let evicted = runner.evict_publisher(pid);
        tracing::info!(
            publisher_id = %pid,
            cache_evicted = evicted,
            "compliance cache invalidated on publisher revoke"
        );
    }
    Ok(Json(cascade))
}

// ─────────────────────────────────────────────────────────────────────
//  POST /v1/publishers/{id}/promote
// ─────────────────────────────────────────────────────────────────────

pub async fn promote_publisher_handler(
    State(state): State<AppState>,
    Claims(claims): Claims,
    Path(publisher_id): Path<uuid::Uuid>,
    Json(req): Json<PromoteRequest>,
) -> Result<impl IntoResponse, ApiError> {
    operator_only_or_403(&claims)?;
    let pid = PublisherId::from_uuid(publisher_id);
    state
        .publisher_api
        .promote(pid, req)
        .await
        .map_err(map_publisher_err)?;
    // **N24 + G12** : promote flips `kyc_tier` ; cached identity
    // now stale.  Evict so the canonical body recomputation uses
    // the new tier.
    if let Some(runner) = state.compliance.as_ref() {
        runner.evict_publisher(pid);
    }
    Ok(Json(serde_json::json!({"ok": true})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_not_found_404() {
        let e = PublisherApiError::NotFound {
            publisher_id: PublisherId::new(),
        };
        let mapped = map_publisher_err(e);
        assert!(matches!(mapped, ApiError::NotFound(_)));
    }

    #[test]
    fn map_kyc_refused_422() {
        let e = PublisherApiError::KycRefused {
            verdict: "failed: rejected".into(),
        };
        let mapped = map_publisher_err(e);
        assert!(matches!(mapped, ApiError::Unprocessable(_)));
    }

    #[test]
    fn map_backend_503() {
        let e = PublisherApiError::Backend {
            message: "pg down".into(),
        };
        assert!(matches!(
            map_publisher_err(e),
            ApiError::DependencyUnavailable(_)
        ));
    }
}
