// SPDX-License-Identifier: UNLICENSED
//! **Sprint 14 — N22** : JWT validator (HS256 Sprint 14).
//!
//! Wraps `jsonwebtoken` crate.  Sprint 15 will add RS256+JWKS.

use async_trait::async_trait;
use jsonwebtoken::{decode, DecodingKey, EncodingKey, Validation};
use thiserror::Error;

use crate::auth::claims::GatewayClaims;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum JwtError {
    #[error("invalid signature or malformed token")]
    InvalidToken,

    #[error("expired")]
    Expired,

    #[error("issuer mismatch : expected {expected:?}, got {got:?}")]
    IssuerMismatch { expected: String, got: String },

    #[error("backend : {message}")]
    Backend { message: String },
}

#[async_trait]
pub trait JwtValidator: Send + Sync + 'static {
    async fn validate(&self, token: &str) -> Result<GatewayClaims, JwtError>;
}

/// **N22** : HS256 validator with a static shared secret.
///
/// Production deployments should use RS256+JWKS (Sprint 15).  HS256
/// is fine for single-tenant dev / test / CI where the secret never
/// leaves the deploy.
pub struct Hs256JwtValidator {
    secret: Vec<u8>,
    expected_issuer: Option<String>,
}

impl std::fmt::Debug for Hs256JwtValidator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hs256JwtValidator")
            .field("secret_len", &self.secret.len())
            .field("expected_issuer", &self.expected_issuer)
            .finish()
    }
}

impl Hs256JwtValidator {
    /// **N22** : Construct with the raw HMAC secret (≥ 32 bytes
    /// recommended).
    pub fn new(secret: impl Into<Vec<u8>>) -> Self {
        Self {
            secret: secret.into(),
            expected_issuer: None,
        }
    }

    /// **N22** : Require a specific `iss` claim.
    pub fn with_expected_issuer(mut self, iss: impl Into<String>) -> Self {
        self.expected_issuer = Some(iss.into());
        self
    }

    /// **N22 — test helper** : sign a `GatewayClaims` with this
    /// validator's secret.  Returns the signed JWT string.
    pub fn sign(&self, claims: &GatewayClaims) -> Result<String, JwtError> {
        let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
        let key = EncodingKey::from_secret(&self.secret);
        jsonwebtoken::encode(&header, claims, &key).map_err(|e| JwtError::Backend {
            message: e.to_string(),
        })
    }
}

#[async_trait]
impl JwtValidator for Hs256JwtValidator {
    async fn validate(&self, token: &str) -> Result<GatewayClaims, JwtError> {
        let key = DecodingKey::from_secret(&self.secret);
        let mut validation = Validation::new(jsonwebtoken::Algorithm::HS256);
        // Validate exp/iat by default ; we'll inspect issuer manually
        // (jsonwebtoken's `set_issuer` is built-in but we want a
        // clean dedicated error).
        validation.validate_exp = true;
        validation.validate_aud = false;
        validation.leeway = 5; // 5s clock skew tolerance
        let data = decode::<GatewayClaims>(token, &key, &validation).map_err(|e| {
            use jsonwebtoken::errors::ErrorKind as K;
            match e.kind() {
                K::ExpiredSignature => JwtError::Expired,
                K::InvalidSignature
                | K::InvalidToken
                | K::Base64(_)
                | K::Json(_)
                | K::Utf8(_)
                | K::InvalidEcdsaKey
                | K::InvalidRsaKey(_) => JwtError::InvalidToken,
                _ => JwtError::Backend {
                    message: e.to_string(),
                },
            }
        })?;
        if let Some(expected) = &self.expected_issuer {
            if &data.claims.iss != expected {
                return Err(JwtError::IssuerMismatch {
                    expected: expected.clone(),
                    got: data.claims.iss.clone(),
                });
            }
        }
        Ok(data.claims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn claims(now: i64, exp_in: i64, iss: &str) -> GatewayClaims {
        GatewayClaims {
            iss: iss.into(),
            sub: "alice".into(),
            iat: now - 60,
            exp: now + exp_in,
            tenant_id: Uuid::new_v4(),
            scope: vec!["read".into()],
            operator_id: None,
        }
    }

    #[tokio::test]
    async fn sign_then_validate_round_trip() {
        let v = Hs256JwtValidator::new(b"a-very-strong-secret-of-32-bytes!!".to_vec());
        let c = claims(GatewayClaims::now_secs(), 3_600, "efe-gateway");
        let token = v.sign(&c).unwrap();
        let back = v.validate(&token).await.unwrap();
        assert_eq!(back.sub, "alice");
    }

    #[tokio::test]
    async fn expired_token_rejected() {
        let v = Hs256JwtValidator::new(b"strong-secret-of-32-bytes-please!!".to_vec());
        let c = claims(GatewayClaims::now_secs(), -3_600, "efe-gateway"); // expired
        let token = v.sign(&c).unwrap();
        let err = v.validate(&token).await.unwrap_err();
        assert_eq!(err, JwtError::Expired);
    }

    #[tokio::test]
    async fn wrong_secret_rejected() {
        let v1 = Hs256JwtValidator::new(b"secret-one-padded-to-32-bytes-ok!!".to_vec());
        let v2 = Hs256JwtValidator::new(b"secret-two-padded-to-32-bytes-ok!!".to_vec());
        let c = claims(GatewayClaims::now_secs(), 3_600, "efe-gateway");
        let token = v1.sign(&c).unwrap();
        let err = v2.validate(&token).await.unwrap_err();
        assert_eq!(err, JwtError::InvalidToken);
    }

    #[tokio::test]
    async fn malformed_token_rejected() {
        let v = Hs256JwtValidator::new(b"x".repeat(32));
        let err = v.validate("not.a.jwt").await.unwrap_err();
        assert_eq!(err, JwtError::InvalidToken);
    }

    #[tokio::test]
    async fn issuer_pin_enforced_match() {
        let v = Hs256JwtValidator::new(b"x".repeat(32))
            .with_expected_issuer("efe-gateway");
        let c = claims(GatewayClaims::now_secs(), 3_600, "efe-gateway");
        let token = v.sign(&c).unwrap();
        assert!(v.validate(&token).await.is_ok());
    }

    #[tokio::test]
    async fn issuer_pin_enforced_mismatch() {
        let v = Hs256JwtValidator::new(b"x".repeat(32))
            .with_expected_issuer("efe-gateway");
        let c = claims(GatewayClaims::now_secs(), 3_600, "other-issuer");
        let token = v.sign(&c).unwrap();
        let err = v.validate(&token).await.unwrap_err();
        assert!(matches!(err, JwtError::IssuerMismatch { .. }));
    }

    #[test]
    fn debug_format_does_not_leak_secret() {
        let v = Hs256JwtValidator::new(b"ultra-secret-key-never-log-me!!!!".to_vec());
        let dbg = format!("{v:?}");
        assert!(!dbg.contains("ultra-secret-key"));
        assert!(dbg.contains("secret_len"));
    }

    #[tokio::test]
    async fn small_clock_skew_tolerated() {
        let v = Hs256JwtValidator::new(b"x".repeat(32));
        let c = claims(GatewayClaims::now_secs(), -2, "efe-gateway"); // 2s past
        let token = v.sign(&c).unwrap();
        // Leeway is 5s, so should still pass.
        assert!(v.validate(&token).await.is_ok());
    }
}
