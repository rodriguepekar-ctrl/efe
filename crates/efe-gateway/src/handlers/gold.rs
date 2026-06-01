// SPDX-License-Identifier: UNLICENSED
//! **Sprint 14 — N22** : N19 gold endpoints — § 9.5.
//!
//! 4 endpoints :
//! - `POST /v1/gold-sets` register a draft
//! - `POST /v1/gold-sets/{id}/approve` (operator scope)
//! - `POST /v1/gold-runs` run a gate
//! - `GET  /v1/gold-runs/{id}` lookup a run

use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Json,
};
use efe_gold_test_gate::api::{ApproveGoldSetRequest, RegisterGoldSetRequest, RunRequest};
use efe_gold_test_gate::GoldApiError;
use efe_nano_sdk::{GoldRunId, GoldSetId};

use crate::auth::layer::{operator_only_or_403, Claims};
use crate::error::ApiError;
use crate::state::AppState;

fn map_gold_err(e: GoldApiError) -> ApiError {
    match e {
        GoldApiError::NotFound => ApiError::NotFound("gold resource".into()),
        GoldApiError::BadRequest { reason } => ApiError::BadRequest(reason),
        GoldApiError::Conflict { reason } => ApiError::Conflict(reason),
        GoldApiError::Forbidden { reason } => ApiError::BadRequest(reason),
        GoldApiError::Backend { message } => ApiError::DependencyUnavailable(message),
    }
}

// ─────────────────────────────────────────────────────────────────────
//  POST /v1/gold-sets — register
// ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct RegisterGoldSetResponse {
    pub gold_set_id: GoldSetId,
}

pub async fn register_gold_set_handler(
    State(state): State<AppState>,
    Claims(_claims): Claims,
    Json(req): Json<RegisterGoldSetRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let id = state
        .gold_api
        .register_gold_set(req)
        .await
        .map_err(map_gold_err)?;
    Ok(Json(RegisterGoldSetResponse { gold_set_id: id }))
}

// ─────────────────────────────────────────────────────────────────────
//  POST /v1/gold-sets/{id}/approve
// ─────────────────────────────────────────────────────────────────────

pub async fn approve_gold_set_handler(
    State(state): State<AppState>,
    Claims(claims): Claims,
    Path(gold_set_id): Path<uuid::Uuid>,
    Json(req): Json<ApproveGoldSetRequest>,
) -> Result<impl IntoResponse, ApiError> {
    operator_only_or_403(&claims)?;
    let set = state
        .gold_api
        .approve_gold_set(GoldSetId::from_uuid(gold_set_id), req)
        .await
        .map_err(map_gold_err)?;
    Ok(Json(set))
}

// ─────────────────────────────────────────────────────────────────────
//  POST /v1/gold-runs — run a gate
// ─────────────────────────────────────────────────────────────────────

pub async fn run_gold_gate_handler(
    State(state): State<AppState>,
    Claims(_claims): Claims,
    Json(req): Json<RunRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let report = state.gold_api.run_gate(req).await.map_err(map_gold_err)?;
    Ok(Json(report))
}

// ─────────────────────────────────────────────────────────────────────
//  GET /v1/gold-runs/{id}
// ─────────────────────────────────────────────────────────────────────

pub async fn lookup_gold_run_handler(
    State(state): State<AppState>,
    Claims(_claims): Claims,
    Path(gold_run_id): Path<uuid::Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let row = state
        .gold_api
        .lookup_gold_run(GoldRunId::from_uuid(gold_run_id))
        .await
        .map_err(map_gold_err)?
        .ok_or_else(|| ApiError::NotFound("gold run".into()))?;
    Ok(Json(row))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_not_found_404() {
        assert!(matches!(
            map_gold_err(GoldApiError::NotFound),
            ApiError::NotFound(_)
        ));
    }

    #[test]
    fn map_bad_request_400() {
        assert!(matches!(
            map_gold_err(GoldApiError::BadRequest {
                reason: "x".into()
            }),
            ApiError::BadRequest(_)
        ));
    }

    #[test]
    fn map_conflict_409() {
        assert!(matches!(
            map_gold_err(GoldApiError::Conflict {
                reason: "x".into()
            }),
            ApiError::Conflict(_)
        ));
    }

    #[test]
    fn map_backend_503() {
        assert!(matches!(
            map_gold_err(GoldApiError::Backend {
                message: "x".into()
            }),
            ApiError::DependencyUnavailable(_)
        ));
    }
}
