//! M30 — `/v1/region/*` HTTP handlers.

use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use efe_region::{DrillConfig, FailoverRequest};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::{errors::RegionApiError, state::RegionApiState};

// ─────────────────────────── REQUEST / RESPONSE BODIES ───────────────────────────

#[derive(Debug, Deserialize)]
pub struct FailoverBody {
    pub target_region: String,
    pub reason: String,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Serialize)]
pub struct FailoverResponse {
    pub status: &'static str,
    pub previous_active: String,
    pub new_active: String,
    pub duration_ms: u64,
}

#[derive(Debug, Deserialize)]
pub struct PromoteBody {
    pub reason: String,
}

#[derive(Debug, Deserialize)]
pub struct DrillBody {
    pub initial_active: String,
    pub failover_target: String,
    #[serde(default)]
    pub force: bool,
    #[serde(default = "default_drill_timeout")]
    pub total_timeout_secs: u64,
}

fn default_drill_timeout() -> u64 {
    300
}

// ─────────────────────────── HANDLERS ───────────────────────────

/// `GET /v1/region/health` — invoked by peer regions.
pub async fn health<S: RegionApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<efe_region::PeerHealth>, RegionApiError> {
    let (parts, _body) = req.into_parts();
    if !state.auth_peer_from_request(&parts) {
        return Err(RegionApiError::Unauthenticated);
    }
    let snap = state.current_health().await?;
    Ok(Json(snap))
}

/// `GET /v1/region/current` — operator endpoint.
pub async fn current<S: RegionApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<serde_json::Value>, RegionApiError> {
    let (parts, _body) = req.into_parts();
    if state.auth_operator_from_request(&parts).is_none() {
        return Err(RegionApiError::Unauthenticated);
    }
    let v = state.current_identity().await?;
    Ok(Json(v))
}

/// `POST /v1/region/failover` — operator endpoint.
pub async fn failover<S: RegionApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<FailoverResponse>, RegionApiError> {
    let (parts, body) = req.into_parts();
    let operator = state
        .auth_operator_from_request(&parts)
        .ok_or(RegionApiError::Unauthenticated)?;
    let body: FailoverBody = decode_json_body(body).await?;
    let req = FailoverRequest {
        target_region: body.target_region,
        reason: body.reason,
        force: body.force,
        operator,
    };
    let res = state.run_failover(&req).await?;
    Ok(Json(FailoverResponse {
        status: res.status,
        previous_active: res.previous_active,
        new_active: res.new_active,
        duration_ms: res.duration_ms,
    }))
}

/// `POST /v1/region/promote` — invoked by a peer's orchestrator.
pub async fn promote<S: RegionApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<serde_json::Value>, RegionApiError> {
    let (parts, body) = req.into_parts();
    let operator = state
        .auth_operator_from_request(&parts)
        .ok_or(RegionApiError::Unauthenticated)?;
    let body: PromoteBody = decode_json_body(body).await?;
    let v = state.run_promote(&operator, &body.reason).await?;
    Ok(Json(v))
}

/// `POST /v1/region/drill` — operator endpoint.
pub async fn drill<S: RegionApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<efe_region::DrillReport>, RegionApiError> {
    let (parts, body) = req.into_parts();
    let operator = state
        .auth_operator_from_request(&parts)
        .ok_or(RegionApiError::Unauthenticated)?;
    let body: DrillBody = decode_json_body(body).await?;
    let cfg = DrillConfig {
        initial_active: body.initial_active,
        failover_target: body.failover_target,
        operator,
        total_timeout: Duration::from_secs(body.total_timeout_secs),
        force: body.force,
    };
    let report = state.run_drill(&cfg).await?;
    Ok(Json(report))
}

// ─────────────────────────── ROUTER ───────────────────────────

/// Build the `/v1/region/*` sub-router.
pub fn router<S: RegionApiState>(state: S) -> Router {
    Router::new()
        .route("/health", get(health::<S>))
        .route("/current", get(current::<S>))
        .route("/failover", post(failover::<S>))
        .route("/promote", post(promote::<S>))
        .route("/drill", post(drill::<S>))
        .with_state(state)
}

// ─────────────────────────── HELPERS ───────────────────────────

async fn decode_json_body<T: for<'de> Deserialize<'de>>(
    body: axum::body::Body,
) -> Result<T, RegionApiError> {
    let bytes = axum::body::to_bytes(body, 64 * 1024)
        .await
        .map_err(|e| RegionApiError::InvalidBody(format!("read body: {e}")))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| RegionApiError::InvalidBody(format!("decode json: {e}")))
}

#[cfg(test)]
mod tests {
    //! M30 audit fix : smoke tests covering the `/v1/region/*`
    //! router wiring. Pinpoint refactor regressions early ;
    //! cross-check the auth + body-decode happy paths.

    use super::*;
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use efe_region::{DrillReport, DrillVerdict, FailoverResult, PeerHealth};
    use tower::ServiceExt;

    #[derive(Clone)]
    struct MockRegionState {
        peer_ok: bool,
        operator: Option<String>,
    }

    #[async_trait]
    impl RegionApiState for MockRegionState {
        fn auth_peer_from_request(&self, _p: &axum::http::request::Parts) -> bool {
            self.peer_ok
        }
        fn auth_operator_from_request(&self, _p: &axum::http::request::Parts) -> Option<String> {
            self.operator.clone()
        }
        async fn current_health(&self) -> Result<PeerHealth, RegionApiError> {
            Ok(PeerHealth {
                region: "eu-west-1".into(),
                up: true,
                rtt_ms: 1,
                replication_lag_secs: Some(2.0),
                ready_for_promote: false,
            })
        }
        async fn current_identity(&self) -> Result<serde_json::Value, RegionApiError> {
            Ok(serde_json::json!({"region": "eu-west-1", "role": "active"}))
        }
        async fn run_failover(
            &self,
            req: &efe_region::FailoverRequest,
        ) -> Result<FailoverResult, RegionApiError> {
            Ok(FailoverResult {
                status: "succeeded",
                previous_active: "eu-west-1".into(),
                new_active: req.target_region.clone(),
                duration_ms: 100,
                target_health_snapshot: None,
            })
        }
        async fn run_promote(
            &self,
            _op: &str,
            _reason: &str,
        ) -> Result<serde_json::Value, RegionApiError> {
            Ok(serde_json::json!({"promoted": true}))
        }
        async fn run_drill(
            &self,
            _cfg: &efe_region::DrillConfig,
        ) -> Result<DrillReport, RegionApiError> {
            Ok(DrillReport {
                drill_id: "test".into(),
                verdict: DrillVerdict::Pass,
                steps: vec![],
                total_duration_ms: 0,
            })
        }
    }

    fn state(operator: Option<&str>, peer_ok: bool) -> MockRegionState {
        MockRegionState {
            peer_ok,
            operator: operator.map(|s| s.to_string()),
        }
    }

    #[tokio::test]
    async fn health_unauthenticated_peer_is_401() {
        let app = router(state(None, false));
        let req = Request::builder()
            .method("GET")
            .uri("/health")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn health_authenticated_peer_is_200() {
        let app = router(state(None, true));
        let req = Request::builder()
            .method("GET")
            .uri("/health")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn failover_unauthenticated_operator_is_401() {
        let app = router(state(None, true));
        let body = serde_json::json!({
            "target_region": "eu-west-3",
            "reason": "test",
            "force": false
        });
        let req = Request::builder()
            .method("POST")
            .uri("/failover")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn failover_authenticated_operator_is_200() {
        let app = router(state(Some("alice"), true));
        let body = serde_json::json!({
            "target_region": "eu-west-3",
            "reason": "test",
            "force": false
        });
        let req = Request::builder()
            .method("POST")
            .uri("/failover")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn drill_authenticated_returns_pass_report() {
        let app = router(state(Some("alice"), true));
        let body = serde_json::json!({
            "initial_active": "eu-west-1",
            "failover_target": "eu-west-3",
            "force": false,
            "total_timeout_secs": 60
        });
        let req = Request::builder()
            .method("POST")
            .uri("/drill")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
