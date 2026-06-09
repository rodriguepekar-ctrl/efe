// SPDX-License-Identifier: UNLICENSED
//! `GatewayError` — l'erreur de fil : un code HTTP + un corps JSON stable.
//!
//! Mappe [`SwarmError`] (SDK) vers le bon statut HTTP. Aucune clé ni secret n'est exposé :
//! seules des descriptions sûres remontent au client.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use efe_swarm_sdk::SwarmError;

/// Erreur de la passerelle → `(status, { error, message })`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatewayError {
    /// 401 — pas (ou mal) authentifié.
    Unauthenticated(String),
    /// 403 — authentifié mais rôle/tenant insuffisant.
    Forbidden(String),
    /// 404 — run introuvable (ou hors du tenant de l'appelant).
    NotFound,
    /// 400 — DTO invalide (topologie vide, paramètre incohérent…).
    BadRequest(String),
    /// 402 — budget : estimation > plafond, ou épuisé en cours (politique `Error`).
    Budget(String),
    /// 429 — trop d'abonnés SSE pour ce tenant (parade anti-DoS).
    TooManyRequests(String),
    /// 503 — backend self-host injoignable (health-check).
    Unavailable(String),
    /// 501 — fonctionnalité non câblée en Inc.2 (ex. routeur `Network`).
    NotImplemented(String),
    /// 500 — erreur interne (dérivation de clé, persistance…).
    Internal(String),
}

impl GatewayError {
    fn parts(&self) -> (StatusCode, &'static str, String) {
        match self {
            GatewayError::Unauthenticated(m) => (StatusCode::UNAUTHORIZED, "unauthenticated", m.clone()),
            GatewayError::Forbidden(m) => (StatusCode::FORBIDDEN, "forbidden", m.clone()),
            GatewayError::NotFound => (StatusCode::NOT_FOUND, "not_found", "run introuvable".into()),
            GatewayError::BadRequest(m) => (StatusCode::BAD_REQUEST, "bad_request", m.clone()),
            GatewayError::Budget(m) => (StatusCode::PAYMENT_REQUIRED, "budget", m.clone()),
            GatewayError::TooManyRequests(m) => (StatusCode::TOO_MANY_REQUESTS, "too_many_requests", m.clone()),
            GatewayError::Unavailable(m) => (StatusCode::SERVICE_UNAVAILABLE, "unavailable", m.clone()),
            GatewayError::NotImplemented(m) => (StatusCode::NOT_IMPLEMENTED, "not_implemented", m.clone()),
            GatewayError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, "internal", m.clone()),
        }
    }
}

impl std::fmt::Display for GatewayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (status, code, msg) = self.parts();
        write!(f, "{} ({code}): {msg}", status.as_u16())
    }
}

impl std::error::Error for GatewayError {}

impl IntoResponse for GatewayError {
    fn into_response(self) -> Response {
        let (status, code, msg) = self.parts();
        (status, Json(json!({ "error": code, "message": msg }))).into_response()
    }
}

/// Mappe l'erreur du SDK vers le statut HTTP idoine.
impl From<SwarmError> for GatewayError {
    fn from(e: SwarmError) -> Self {
        match e {
            SwarmError::UnknownAgent { .. } | SwarmError::EmptySpec(_) => GatewayError::BadRequest(e.to_string()),
            // Garde-fou souveraineté : endpoint self-host non-local refusé → requête invalide.
            SwarmError::RemoteBackendDenied { .. } => GatewayError::BadRequest(e.to_string()),
            // Backend local injoignable (health-check) → 503.
            SwarmError::BackendUnavailable { .. } => GatewayError::Unavailable(e.to_string()),
            SwarmError::BudgetTooLow { .. } | SwarmError::BudgetExceeded { .. } => GatewayError::Budget(e.to_string()),
            SwarmError::KeyError(_) => GatewayError::Internal(e.to_string()),
            // En Inc.2 le routeur frontière n'est pas câblé : la passerelle, qui détiendrait
            // les clés server-side, prendra le relais en Inc.4 ; ici on l'annonce honnêtement.
            SwarmError::NetworkRequiresGateway => GatewayError::NotImplemented(
                "ClientMode::Network : routeur frontière non câblé en Inc.2 (clés server-side à venir)".into(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_sdk_errors_to_statuses() {
        let bad: GatewayError = SwarmError::EmptySpec("vide").into();
        assert!(matches!(bad, GatewayError::BadRequest(_)));
        let budget: GatewayError = SwarmError::BudgetTooLow { needed_cents: 10, max_cents: 3 }.into();
        assert!(matches!(budget, GatewayError::Budget(_)));
        let net: GatewayError = SwarmError::NetworkRequiresGateway.into();
        assert_eq!(net.parts().0, StatusCode::NOT_IMPLEMENTED);
    }

    #[test]
    fn statuses_are_stable() {
        assert_eq!(GatewayError::NotFound.parts().0, StatusCode::NOT_FOUND);
        assert_eq!(GatewayError::Forbidden("x".into()).parts().0, StatusCode::FORBIDDEN);
        assert_eq!(GatewayError::Budget("x".into()).parts().0, StatusCode::PAYMENT_REQUIRED);
    }
}
