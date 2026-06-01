//! JWT validation — HS256, multi-key rotation, scope checks.

use std::sync::{Arc, RwLock};

use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};

use efe_core::{EfeError, Result};

/// JWT claims recognised by the server.
///
/// `tenant_id` and `scopes` are custom claims — the rest follow RFC 7519.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Claims {
    /// Issuer.
    pub iss: Option<String>,
    /// Subject (user / service account).
    pub sub: String,
    /// Audience. Must match the configured `jwt_audience`.
    pub aud: String,
    /// Expiry (seconds since epoch). RFC 7519 NumericDate; fixed to
    /// `u64` for platform portability (`usize` is 32-bit on 32-bit hosts
    /// and would overflow past 2038).
    pub exp: u64,
    /// Issued at (seconds since epoch).
    pub iat: Option<u64>,
    /// Tenant identifier, propagated to domain `Request.tenant_id`.
    pub tenant_id: String,
    /// Space-delimited list of scopes (e.g. `"swarm:execute admin:cone:read"`).
    #[serde(default)]
    pub scopes: Vec<String>,
}

impl Claims {
    /// Check if a scope is granted by these claims.
    #[must_use]
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope)
    }

    /// Build a synthetic `anonymous` claim — used for public endpoints
    /// (e.g. `Health`) and as the bypass when auth is disabled.
    ///
    /// **Φ-D-OPS audit fix** : the anonymous claim now carries the
    /// three D22 broad scopes (`data`, `admin`, `audit`). Without
    /// this, downstream `ScopeGuard::require(...)` would reject every
    /// request when `auth.enabled = false` — defeating the test
    /// harness and any "trusted environment" deployment. Granular
    /// per-RPC scopes (e.g. `swarm:execute`) are still NOT granted ;
    /// callers must rely on the auth-disabled config flag for those.
    ///
    /// Security rationale : `enabled = false` is itself a strong
    /// trust signal (= local dev, `k3d` smoke, CI fixture). Carrying
    /// broad scopes here doesn't bypass the granular scope gate
    /// because the **`required_scope`** parameter of `AuthGuard::check`
    /// is what's tested first ; the broad scope is purely additive
    /// defence-in-depth that only applies when the granular gate
    /// already approved the call path.
    #[must_use]
    pub fn anonymous() -> Self {
        Self {
            iss: Some("efe.anonymous".into()),
            sub: "anonymous".into(),
            aud: "efe.v1".into(),
            exp: u64::MAX,
            iat: None,
            tenant_id: "anonymous".into(),
            scopes: vec!["data".to_string(), "admin".to_string(), "audit".to_string()],
        }
    }
}

/// HS256 JWT validator with zero-downtime key rotation.
///
/// Multiple keys can be accepted simultaneously. During a rotation the
/// new and previous keys coexist ; after the grace period only the
/// newest remains.
#[derive(Clone)]
pub struct JwtValidator {
    keys: Arc<RwLock<Vec<DecodingKey>>>,
    validation: Validation,
}

impl std::fmt::Debug for JwtValidator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let len = self.keys.read().map(|k| k.len()).unwrap_or(0);
        f.debug_struct("JwtValidator")
            .field("keys", &format!("<{len} keys>"))
            .field("validation", &"<redacted>")
            .finish()
    }
}

impl JwtValidator {
    /// Construct a validator with the given HS256 secrets and audience.
    ///
    /// At least one secret is required.
    pub fn new(secrets: &[Vec<u8>], audience: &str, issuer: Option<&str>) -> Result<Self> {
        if secrets.is_empty() {
            return Err(EfeError::InvalidConfig(
                "JwtValidator: at least one HS256 secret is required".into(),
            ));
        }
        let keys: Vec<DecodingKey> = secrets
            .iter()
            .map(|s| DecodingKey::from_secret(s))
            .collect();
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_audience(&[audience]);
        // Zero leeway: reject tokens whose `exp` has just passed. A
        // generous leeway would hide clock drift bugs.
        validation.leeway = 0;
        if let Some(iss) = issuer {
            validation.set_issuer(&[iss]);
        }
        Ok(Self {
            keys: Arc::new(RwLock::new(keys)),
            validation,
        })
    }

    /// Replace the accepted keys (zero-downtime rotation).
    pub fn rotate_keys(&self, new: &[Vec<u8>]) -> Result<()> {
        if new.is_empty() {
            return Err(EfeError::InvalidConfig(
                "JwtValidator::rotate_keys: at least one key required".into(),
            ));
        }
        let mut w = self
            .keys
            .write()
            .map_err(|_| EfeError::Internal("JwtValidator: rotate_keys poisoned lock"))?;
        *w = new.iter().map(|s| DecodingKey::from_secret(s)).collect();
        Ok(())
    }

    /// Validate a token against every configured key until one succeeds.
    ///
    /// Returns the decoded `Claims` on success.
    pub fn validate(&self, token: &str) -> Result<Claims> {
        let keys = self
            .keys
            .read()
            .map_err(|_| EfeError::Internal("JwtValidator: validate poisoned lock"))?;
        let mut last_err: Option<String> = None;
        for key in keys.iter() {
            match decode::<Claims>(token, key, &self.validation) {
                Ok(data) => return Ok(data.claims),
                Err(e) => last_err = Some(e.to_string()),
            }
        }
        Err(EfeError::InvalidConfig(format!(
            "JWT validation failed: {}",
            last_err.unwrap_or_else(|| "no keys".into())
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    fn sign(secret: &[u8], claims: &Claims) -> String {
        encode(
            &Header::new(Algorithm::HS256),
            claims,
            &EncodingKey::from_secret(secret),
        )
        .unwrap()
    }

    fn base_claims() -> Claims {
        Claims {
            iss: Some("efe.internal".into()),
            sub: "svc".into(),
            aud: "efe.v1".into(),
            exp: now_secs() + 60,
            iat: Some(now_secs()),
            tenant_id: "acme".into(),
            scopes: vec!["swarm:execute".into()],
        }
    }

    #[test]
    fn accepts_valid_token() {
        let secret = b"super-secret".to_vec();
        let token = sign(&secret, &base_claims());
        let v = JwtValidator::new(&[secret], "efe.v1", None).unwrap();
        let claims = v.validate(&token).unwrap();
        assert_eq!(claims.tenant_id, "acme");
        assert!(claims.has_scope("swarm:execute"));
    }

    #[test]
    fn rejects_expired_token() {
        let secret = b"s".to_vec();
        let mut c = base_claims();
        c.exp = now_secs().saturating_sub(10);
        let token = sign(&secret, &c);
        let v = JwtValidator::new(&[secret], "efe.v1", None).unwrap();
        assert!(v.validate(&token).is_err());
    }

    #[test]
    fn rejects_wrong_audience() {
        let secret = b"s".to_vec();
        let token = sign(&secret, &base_claims());
        let v = JwtValidator::new(&[secret], "wrong-aud", None).unwrap();
        assert!(v.validate(&token).is_err());
    }

    #[test]
    fn rejects_wrong_signature() {
        let signing = b"signing-secret".to_vec();
        let validating = b"other-secret".to_vec();
        let token = sign(&signing, &base_claims());
        let v = JwtValidator::new(&[validating], "efe.v1", None).unwrap();
        assert!(v.validate(&token).is_err());
    }

    #[test]
    fn requires_at_least_one_secret() {
        assert!(JwtValidator::new(&[], "efe.v1", None).is_err());
    }

    #[test]
    fn rotation_accepts_new_key() {
        let old = b"old".to_vec();
        let new = b"new".to_vec();
        let v = JwtValidator::new(&[old.clone()], "efe.v1", None).unwrap();
        let tok_old = sign(&old, &base_claims());
        v.validate(&tok_old).unwrap();
        v.rotate_keys(&[new.clone()]).unwrap();
        assert!(v.validate(&tok_old).is_err()); // old no longer accepted.
        let tok_new = sign(&new, &base_claims());
        v.validate(&tok_new).unwrap();
    }

    #[test]
    fn rotation_accepts_both_during_transition() {
        let old = b"old".to_vec();
        let new = b"new".to_vec();
        let v = JwtValidator::new(&[old.clone(), new.clone()], "efe.v1", None).unwrap();
        let tok_old = sign(&old, &base_claims());
        let tok_new = sign(&new, &base_claims());
        v.validate(&tok_old).unwrap();
        v.validate(&tok_new).unwrap();
    }

    #[test]
    fn rotate_rejects_empty_key_list() {
        let v = JwtValidator::new(&[b"x".to_vec()], "efe.v1", None).unwrap();
        assert!(v.rotate_keys(&[]).is_err());
    }

    #[test]
    fn claims_anonymous_has_broad_scopes_only() {
        // Φ-D-OPS audit fix : when auth is disabled, the anonymous
        // claim must carry the three D22 broad scopes so downstream
        // `ScopeGuard::require(...)` calls don't reject every
        // request. Granular scopes (e.g. `swarm:execute`) are still
        // NOT granted — those are enforced first by the
        // `AuthGuard::check` `required_scope` parameter, before the
        // broad-scope check runs.
        let c = Claims::anonymous();
        assert_eq!(c.tenant_id, "anonymous");
        // Broad scopes present
        assert!(c.has_scope("data"));
        assert!(c.has_scope("admin"));
        assert!(c.has_scope("audit"));
        // Granular scopes absent
        assert!(!c.has_scope("swarm:execute"));
        assert!(!c.has_scope("admin:cone:write"));
        // Exactly the 3 broad scopes, no more.
        assert_eq!(c.scopes.len(), 3);
    }

    #[test]
    fn claims_has_scope_checks_membership() {
        let c = base_claims();
        assert!(c.has_scope("swarm:execute"));
        assert!(!c.has_scope("admin:cone:write"));
    }
}
