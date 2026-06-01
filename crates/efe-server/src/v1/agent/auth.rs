//! OIDC bearer-token authentication middleware.
//!
//! The middleware is applied to the entire `/v1/agent/*` sub-router.
//! It extracts a JWT from `Authorization: Bearer ...`, validates the
//! signature against `EFE_OIDC_PUBLIC_KEY_PEM`, the audience
//! (`efe-agent-runtime`), the issuer (`EFE_OIDC_ISSUER_URL`), and
//! exposes a typed `AuthCtx` to handlers.

use axum::{extract::FromRequestParts, http::request::Parts};
use jsonwebtoken::{decode, DecodingKey, Validation};
use serde::Deserialize;

use super::errors::AgentApiError;

/// Minimal claim set we care about. Other claims (`name`, `email`, etc.)
/// are tolerated but ignored.
#[derive(Debug, Deserialize)]
struct OidcClaims {
    pub sub: String,
    pub tenant: String,
    #[serde(default)]
    pub roles: Vec<String>,
    pub iat: i64,
    pub exp: i64,
}

/// Caller identity surfaced to handlers.
#[derive(Debug, Clone)]
pub struct AuthCtx {
    pub subject: String,
    pub tenant: String,
    pub roles: Vec<String>,
    pub token_age_seconds: i64,
}

impl AuthCtx {
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role)
    }

    pub fn require_role(&self, role: &str) -> Result<(), AgentApiError> {
        if self.has_role(role) {
            Ok(())
        } else {
            Err(AgentApiError::Forbidden(format!(
                "role '{}' required",
                role
            )))
        }
    }

    pub fn require_any_role(&self, roles: &[&str]) -> Result<(), AgentApiError> {
        if roles.iter().any(|r| self.has_role(r)) {
            Ok(())
        } else {
            Err(AgentApiError::Forbidden(format!(
                "one of roles {:?} required",
                roles
            )))
        }
    }

    pub fn require_tenant(&self, tenant: &str) -> Result<(), AgentApiError> {
        if self.tenant == tenant {
            Ok(())
        } else {
            Err(AgentApiError::Forbidden(format!(
                "caller tenant '{}' != requested '{}'",
                self.tenant, tenant
            )))
        }
    }
}

#[async_trait::async_trait]
impl<S: Send + Sync> FromRequestParts<S> for AuthCtx {
    type Rejection = AgentApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let bearer = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .ok_or(AgentApiError::Unauthenticated)?;

        let key_pem = std::env::var("EFE_OIDC_PUBLIC_KEY_PEM")
            .map_err(|_| AgentApiError::Internal("OIDC key not configured".into()))?;
        let key = DecodingKey::from_ec_pem(key_pem.as_bytes())
            .map_err(|e| AgentApiError::Internal(format!("OIDC key parse: {}", e)))?;

        let issuer = std::env::var("EFE_OIDC_ISSUER_URL")
            .unwrap_or_else(|_| "https://oidc.efe.example.com".into());
        let mut validation = Validation::new(jsonwebtoken::Algorithm::ES256);
        validation.set_audience(&["efe-agent-runtime"]);
        validation.set_issuer(&[issuer.as_str()]);
        validation.leeway = 30;

        let claims: OidcClaims = decode(bearer, &key, &validation)
            .map_err(|e| {
                tracing::debug!(?e, "JWT validation failed");
                AgentApiError::InvalidToken
            })?
            .claims;

        let now = chrono::Utc::now().timestamp();
        let age = now - claims.iat;

        // Reject obvious replay : token from > 24h in the past.
        if age > 24 * 3600 {
            return Err(AgentApiError::TokenStale {
                age_seconds: age,
                max: 24 * 3600,
            });
        }

        Ok(AuthCtx {
            subject: claims.sub,
            tenant: claims.tenant,
            roles: claims.roles,
            token_age_seconds: age,
        })
    }
}
