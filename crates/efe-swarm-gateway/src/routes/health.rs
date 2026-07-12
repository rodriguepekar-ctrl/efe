// SPDX-License-Identifier: UNLICENSED
//! **Sondes de santé k8s** (`GET /healthz` liveness, `GET /readyz` readiness).
//!
//! Avant M4, liveness ET readiness (et le HEALTHCHECK Docker) tapaient `/metrics`, qui répond
//! `200` dès le bind du socket — un pod dont la **frontière est activée mais non câblée** (pas
//! de clé provider ⟹ `/v1/llm/*` = 501) restait **Ready**. On sépare :
//! - `/healthz` — liveness triviale : le process répond (redémarrer si non).
//! - `/readyz` — readiness **réelle** : refuse le trafic tant que la frontière **annoncée**
//!   (`EFE_LLM_FRONTIER_ENABLED=true`) n'a pas de coordinator câblé. Le boot guard M4 garantit
//!   déjà clé/auth/store ; `/readyz` ferme le dernier trou (clés provider absentes ⟹ 501).

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;

use crate::state::SwarmGatewayState;

/// **Liveness** : le process est vivant (redémarrage k8s si échec). Toujours `200 OK` ici — un
/// process qui ne répond plus ne renverra tout simplement rien (timeout ⟹ restart).
pub async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, "ok\n")
}

/// **Readiness** : le service peut-il servir ? `503` (retiré du load-balancer) si la frontière
/// est **annoncée** mais son coordinator n'est **pas câblé** (clés provider absentes) — sinon
/// `/v1/llm/*` renverrait 501 à des clients routés. Le chemin essaim reste toujours prêt.
pub async fn readyz(State(s): State<SwarmGatewayState>) -> impl IntoResponse {
    let _ = &s;
    #[cfg(feature = "frontier-llm")]
    {
        let frontier_intended =
            std::env::var("EFE_LLM_FRONTIER_ENABLED").map(|v| v == "true").unwrap_or(false);
        if frontier_intended && s.coordinator.is_none() {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "not ready: frontier enabled but coordinator not wired (missing ANTHROPIC_API_KEY / OPENAI_API_KEY / GOOGLE_AI_KEY)\n",
            );
        }
    }
    (StatusCode::OK, "ready\n")
}
