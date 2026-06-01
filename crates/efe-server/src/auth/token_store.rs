//! In-memory token store — wraps [`JwtValidator`] with a clean API.

use std::sync::Arc;

use crate::auth::jwt::{Claims, JwtValidator};
use efe_core::Result;

/// Opaque wrapper around [`JwtValidator`] that can be shared (`Arc`)
/// and injected into interceptors.
#[derive(Clone)]
pub struct TokenStore {
    validator: Arc<JwtValidator>,
}

impl std::fmt::Debug for TokenStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenStore")
            .field("validator", &self.validator)
            .finish()
    }
}

impl TokenStore {
    /// Construct from existing validator.
    #[must_use]
    pub fn new(validator: Arc<JwtValidator>) -> Self {
        Self { validator }
    }

    /// Build from raw HS256 secrets + audience.
    pub fn from_hs256(secrets: &[Vec<u8>], audience: &str, issuer: Option<&str>) -> Result<Self> {
        Ok(Self {
            validator: Arc::new(JwtValidator::new(secrets, audience, issuer)?),
        })
    }

    /// Rotate the HS256 secrets.
    pub fn rotate(&self, new: &[Vec<u8>]) -> Result<()> {
        self.validator.rotate_keys(new)
    }

    /// Validate a bearer token.
    pub fn validate(&self, token: &str) -> Result<Claims> {
        self.validator.validate(token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    fn sign(secret: &[u8]) -> String {
        let claims = Claims {
            iss: None,
            sub: "svc".into(),
            aud: "efe.v1".into(),
            exp: now_secs() + 60,
            iat: None,
            tenant_id: "t".into(),
            scopes: vec!["swarm:execute".into()],
        };
        encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(secret),
        )
        .unwrap()
    }

    #[test]
    fn store_validate_happy_path() {
        let s = b"sec".to_vec();
        let store = TokenStore::from_hs256(&[s.clone()], "efe.v1", None).unwrap();
        let token = sign(&s);
        let claims = store.validate(&token).unwrap();
        assert_eq!(claims.tenant_id, "t");
    }

    #[test]
    fn store_rotate_swaps_keys() {
        let old = b"old".to_vec();
        let new = b"new".to_vec();
        let store = TokenStore::from_hs256(&[old.clone()], "efe.v1", None).unwrap();
        store.rotate(&[new.clone()]).unwrap();
        assert!(store.validate(&sign(&old)).is_err());
        assert!(store.validate(&sign(&new)).is_ok());
    }
}
