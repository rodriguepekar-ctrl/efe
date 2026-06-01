//! M31 — `/v1/usage/*` HTTP handlers.

use axum::{
    extract::{Path, State},
    routing::{get, put},
    Json, Router,
};
use efe_billing::BudgetAlert;
use serde::Deserialize;

use super::{errors::UsageApiError, state::UsageApiState};

/// `GET /v1/usage/now`
pub async fn usage_now<S: UsageApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<super::state::UsageNow>, UsageApiError> {
    let (parts, _body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(UsageApiError::Unauthenticated)?;
    let snap = state.usage_now(&tenant).await?;
    Ok(Json(snap))
}

#[derive(Debug, Deserialize)]
struct DailyQuery {
    #[serde(default)]
    month_start: Option<String>,
    #[serde(default)]
    month_end: Option<String>,
}

/// `GET /v1/usage/daily?month_start=&month_end=`
pub async fn usage_daily<S: UsageApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<Vec<efe_metering::RollupRow>>, UsageApiError> {
    let (parts, _body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(UsageApiError::Unauthenticated)?;
    let q: DailyQuery = serde_urlencoded::from_str(parts.uri.query().unwrap_or(""))
        .map_err(|e| UsageApiError::InvalidQuery(format!("urldecode: {e}")))?;
    let month_start = q.month_start.unwrap_or_else(default_month_start);
    let month_end = q.month_end.unwrap_or_else(default_month_end);
    if month_start > month_end {
        return Err(UsageApiError::InvalidQuery(
            "month_start > month_end".into(),
        ));
    }
    let rows = state.usage_daily(&tenant, &month_start, &month_end).await?;
    Ok(Json(rows))
}

/// `GET /v1/usage/projection`
pub async fn projection<S: UsageApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<super::state::UsageProjection>, UsageApiError> {
    let (parts, _body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(UsageApiError::Unauthenticated)?;
    let p = state.projection(&tenant).await?;
    Ok(Json(p))
}

/// `GET /v1/budgets`
pub async fn list_budgets<S: UsageApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<Vec<BudgetAlert>>, UsageApiError> {
    let (parts, _body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(UsageApiError::Unauthenticated)?;
    let list = state.list_budgets(&tenant).await?;
    Ok(Json(list))
}

/// `POST /v1/budgets`
pub async fn create_budget<S: UsageApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<BudgetAlert>, UsageApiError> {
    let (parts, body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(UsageApiError::Unauthenticated)?;
    let alert: BudgetAlert = decode_json_body(body).await?;
    let saved = state.create_budget(&tenant, alert).await?;
    Ok(Json(saved))
}

/// `PUT /v1/budgets/:id`
pub async fn update_budget<S: UsageApiState>(
    State(state): State<S>,
    Path(id): Path<String>,
    req: axum::extract::Request,
) -> Result<Json<BudgetAlert>, UsageApiError> {
    let (parts, body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(UsageApiError::Unauthenticated)?;
    let alert: BudgetAlert = decode_json_body(body).await?;
    let saved = state.update_budget(&tenant, &id, alert).await?;
    Ok(Json(saved))
}

/// `DELETE /v1/budgets/:id`
pub async fn delete_budget<S: UsageApiState>(
    State(state): State<S>,
    Path(id): Path<String>,
    req: axum::extract::Request,
) -> Result<axum::http::StatusCode, UsageApiError> {
    let (parts, _body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(UsageApiError::Unauthenticated)?;
    state.delete_budget(&tenant, &id).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Build the `/v1/usage/*` + `/v1/budgets/*` router.
pub fn router<S: UsageApiState>(state: S) -> Router {
    Router::new()
        .route("/usage/now", get(usage_now::<S>))
        .route("/usage/daily", get(usage_daily::<S>))
        .route("/usage/projection", get(projection::<S>))
        .route("/budgets", get(list_budgets::<S>).post(create_budget::<S>))
        .route(
            "/budgets/:id",
            put(update_budget::<S>).delete(delete_budget::<S>),
        )
        .with_state(state)
}

// ─────────────────────────── HELPERS ───────────────────────────

async fn decode_json_body<T: for<'de> Deserialize<'de>>(
    body: axum::body::Body,
) -> Result<T, UsageApiError> {
    let bytes = axum::body::to_bytes(body, 64 * 1024)
        .await
        .map_err(|e| UsageApiError::InvalidBody(format!("read body: {e}")))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| UsageApiError::InvalidBody(format!("decode json: {e}")))
}

fn default_month_start() -> String {
    let now = chrono::Utc::now();
    format!(
        "{:04}-{:02}-01",
        chrono::Datelike::year(&now),
        chrono::Datelike::month(&now)
    )
}

fn default_month_end() -> String {
    let now = chrono::Utc::now();
    format!(
        "{:04}-{:02}-{:02}",
        chrono::Datelike::year(&now),
        chrono::Datelike::month(&now),
        chrono::Datelike::day(&now)
    )
}

#[cfg(test)]
mod tests {
    //! M31 smoke tests for the router wiring.

    use super::*;
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use efe_billing::{BudgetAlert, BudgetSeverity, NotificationChannel};
    use efe_metering::{Meter, RollupRow};
    use std::collections::HashMap;
    use tower::ServiceExt;

    #[derive(Clone)]
    struct MockState {
        tenant: Option<String>,
    }

    #[async_trait]
    impl UsageApiState for MockState {
        fn auth_tenant_from_request(&self, _p: &axum::http::request::Parts) -> Option<String> {
            self.tenant.clone()
        }
        async fn usage_now(&self, t: &str) -> Result<super::super::state::UsageNow, UsageApiError> {
            let mut by_meter = HashMap::new();
            by_meter.insert(Meter::ClassifyRequests, 42);
            Ok(super::super::state::UsageNow {
                tenant_id: t.into(),
                since: "2026-05-11T11:00:00Z".into(),
                until: "2026-05-11T12:00:00Z".into(),
                by_meter,
            })
        }
        async fn usage_daily(
            &self,
            _t: &str,
            _ms: &str,
            _me: &str,
        ) -> Result<Vec<RollupRow>, UsageApiError> {
            Ok(vec![])
        }
        async fn projection(
            &self,
            t: &str,
        ) -> Result<super::super::state::UsageProjection, UsageApiError> {
            Ok(super::super::state::UsageProjection {
                tenant_id: t.into(),
                days_elapsed: 11,
                days_remaining: 20,
                current_cost_cents: 4900,
                projected_month_end_cents: 14000,
                plan_name: "starter".into(),
            })
        }
        async fn list_budgets(&self, _t: &str) -> Result<Vec<BudgetAlert>, UsageApiError> {
            Ok(vec![])
        }
        async fn create_budget(
            &self,
            _t: &str,
            a: BudgetAlert,
        ) -> Result<BudgetAlert, UsageApiError> {
            Ok(a)
        }
        async fn update_budget(
            &self,
            _t: &str,
            _id: &str,
            a: BudgetAlert,
        ) -> Result<BudgetAlert, UsageApiError> {
            Ok(a)
        }
        async fn delete_budget(&self, _t: &str, _id: &str) -> Result<(), UsageApiError> {
            Ok(())
        }
    }

    fn state(tenant: Option<&str>) -> MockState {
        MockState {
            tenant: tenant.map(|s| s.into()),
        }
    }

    #[tokio::test]
    async fn usage_now_unauthed_is_401() {
        let app = router(state(None));
        let req = Request::builder()
            .method("GET")
            .uri("/usage/now")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn usage_now_authed_is_200() {
        let app = router(state(Some("acme")));
        let req = Request::builder()
            .method("GET")
            .uri("/usage/now")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn projection_authed_is_200() {
        let app = router(state(Some("acme")));
        let req = Request::builder()
            .method("GET")
            .uri("/usage/projection")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn budgets_list_authed_is_200() {
        let app = router(state(Some("acme")));
        let req = Request::builder()
            .method("GET")
            .uri("/budgets")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn budget_create_validates_body() {
        let app = router(state(Some("acme")));
        let body = serde_json::json!({
            "id": "abc",
            "tenant_id": "acme",
            "threshold_cents": 5000,
            "severity": "soft",
            "channels": [{"kind":"email","to":"o@e.com"}]
        });
        let req = Request::builder()
            .method("POST")
            .uri("/budgets")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn budget_delete_returns_204() {
        let app = router(state(Some("acme")));
        let req = Request::builder()
            .method("DELETE")
            .uri("/budgets/abc")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    // Silence the unused-channel import.
    fn _silence() {
        let _ = NotificationChannel::Email { to: "x".into() };
        let _ = BudgetSeverity::Soft;
    }
}
