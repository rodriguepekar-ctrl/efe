//! M31 — Stripe webhook receiver.
//!
//! Mounted at `POST /v1/billing/webhook` outside the bearer-auth
//! gate ; authentication is via HMAC signature in the
//! `Stripe-Signature` header. The handler :
//!
//! 1. Reads the **raw** body (signature is over raw bytes — must
//!    NOT round-trip through serde).
//! 2. Verifies the signature via `efe_billing::verify_stripe_signature`.
//! 3. Hands the decoded event to the `WebhookHandler` trait.
//! 4. Returns 200 if the handler succeeded, 4xx for signature /
//!    payload errors, 5xx for downstream errors.
//!
//! **Idempotency** : every event carries a `evt_...` id. The
//! handler must dedupe on this id (typically via a unique index in
//! `stripe_webhook_event`).

use async_trait::async_trait;
use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Router,
};
use efe_billing::{verify_stripe_signature, StripeEvent, WebhookConfig};
use std::sync::Arc;
use tracing::warn;

/// Handler that processes a verified Stripe event.
///
/// **M31 audit fix** : the trait now exposes an explicit
/// `is_event_processed` hook so the receiver can dedup BEFORE
/// invoking the side-effecting `handle`. Stripe sends the same
/// `evt_…` id on retry, so without dedup a webhook storm doubles
/// our state mutations.
#[async_trait]
pub trait WebhookHandler: Send + Sync {
    /// `true` iff the event has already been processed (mirror in
    /// `stripe_webhook_event` table). Default impl returns `false`
    /// so naïve impls remain functional ; concrete impls in
    /// `efe-server` query PG.
    async fn is_event_processed(&self, _stripe_event_id: &str) -> bool {
        false
    }

    /// Mark the event as processed (success) or failed. Called by
    /// the receiver AFTER `handle` completes (regardless of outcome).
    /// Default impl is a no-op.
    async fn mark_event(&self, _stripe_event_id: &str, _outcome: HandleOutcome) {}

    /// Process the event ; return `Ok(())` to ACK Stripe.
    async fn handle(&self, evt: &StripeEvent) -> Result<(), String>;
}

/// Outcome surfaced to [`WebhookHandler::mark_event`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandleOutcome {
    /// Event processed successfully.
    Success,
    /// Event was a duplicate (already in dedup table).
    Duplicate,
    /// Handler returned an error.
    Failed,
    /// Event was skipped (unrecognised type).
    Skipped,
}

/// Shared state required by [`stripe_webhook`].
#[derive(Clone)]
pub struct WebhookState {
    /// Verifier configuration (signing secret + tolerance).
    pub config: WebhookConfig,
    /// Concrete handler — installed by the binary at startup.
    pub handler: Arc<dyn WebhookHandler>,
}

impl WebhookState {
    /// Construct from verifier config + handler.
    pub fn new(config: WebhookConfig, handler: Arc<dyn WebhookHandler>) -> Self {
        Self { config, handler }
    }
}

/// `POST /v1/billing/webhook` axum handler.
pub async fn stripe_webhook(
    State(state): State<WebhookState>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let sig = match headers
        .get("stripe-signature")
        .and_then(|v| v.to_str().ok())
    {
        Some(s) => s.to_string(),
        None => {
            return error_response(StatusCode::BAD_REQUEST, "missing stripe-signature")
                .into_response();
        }
    };
    let raw = match axum::body::to_bytes(body, 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            return error_response(StatusCode::BAD_REQUEST, &format!("body read: {e}"))
                .into_response();
        }
    };
    let now = chrono::Utc::now().timestamp();
    let evt = match verify_stripe_signature(&raw, &sig, now, &state.config) {
        Ok(e) => e,
        Err(e) => {
            warn!(error = %e, "stripe webhook verification failed");
            let status = StatusCode::from_u16(e.http_status()).unwrap_or(StatusCode::BAD_REQUEST);
            return error_response(status, &e.to_string()).into_response();
        }
    };

    // M31 audit fix : dedup BEFORE the side-effecting handler.
    // Stripe retries on 5xx for up to 3 days, so re-processing the
    // same `evt_...` id would double-write our PG mirror.
    if state.handler.is_event_processed(&evt.id).await {
        state
            .handler
            .mark_event(&evt.id, HandleOutcome::Duplicate)
            .await;
        return (StatusCode::OK, "duplicate (already processed)").into_response();
    }

    match state.handler.handle(&evt).await {
        Ok(()) => {
            state
                .handler
                .mark_event(&evt.id, HandleOutcome::Success)
                .await;
            (StatusCode::OK, "ok").into_response()
        }
        Err(e) => {
            state
                .handler
                .mark_event(&evt.id, HandleOutcome::Failed)
                .await;
            warn!(event_id = %evt.id, error = %e, "stripe webhook handler error");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, &e).into_response()
        }
    }
}

/// Build the webhook router. Independent of the main billing router
/// because the webhook auth model is HMAC, not Bearer.
pub fn webhook_router(state: WebhookState) -> Router {
    Router::new()
        .route("/webhook", axum::routing::post(stripe_webhook))
        .with_state(state)
}

fn error_response(status: StatusCode, msg: &str) -> impl IntoResponse {
    let body = serde_json::json!({"error": msg});
    (status, axum::Json(body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use tower::ServiceExt;

    struct NoopHandler;
    #[async_trait]
    impl WebhookHandler for NoopHandler {
        async fn handle(&self, _evt: &StripeEvent) -> Result<(), String> {
            Ok(())
        }
    }

    /// Dedup-aware test handler — short-circuits `handle` for a
    /// specific pre-seeded event id.
    struct DedupedHandler {
        already_seen: String,
        success_count: std::sync::atomic::AtomicU32,
        duplicate_count: std::sync::atomic::AtomicU32,
    }
    impl DedupedHandler {
        fn new(already_seen: &str) -> Self {
            Self {
                already_seen: already_seen.to_string(),
                success_count: 0.into(),
                duplicate_count: 0.into(),
            }
        }
    }
    #[async_trait]
    impl WebhookHandler for DedupedHandler {
        async fn is_event_processed(&self, evt_id: &str) -> bool {
            evt_id == self.already_seen
        }
        async fn mark_event(&self, _evt_id: &str, outcome: HandleOutcome) {
            use std::sync::atomic::Ordering;
            match outcome {
                HandleOutcome::Success => {
                    self.success_count.fetch_add(1, Ordering::SeqCst);
                }
                HandleOutcome::Duplicate => {
                    self.duplicate_count.fetch_add(1, Ordering::SeqCst);
                }
                _ => {}
            }
        }
        async fn handle(&self, _evt: &StripeEvent) -> Result<(), String> {
            Ok(())
        }
    }

    fn sign(secret: &str, ts: i64, body: &[u8]) -> String {
        let signed = format!("{ts}.");
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(signed.as_bytes());
        mac.update(body);
        let hex = hex::encode(mac.finalize().into_bytes());
        format!("t={ts},v1={hex}")
    }

    #[tokio::test]
    async fn webhook_missing_signature_400() {
        let cfg = WebhookConfig {
            signing_secret: "whsec_x".into(),
            tolerance_secs: 60,
        };
        let state = WebhookState::new(cfg, Arc::new(NoopHandler));
        let app = webhook_router(state);
        let req = Request::builder()
            .method("POST")
            .uri("/webhook")
            .body(Body::from("{}"))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn webhook_invalid_signature_400() {
        let cfg = WebhookConfig {
            signing_secret: "whsec_x".into(),
            tolerance_secs: 60,
        };
        let state = WebhookState::new(cfg, Arc::new(NoopHandler));
        let app = webhook_router(state);
        let body = br#"{"id":"evt_1","type":"x","created":0,"data":{}}"#;
        let req = Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("stripe-signature", "t=1,v1=ff")
            .body(Body::from(&body[..]))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn webhook_happy_path_200() {
        let now = chrono::Utc::now().timestamp();
        let body = format!(r#"{{"id":"evt_1","type":"invoice.paid","created":{now},"data":{{}}}}"#)
            .into_bytes();
        let cfg = WebhookConfig {
            signing_secret: "whsec_x".into(),
            tolerance_secs: 60,
        };
        let state = WebhookState::new(cfg, Arc::new(NoopHandler));
        let app = webhook_router(state);
        let sig = sign("whsec_x", now, &body);
        let req = Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("stripe-signature", sig)
            .body(Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn webhook_dedup_short_circuits_handler() {
        let now = chrono::Utc::now().timestamp();
        let body =
            format!(r#"{{"id":"evt_dup_1","type":"invoice.paid","created":{now},"data":{{}}}}"#)
                .into_bytes();
        let cfg = WebhookConfig {
            signing_secret: "whsec_x".into(),
            tolerance_secs: 60,
        };
        let handler = Arc::new(DedupedHandler::new("evt_dup_1"));
        let state = WebhookState::new(cfg, handler.clone());
        let app = webhook_router(state);
        let sig = sign("whsec_x", now, &body);
        let req = Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("stripe-signature", sig)
            .body(Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        use std::sync::atomic::Ordering;
        assert_eq!(handler.duplicate_count.load(Ordering::SeqCst), 1);
        assert_eq!(handler.success_count.load(Ordering::SeqCst), 0);
    }
}
