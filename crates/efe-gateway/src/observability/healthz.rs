// SPDX-License-Identifier: UNLICENSED
//! **Sprint 14 — N22** : `/healthz` — liveness probe — § 9.1 + I16.

use axum::{http::StatusCode, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthzResponse {
    pub status: &'static str,
    pub version: &'static str,
}

/// **N22 + I16** : Liveness probe — returns 200 as long as the
/// process is alive (K8s livenessProbe will only fail if this
/// handler hangs, indicating deadlock).
pub async fn healthz_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(HealthzResponse {
            status: "ok",
            version: crate::CRATE_VERSION,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[tokio::test]
    async fn healthz_returns_200() {
        let resp = healthz_handler().await.into_response();
        assert_eq!(resp.status(), StatusCode::OK);
        // Parse as serde_json::Value so we don't bind &'static lifetimes.
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed["status"], "ok");
        assert!(parsed["version"].as_str().is_some());
    }
}
