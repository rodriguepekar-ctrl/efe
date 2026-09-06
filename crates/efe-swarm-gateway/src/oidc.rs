// SPDX-License-Identifier: UNLICENSED
//! `OidcVerifier` — auth `Caller` via **JWT OIDC ES256** (feature `oidc`).
//!
//! **Règle absolue tenue** : on ne manipule **que** la **clé PUBLIQUE** (vérification de
//! signature), lue depuis l'env **server-side** (`EFE_OIDC_PUBLIC_KEY_PEM`) — jamais une clé
//! privée, jamais un secret saisi. Le JWT porte le `tenant` + les `roles` ; on les projette
//! sur le [`Caller`] (même surface que l'extracteur dev → les handlers ne changent pas).

use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::Deserialize;

use efe_nano_sdk::TenantId;

use crate::auth::Caller;
use crate::error::GatewayError;

/// Claims OIDC reconnues. `tenant`/`tid` portent le tenant ; `roles` (ou `realm_access.roles`
/// façon Keycloak) portent le RBAC. `exp`/`iss`/`aud` sont validés par `jsonwebtoken`.
#[derive(Debug, Deserialize)]
struct Claims {
    #[allow(dead_code)]
    sub: String,
    #[serde(default)]
    tenant: Option<String>,
    #[serde(default)]
    tid: Option<String>,
    #[serde(default)]
    roles: Option<Vec<String>>,
    #[serde(default)]
    realm_access: Option<RealmAccess>,
}

#[derive(Debug, Deserialize)]
struct RealmAccess {
    #[serde(default)]
    roles: Vec<String>,
}

/// Vérificateur de JWT OIDC (clé publique ES256 + politique de validation).
pub struct OidcVerifier {
    key: DecodingKey,
    validation: Validation,
}

impl OidcVerifier {
    /// Construit depuis une **clé publique PEM** (EC P-256) + issuer/audience optionnels.
    pub fn from_pem(public_key_pem: &str, issuer: Option<&str>, audience: Option<&str>) -> Result<Self, GatewayError> {
        let key = DecodingKey::from_ec_pem(public_key_pem.as_bytes())
            .map_err(|e| GatewayError::Internal(format!("clé publique OIDC invalide : {e}")))?;
        let mut validation = Validation::new(Algorithm::ES256);
        if let Some(iss) = issuer {
            validation.set_issuer(&[iss]);
        }
        match audience {
            Some(aud) => validation.set_audience(&[aud]),
            None => validation.validate_aud = false,
        }
        Ok(Self { key, validation })
    }

    /// Construit depuis l'env **server-side** : `EFE_OIDC_PUBLIC_KEY_PEM` (requis),
    /// `EFE_OIDC_ISSUER_URL` + `EFE_OIDC_AUDIENCE` (optionnels). `None` si non configuré.
    pub fn from_env() -> Result<Option<Self>, GatewayError> {
        let Some(pem) = crate::profile::read::oidc_public_key_pem() else {
            return Ok(None);
        };
        let issuer = crate::profile::read::oidc_issuer_url();
        let audience = crate::profile::read::oidc_audience();
        Ok(Some(Self::from_pem(&pem, issuer.as_deref(), audience.as_deref())?))
    }

    /// **Couche 2 (M3)** : vérificateur **dédié au chemin frontière** — audience **épinglée à
    /// `efe-llm`** (NON conditionnelle : ignore `EFE_OIDC_AUDIENCE`). Un jeton d'essaim
    /// souverain (`aud` absent ou autre) est ainsi **rejeté 401** avant tout egress tiers
    /// coûteux (§4). `EFE_OIDC_LLM_AUDIENCE` surcharge l'audience (défaut `efe-llm`) ; même
    /// clé publique + issuer que [`Self::from_env`]. `None` si `EFE_OIDC_PUBLIC_KEY_PEM` absent.
    pub fn llm_from_env() -> Result<Option<Self>, GatewayError> {
        let Some(pem) = crate::profile::read::oidc_public_key_pem() else {
            return Ok(None);
        };
        let issuer = crate::profile::read::oidc_issuer_url();
        let audience = crate::profile::read::oidc_llm_audience();
        Ok(Some(Self::from_pem(&pem, issuer.as_deref(), Some(&audience))?))
    }

    /// Vérifie le jeton → `Caller`. Signature/exp/iss/aud invalides → 401.
    pub fn verify(&self, token: &str) -> Result<Caller, GatewayError> {
        let data = decode::<Claims>(token, &self.key, &self.validation)
            .map_err(|e| GatewayError::Unauthenticated(format!("JWT invalide : {e}")))?;
        Ok(claims_to_caller(data.claims))
    }
}

fn claims_to_caller(c: Claims) -> Caller {
    let tenant = c.tenant.or(c.tid).filter(|s| !s.is_empty()).unwrap_or_else(|| TenantId::system().to_string());
    let mut roles = c.roles.unwrap_or_default();
    if let Some(ra) = c.realm_access {
        roles.extend(ra.roles);
    }
    Caller { tenant, roles }
}

/// Extrait le jeton d'un en-tête `Authorization: Bearer <token>`.
pub(crate) fn parse_bearer(headers: &axum::http::HeaderMap) -> Option<String> {
    let raw = headers.get(axum::http::header::AUTHORIZATION)?.to_str().ok()?;
    raw.strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// **La paire de test, partagée** — pour que le critère de M3 soit prouvable SOUS `oidc`.
///
/// Elle vivait dans le `mod tests` de ce fichier, donc invisible ailleurs. Conséquence : les deux
/// tests qui portent le critère de M3 s'authentifiaient par en-têtes de développement et étaient
/// `#[cfg(not(feature = "oidc"))]` — c'est-à-dire qu'ils **ne tournaient sous aucun jeu de features
/// de production**. Le trou n'était pas dans M3 : il était dans l'accessibilité de cette fixture.
#[cfg(test)]
pub(crate) mod fixture {
    use super::{Algorithm, GatewayError, OidcVerifier};
    use jsonwebtoken::{encode, EncodingKey, Header};

    // ── Paire de clés EC P-256 de TEST (fixture déterministe — PAS un secret) ──────
    // Générée localement pour les tests unitaires de validation JWT. Ne protège rien.
    pub(crate) const TEST_PRIV_PKCS8: &str = "-----BEGIN PRIVATE KEY-----\n\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgv4ad14ofQ83JXwDG\n\
VEWEF6e59s+JE5EvkQaqvNuxN1ShRANCAARoN8FXUFCqiAOqKjEZBXA9x83gosue\n\
1m0JJbEEeOUyiLXTD1lsnCVeQKm3hfz6Dkk06VYWCtBVfNBO13YdRLC5\n\
-----END PRIVATE KEY-----\n";
    pub(crate) const TEST_PUB: &str = "-----BEGIN PUBLIC KEY-----\n\
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEaDfBV1BQqogDqioxGQVwPcfN4KLL\n\
ntZtCSWxBHjlMoi10w9ZbJwlXkCpt4X8+g5JNOlWFgrQVXzQTtd2HUSwuQ==\n\
-----END PUBLIC KEY-----\n";

    /// Signe des revendications arbitraires avec la clé privée de test.
    pub(crate) fn sign(claims: serde_json::Value) -> String {
        encode(
            &Header::new(Algorithm::ES256),
            &claims,
            &EncodingKey::from_ec_pem(TEST_PRIV_PKCS8.as_bytes()).unwrap(),
        )
        .unwrap()
    }

    /// Un vérifieur du chemin frontière — audience épinglée `efe-llm`, comme `llm_from_env`.
    pub(crate) fn verifieur_llm() -> Result<OidcVerifier, GatewayError> {
        OidcVerifier::from_pem(TEST_PUB, None, Some("efe-llm"))
    }

    /// Un jeton d'opérateur valide pour le chemin frontière.
    pub(crate) fn jeton_operateur(tenant: &str) -> String {
        sign(serde_json::json!({
            "sub": "test-operateur", "tenant": tenant, "aud": "efe-llm",
            "roles": ["operator"], "exp": 9_999_999_999i64
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::fixture::{sign, TEST_PUB};

    // Les constantes vivent désormais dans `fixture` (cf. ci-dessus). Ce bloc les emploie.

    #[test]
    fn verifies_valid_jwt_and_maps_tenant_and_roles() {
        let token = sign(serde_json::json!({
            "sub": "u-1", "tenant": "11111111-1111-1111-1111-111111111111",
            "roles": ["operator", "viewer"], "exp": 9_999_999_999i64
        }));
        let v = OidcVerifier::from_pem(TEST_PUB, None, None).unwrap();
        let caller = v.verify(&token).unwrap();
        assert_eq!(caller.tenant, "11111111-1111-1111-1111-111111111111");
        assert!(caller.roles.contains(&"operator".to_string()));
        assert!(caller.require_any_role(&["operator"]).is_ok());
    }

    #[test]
    fn keycloak_realm_access_roles_are_extracted() {
        let token = sign(serde_json::json!({
            "sub": "u-2", "tid": "system",
            "realm_access": { "roles": ["admin"] }, "exp": 9_999_999_999i64
        }));
        let caller = OidcVerifier::from_pem(TEST_PUB, None, None).unwrap().verify(&token).unwrap();
        assert!(caller.roles.contains(&"admin".to_string()));
    }

    #[test]
    fn expired_token_is_rejected() {
        let token = sign(serde_json::json!({ "sub": "u", "exp": 1i64 })); // 1970 → expiré
        let err = OidcVerifier::from_pem(TEST_PUB, None, None).unwrap().verify(&token);
        assert!(matches!(err, Err(GatewayError::Unauthenticated(_))));
    }

    #[test]
    fn wrong_issuer_is_rejected() {
        let token = sign(serde_json::json!({ "sub": "u", "iss": "https://evil.example", "exp": 9_999_999_999i64 }));
        let v = OidcVerifier::from_pem(TEST_PUB, Some("https://trusted.example"), None).unwrap();
        assert!(matches!(v.verify(&token), Err(GatewayError::Unauthenticated(_))));
    }

    #[test]
    fn parse_bearer_handles_scheme() {
        let mut h = axum::http::HeaderMap::new();
        h.insert(axum::http::header::AUTHORIZATION, "Bearer abc.def.ghi".parse().unwrap());
        assert_eq!(parse_bearer(&h).as_deref(), Some("abc.def.ghi"));
        assert!(parse_bearer(&axum::http::HeaderMap::new()).is_none());
    }
}
