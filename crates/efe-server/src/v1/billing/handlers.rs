//! M31 — `/v1/billing/*` HTTP handlers.

use axum::{
    extract::{Path, State},
    routing::{get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use super::{
    errors::BillingApiError,
    state::{BillingApiState, PlanSwitchBody, PlanView, UpdatePaymentMethodBody},
};

/// Portal session body.
#[derive(Debug, Deserialize)]
pub struct PortalBody {
    /// URL Stripe should redirect back to.
    pub return_url: String,
}

/// Portal session response.
#[derive(Debug, Serialize)]
pub struct PortalResponse {
    /// Hosted Customer Portal URL.
    pub url: String,
}

/// `GET /v1/billing/plan`
pub async fn get_plan<S: BillingApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<PlanView>, BillingApiError> {
    let (parts, _body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(BillingApiError::Unauthenticated)?;
    Ok(Json(state.current_plan(&tenant).await?))
}

/// `PUT /v1/billing/plan`
pub async fn switch_plan<S: BillingApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<PlanView>, BillingApiError> {
    let (parts, body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(BillingApiError::Unauthenticated)?;
    if !state.caller_has_billing_scope(&parts) {
        return Err(BillingApiError::Forbidden);
    }
    let b: PlanSwitchBody = decode_json_body(body).await?;
    Ok(Json(state.switch_plan(&tenant, &b.plan_name).await?))
}

/// `GET /v1/billing/subscriptions`
pub async fn list_subscriptions<S: BillingApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<Vec<efe_billing::StripeSubscription>>, BillingApiError> {
    let (parts, _body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(BillingApiError::Unauthenticated)?;
    Ok(Json(state.list_subscriptions(&tenant).await?))
}

/// `GET /v1/billing/invoices`
pub async fn list_invoices<S: BillingApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<Vec<efe_billing::StripeInvoice>>, BillingApiError> {
    let (parts, _body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(BillingApiError::Unauthenticated)?;
    Ok(Json(state.list_invoices(&tenant).await?))
}

/// `GET /v1/billing/invoices/:id`
pub async fn get_invoice<S: BillingApiState>(
    State(state): State<S>,
    Path(id): Path<String>,
    req: axum::extract::Request,
) -> Result<Json<efe_billing::StripeInvoice>, BillingApiError> {
    let (parts, _body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(BillingApiError::Unauthenticated)?;
    Ok(Json(state.get_invoice(&tenant, &id).await?))
}

/// `PUT /v1/billing/payment_method`
pub async fn update_payment_method<S: BillingApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<axum::http::StatusCode, BillingApiError> {
    let (parts, body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(BillingApiError::Unauthenticated)?;
    if !state.caller_has_billing_scope(&parts) {
        return Err(BillingApiError::Forbidden);
    }
    let b: UpdatePaymentMethodBody = decode_json_body(body).await?;
    state
        .update_payment_method(&tenant, &b.payment_method_id)
        .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// `POST /v1/billing/portal`
pub async fn create_portal<S: BillingApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<PortalResponse>, BillingApiError> {
    let (parts, body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(BillingApiError::Unauthenticated)?;
    let b: PortalBody = decode_json_body(body).await?;
    let url = state.create_portal_session(&tenant, &b.return_url).await?;
    Ok(Json(PortalResponse { url }))
}

/// Build the `/v1/billing/*` router. Note the webhook is mounted
/// separately because it has its own auth model (HMAC) — see
/// [`super::webhook::stripe_webhook`].
pub fn router<S: BillingApiState>(state: S) -> Router {
    Router::new()
        .route("/plan", get(get_plan::<S>).put(switch_plan::<S>))
        .route("/subscriptions", get(list_subscriptions::<S>))
        .route("/invoices", get(list_invoices::<S>))
        .route("/invoices/:id", get(get_invoice::<S>))
        .route("/payment_method", put(update_payment_method::<S>))
        .route("/portal", post(create_portal::<S>))
        .with_state(state)
}

// ─────────────────────────── HELPERS ───────────────────────────

async fn decode_json_body<T: for<'de> Deserialize<'de>>(
    body: axum::body::Body,
) -> Result<T, BillingApiError> {
    let bytes = axum::body::to_bytes(body, 64 * 1024)
        .await
        .map_err(|e| BillingApiError::InvalidBody(format!("read body: {e}")))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| BillingApiError::InvalidBody(format!("decode json: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use efe_billing::{StripeInvoice, StripeInvoiceStatus, StripeSubscription};
    use tower::ServiceExt;

    #[derive(Clone)]
    struct MockState {
        tenant: Option<String>,
        scope: bool,
    }

    #[async_trait]
    impl BillingApiState for MockState {
        fn auth_tenant_from_request(&self, _p: &axum::http::request::Parts) -> Option<String> {
            self.tenant.clone()
        }
        fn caller_has_billing_scope(&self, _p: &axum::http::request::Parts) -> bool {
            self.scope
        }
        async fn current_plan(&self, _t: &str) -> Result<PlanView, BillingApiError> {
            Ok(PlanView {
                plan_name: "starter".into(),
                display_name: "Starter".into(),
                monthly_base_cents: 4900,
            })
        }
        async fn switch_plan(&self, _t: &str, name: &str) -> Result<PlanView, BillingApiError> {
            Ok(PlanView {
                plan_name: name.into(),
                display_name: name.into(),
                monthly_base_cents: 0,
            })
        }
        async fn list_subscriptions(
            &self,
            _t: &str,
        ) -> Result<Vec<StripeSubscription>, BillingApiError> {
            Ok(vec![])
        }
        async fn list_invoices(&self, _t: &str) -> Result<Vec<StripeInvoice>, BillingApiError> {
            Ok(vec![])
        }
        async fn get_invoice(&self, _t: &str, id: &str) -> Result<StripeInvoice, BillingApiError> {
            Ok(StripeInvoice {
                id: id.into(),
                customer: "cus_x".into(),
                status: StripeInvoiceStatus::Paid,
                total: 1000,
                currency: "usd".into(),
                hosted_invoice_url: None,
                period_start: 0,
                period_end: 0,
            })
        }
        async fn update_payment_method(&self, _t: &str, _pm: &str) -> Result<(), BillingApiError> {
            Ok(())
        }
        async fn create_portal_session(
            &self,
            _t: &str,
            _r: &str,
        ) -> Result<String, BillingApiError> {
            Ok("https://billing.stripe.com/p/session/test".into())
        }
    }

    fn state(tenant: Option<&str>, scope: bool) -> MockState {
        MockState {
            tenant: tenant.map(|s| s.into()),
            scope,
        }
    }

    #[tokio::test]
    async fn get_plan_unauthed_401() {
        let app = router(state(None, true));
        let req = Request::builder()
            .method("GET")
            .uri("/plan")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn get_plan_authed_200() {
        let app = router(state(Some("acme"), true));
        let req = Request::builder()
            .method("GET")
            .uri("/plan")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn switch_plan_without_scope_403() {
        let app = router(state(Some("acme"), false));
        let body = serde_json::json!({"plan_name": "pro"});
        let req = Request::builder()
            .method("PUT")
            .uri("/plan")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn portal_returns_url() {
        let app = router(state(Some("acme"), true));
        let body = serde_json::json!({"return_url":"https://efe.ai/dashboard"});
        let req = Request::builder()
            .method("POST")
            .uri("/portal")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn payment_method_update_returns_204() {
        let app = router(state(Some("acme"), true));
        let body = serde_json::json!({"payment_method_id":"pm_test"});
        let req = Request::builder()
            .method("PUT")
            .uri("/payment_method")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }
}
