// SPDX-License-Identifier: UNLICENSED
//! **Sprint 14 — N22** : `/v1/admin/*` — operator-scope endpoints — § 9.8.

use axum::{extract::State, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};

use crate::auth::layer::{operator_only_or_403, Claims};
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerStatus {
    pub provider: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakersResponse {
    pub breakers: Vec<CircuitBreakerStatus>,
}

/// **N22** : GET /v1/admin/circuit-breakers.
pub async fn circuit_breakers_handler(
    State(_state): State<AppState>,
    Claims(claims): Claims,
) -> Result<impl IntoResponse, ApiError> {
    operator_only_or_403(&claims)?;
    // Sprint 14 stub : we'd query each MultiProviderEmbeddingClient
    // in state ; for now, return an empty list (Sprint 15 wires the
    // real reader).
    Ok(Json(CircuitBreakersResponse { breakers: vec![] }))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftSweepTriggerResponse {
    pub batch_id: uuid::Uuid,
    pub triggered_at: String,
}

/// **N22** : POST /v1/admin/drift-sweep/trigger.
pub async fn drift_sweep_trigger_handler(
    State(_state): State<AppState>,
    Claims(claims): Claims,
) -> Result<impl IntoResponse, ApiError> {
    operator_only_or_403(&claims)?;
    Ok(Json(DriftSweepTriggerResponse {
        batch_id: uuid::Uuid::new_v4(),
        triggered_at: chrono::Utc::now().to_rfc3339(),
    }))
}

/// **N24 + G13** : `GET /v1/admin/compliance-status` —
/// operator-scope live snapshot of the wired compliance stack.
///
/// Returns 200 + JSON body shaped per
/// [`efe_compliance_wiring::ComplianceStatus`] when the wiring
/// runner is configured ; returns 503 otherwise (the gateway is
/// running in N23 experimental-fast mode without a compliance
/// runner).
pub async fn compliance_status_handler(
    State(state): State<AppState>,
    Claims(claims): Claims,
) -> Result<impl IntoResponse, ApiError> {
    operator_only_or_403(&claims)?;
    let runner = state
        .compliance
        .as_ref()
        .ok_or_else(|| ApiError::DependencyUnavailable(
            "compliance runner not configured ; gateway running in N23 experimental_fast mode".into(),
        ))?;
    Ok(Json(runner.compliance_status()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn op_claims() -> crate::auth::GatewayClaims {
        crate::auth::GatewayClaims {
            iss: "x".into(),
            sub: "alice".into(),
            iat: 0,
            exp: 9_999_999_999,
            tenant_id: Uuid::nil(),
            scope: vec!["operator".into()],
            operator_id: Some(Uuid::new_v4()),
        }
    }

    fn tenant_claims() -> crate::auth::GatewayClaims {
        crate::auth::GatewayClaims {
            iss: "x".into(),
            sub: "alice".into(),
            iat: 0,
            exp: 9_999_999_999,
            tenant_id: Uuid::new_v4(),
            scope: vec![],
            operator_id: None,
        }
    }

    #[test]
    fn operator_only_accepts_operator() {
        assert!(operator_only_or_403(&op_claims()).is_ok());
    }

    #[test]
    fn operator_only_rejects_tenant() {
        let err = operator_only_or_403(&tenant_claims()).unwrap_err();
        assert!(matches!(err, ApiError::OperatorScopeRequired));
    }
}
