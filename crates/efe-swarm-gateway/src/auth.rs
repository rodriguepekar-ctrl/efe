// SPDX-License-Identifier: UNLICENSED
//! `Caller` — l'extracteur d'identité/tenant.
//!
//! **Inc.2 (dev/standalone)** : lit `x-efe-tenant` (défaut : tenant système) et
//! `x-efe-roles` (CSV). **Aucune clé manipulée.** En **prod**, ce `FromRequestParts` est
//! remplacé par l'`AuthCtx` OIDC ES256 d'`efe-server` (clés `EFE_OIDC_*` **server-side**) —
//! même surface (`tenant`, `roles`, `require_any_role`), donc les handlers ne changent pas.
//!
//! La règle absolue tient : on **dérive** la clé de provenance par tenant **server-side**
//! (dans l'état), jamais ici ; on n'expose que le `root`.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::HeaderMap;
use efe_nano_sdk::TenantId;

use crate::error::GatewayError;

const HEADER_TENANT: &str = "x-efe-tenant";
const HEADER_ROLES: &str = "x-efe-roles";

/// Identité de l'appelant : tenant + rôles (RBAC).
#[derive(Debug, Clone)]
pub struct Caller {
    pub tenant: String,
    pub roles: Vec<String>,
}

impl Caller {
    /// Exige l'un des rôles `allowed`. **Mode dev** : si l'appelant ne déclare **aucun**
    /// rôle (pas d'en-tête `x-efe-roles`), on autorise (standalone keyless). En prod,
    /// l'`AuthCtx` OIDC fournit toujours des rôles → la vérification mord réellement.
    pub fn require_any_role(&self, allowed: &[&str]) -> Result<(), GatewayError> {
        if self.roles.is_empty() || self.roles.iter().any(|r| allowed.contains(&r.as_str())) {
            Ok(())
        } else {
            Err(GatewayError::Forbidden(format!("rôle requis parmi {allowed:?}")))
        }
    }
}

/// Extraction **dev/standalone** : tenant + rôles depuis les en-têtes (`x-efe-*`).
pub(crate) fn dev_caller_from_headers(headers: &HeaderMap) -> Caller {
    let tenant = headers
        .get(HEADER_TENANT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| TenantId::system().to_string());
    let roles = headers
        .get(HEADER_ROLES)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').map(|r| r.trim().to_string()).filter(|r| !r.is_empty()).collect())
        .unwrap_or_default();
    Caller { tenant, roles }
}

/// **Dev/standalone** (sans la feature `oidc`) : extracteur générique par en-têtes.
#[cfg(not(feature = "oidc"))]
#[async_trait::async_trait]
impl<S: Send + Sync> FromRequestParts<S> for Caller {
    type Rejection = GatewayError;
    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(dev_caller_from_headers(&parts.headers))
    }
}

/// **Prod** (feature `oidc`) : valide un JWT OIDC ES256 (clé publique server-side dans
/// l'état). Si OIDC non configuré (état sans vérificateur), repli sur les en-têtes (dev).
/// Même type `Caller` → les handlers ne changent pas.
#[cfg(feature = "oidc")]
#[async_trait::async_trait]
impl FromRequestParts<crate::state::SwarmGatewayState> for Caller {
    type Rejection = GatewayError;
    async fn from_request_parts(
        parts: &mut Parts,
        state: &crate::state::SwarmGatewayState,
    ) -> Result<Self, Self::Rejection> {
        match &state.oidc {
            Some(verifier) => {
                let token = crate::oidc::parse_bearer(&parts.headers)
                    .ok_or_else(|| GatewayError::Unauthenticated("jeton Bearer manquant".into()))?;
                verifier.verify(&token)
            }
            None => Ok(dev_caller_from_headers(&parts.headers)),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Couche 2 (M3) — `LlmCaller` : l'identité du chemin frontière NON-souverain.
// ─────────────────────────────────────────────────────────────────────────────

/// Identité de l'appelant sur le chemin **frontière** (`/v1/llm/*`, egress tiers coûteux).
/// Enveloppe [`Caller`] mais, sous la feature `oidc`, impose un JWT **`aud=efe-llm`** (via
/// `state.oidc_llm`) : un jeton d'essaim souverain ne déverrouille PAS l'egress non-souverain
/// (§4). Le RBAC y est **strict** (voir [`LlmCaller::require_any_role`]).
#[derive(Debug, Clone)]
pub struct LlmCaller(pub Caller);

impl std::ops::Deref for LlmCaller {
    type Target = Caller;
    fn deref(&self) -> &Caller {
        &self.0
    }
}

impl LlmCaller {
    /// RBAC **strict** (chemin coûteux) : contrairement à [`Caller::require_any_role`], des
    /// rôles **vides sont refusés** — le frontière exige un rôle explicite (§4 puce 4 : pas
    /// de keyless sur l'egress payant). Masque (inherent > Deref) le `require_any_role` laxiste.
    pub fn require_any_role(&self, allowed: &[&str]) -> Result<(), GatewayError> {
        if self.0.roles.iter().any(|r| allowed.contains(&r.as_str())) {
            Ok(())
        } else {
            Err(GatewayError::Forbidden(format!("rôle requis parmi {allowed:?} (chemin frontière)")))
        }
    }
}

/// **Dev/standalone** (sans `oidc`) : mêmes en-têtes `x-efe-*` que [`Caller`] — parité de dev
/// (le durcissement `aud=efe-llm` mord sur le build prod `oidc`, seul chemin où l'egress réel
/// existe). Le RBAC strict de [`LlmCaller::require_any_role`] s'applique quand même.
#[cfg(not(feature = "oidc"))]
#[async_trait::async_trait]
impl<S: Send + Sync> FromRequestParts<S> for LlmCaller {
    type Rejection = GatewayError;
    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(LlmCaller(dev_caller_from_headers(&parts.headers)))
    }
}

/// **Prod** (feature `oidc`) : exige un JWT **`aud=efe-llm`** via `state.oidc_llm`. **Aucun
/// repli keyless** : `oidc_llm` absent (prod mal-configurée) ⟹ **401** (fail-closed, chemin
/// non-souverain) ; `aud` absent/autre ⟹ 401 (`InvalidAudience` de `jsonwebtoken`).
#[cfg(feature = "oidc")]
#[async_trait::async_trait]
impl FromRequestParts<crate::state::SwarmGatewayState> for LlmCaller {
    type Rejection = GatewayError;
    async fn from_request_parts(
        parts: &mut Parts,
        state: &crate::state::SwarmGatewayState,
    ) -> Result<Self, Self::Rejection> {
        let verifier = state
            .oidc_llm
            .as_ref()
            .ok_or_else(|| GatewayError::Unauthenticated("auth frontière (aud=efe-llm) non configurée".into()))?;
        let token = crate::oidc::parse_bearer(&parts.headers)
            .ok_or_else(|| GatewayError::Unauthenticated("jeton Bearer manquant".into()))?;
        verifier.verify(&token).map(LlmCaller)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn llm_caller_require_any_role_is_strict_rejects_empty() {
        // Le chemin frontière refuse les rôles vides (contraste avec Caller laxiste).
        let empty = LlmCaller(Caller { tenant: "system".into(), roles: vec![] });
        assert!(empty.require_any_role(&["operator", "admin"]).is_err(), "rôles vides ⟹ refus (frontière)");
        let op = LlmCaller(Caller { tenant: "system".into(), roles: vec!["operator".into()] });
        assert!(op.require_any_role(&["operator", "admin"]).is_ok());
        assert!(op.require_any_role(&["admin"]).is_err(), "rôle présent mais hors liste ⟹ refus");
    }

    #[test]
    fn require_any_role_dev_allows_when_no_roles() {
        let c = Caller { tenant: "system".into(), roles: vec![] };
        assert!(c.require_any_role(&["operator"]).is_ok());
    }

    #[test]
    fn require_any_role_enforces_when_roles_present() {
        let c = Caller { tenant: "system".into(), roles: vec!["viewer".into()] };
        assert!(c.require_any_role(&["operator", "admin"]).is_err());
        assert!(c.require_any_role(&["viewer"]).is_ok());
    }
}
