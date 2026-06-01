//! axum HTTPS server for the admission webhook.

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

use super::{validators, WebhookState};
use crate::AgentRunnerResult;

#[derive(Debug, Deserialize, Serialize)]
pub struct AdmissionRequest {
    pub uid: String,
    pub kind: serde_json::Value,
    pub resource: serde_json::Value,
    pub name: Option<String>,
    pub namespace: Option<String>,
    pub operation: String,
    pub user_info: Option<serde_json::Value>,
    pub object: Option<serde_json::Value>,
    #[serde(rename = "oldObject")]
    pub old_object: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AdmissionReview {
    pub api_version: String,
    pub kind: String,
    pub request: Option<AdmissionRequest>,
    pub response: Option<AdmissionResponseInner>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AdmissionResponseInner {
    pub uid: String,
    pub allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub status: Option<AdmissionStatus>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub warnings: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AdmissionStatus {
    pub message: String,
    pub code: u16,
}

/// Construct the webhook router. The TLS layer is wired by the caller
/// in `serve_https`.
pub fn router(state: WebhookState) -> Router {
    Router::new()
        .route("/validate-agentrun", post(validate_agentrun))
        .route("/validate-agentruntemplate", post(validate_agentruntemplate))
        .route("/healthz", get(healthz))
        .route("/readyz", get(|State(s): State<WebhookState>| async move { readyz(s).await }))
        .with_state(state)
}

async fn validate_agentrun(
    State(state): State<WebhookState>,
    Json(review): Json<AdmissionReview>,
) -> Response {
    let req = match review.request.as_ref() {
        Some(r) => r,
        None => return reject_review("missing AdmissionReview.request").into_response(),
    };

    let outcome = match req.object.as_ref() {
        Some(obj) => validators::validate_agent_run(&state, obj).await,
        None => Ok(super::ValidationOutcome::deny("missing object in admission request")),
    };

    let resp = match outcome {
        Ok(super::ValidationOutcome::Allow) => AdmissionResponseInner {
            uid: req.uid.clone(),
            allowed: true,
            status: None,
            warnings: None,
        },
        Ok(super::ValidationOutcome::Deny(msg)) => AdmissionResponseInner {
            uid: req.uid.clone(),
            allowed: false,
            status: Some(AdmissionStatus { message: msg, code: 403 }),
            warnings: None,
        },
        Err(e) => {
            tracing::error!(?e, "webhook AgentRun validator errored ; failing closed");
            AdmissionResponseInner {
                uid: req.uid.clone(),
                allowed: false,
                status: Some(AdmissionStatus {
                    message: format!("webhook internal error : {}", e),
                    code: 500,
                }),
                warnings: None,
            }
        }
    };

    Json(AdmissionReview {
        api_version: "admission.k8s.io/v1".into(),
        kind: "AdmissionReview".into(),
        request: None,
        response: Some(resp),
    })
    .into_response()
}

async fn validate_agentruntemplate(
    State(state): State<WebhookState>,
    Json(review): Json<AdmissionReview>,
) -> Response {
    let req = match review.request.as_ref() {
        Some(r) => r,
        None => return reject_review("missing AdmissionReview.request").into_response(),
    };

    let outcome = match req.object.as_ref() {
        Some(obj) => validators::validate_agent_run_template(&state, obj).await,
        None => Ok(super::ValidationOutcome::deny("missing object in admission request")),
    };

    let resp = match outcome {
        Ok(super::ValidationOutcome::Allow) => AdmissionResponseInner {
            uid: req.uid.clone(),
            allowed: true,
            status: None,
            warnings: None,
        },
        Ok(super::ValidationOutcome::Deny(msg)) => AdmissionResponseInner {
            uid: req.uid.clone(),
            allowed: false,
            status: Some(AdmissionStatus { message: msg, code: 403 }),
            warnings: None,
        },
        Err(e) => AdmissionResponseInner {
            uid: req.uid.clone(),
            allowed: false,
            status: Some(AdmissionStatus {
                message: format!("webhook internal error : {}", e),
                code: 500,
            }),
            warnings: None,
        },
    };

    Json(AdmissionReview {
        api_version: "admission.k8s.io/v1".into(),
        kind: "AdmissionReview".into(),
        request: None,
        response: Some(resp),
    })
    .into_response()
}

async fn healthz() -> &'static str { "ok" }

async fn readyz(state: WebhookState) -> Response {
    // Readiness : DB reachable + profile registry has at least 1 profile.
    if state.profiles.slugs().is_empty() {
        return (StatusCode::SERVICE_UNAVAILABLE, "no profiles loaded").into_response();
    }
    if let Err(_) = sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.pg)
        .await
    {
        return (StatusCode::SERVICE_UNAVAILABLE, "db unreachable").into_response();
    }
    "ready".into_response()
}

fn reject_review(msg: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(AdmissionReview {
            api_version: "admission.k8s.io/v1".into(),
            kind: "AdmissionReview".into(),
            request: None,
            response: Some(AdmissionResponseInner {
                uid: String::new(),
                allowed: false,
                status: Some(AdmissionStatus {
                    message: msg.into(),
                    code: 400,
                }),
                warnings: None,
            }),
        }),
    )
        .into_response()
}

/// Run the webhook HTTPS server with rustls TLS termination in-process.
///
/// `tls_cert_path` + `tls_key_path` point at PEM files written by
/// cert-manager into a mounted Secret. The handler watches the mtime
/// in `super::tls::spawn_watcher` and rebuilds the rustls config on
/// rotation ; existing connections finish on the old cert.
pub async fn serve_https(
    state: WebhookState,
    addr: SocketAddr,
    tls_cert_path: std::path::PathBuf,
    tls_key_path: std::path::PathBuf,
) -> AgentRunnerResult<()> {
    let app = router(state);
    let material = super::tls::TlsMaterial {
        cert_path: tls_cert_path,
        key_path: tls_key_path,
    };
    let rustls_config = super::tls::load(&material)
        .map_err(|e| crate::AgentRunnerError::Internal(format!("TLS load : {}", e)))?;
    tracing::info!(%addr, "webhook HTTPS server listening (rustls)");
    axum_server::bind_rustls(
        addr,
        axum_server::tls_rustls::RustlsConfig::from_config(rustls_config),
    )
    .serve(app.into_make_service())
    .await
    .map_err(|e| crate::AgentRunnerError::Internal(format!("webhook serve : {}", e)))?;
    Ok(())
}

/// Plaintext HTTP variant for sidecar-TLS deployments. Not used by the
/// canonical Helm chart in production.
pub async fn serve_http(state: WebhookState, addr: SocketAddr) -> AgentRunnerResult<()> {
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| crate::AgentRunnerError::Internal(format!("webhook bind : {}", e)))?;
    tracing::warn!(
        %addr,
        "webhook HTTP listener (no TLS) — only safe with sidecar TLS termination"
    );
    axum::serve(listener, app)
        .await
        .map_err(|e| crate::AgentRunnerError::Internal(format!("webhook serve : {}", e)))?;
    Ok(())
}
