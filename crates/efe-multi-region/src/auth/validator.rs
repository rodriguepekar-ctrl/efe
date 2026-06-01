// SPDX-License-Identifier: UNLICENSED
//! **Sprint 9 — N16** : JWT validator (HS256).
//!
//! Validates a 3-part JWT (`header.payload.signature`) :
//! 1. base64-decode header, payload, signature.
//! 2. recompute HMAC-SHA256 over `header.payload`, compare to signature.
//! 3. deserialize payload into [`OperatorClaims`].
//! 4. check expiry against current time.
//!
//! The implementation is intentionally minimal — no external
//! `jsonwebtoken` dependency, just `hmac` + `sha2` + `base64`.  Suitable
//! for the Sprint 9 ship ; Sprint 10 will move to RS256 + Vault.

use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use thiserror::Error;

use super::claims::OperatorClaims;

type HmacSha256 = Hmac<Sha256>;

// ─────────────────────────────────────────────────────────────────────
//  JwtError
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N16** : Closed taxonomy of JWT validation errors.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum JwtError {
    /// Token is not three dot-separated segments.
    #[error("malformed token : expected 3 segments, got {0}")]
    MalformedToken(usize),

    /// Header / payload / signature segment is not valid base64.
    #[error("base64 decode failed : {0}")]
    Base64(String),

    /// Header doesn't deserialize to expected JSON.
    #[error("header malformed : {0}")]
    MalformedHeader(String),

    /// Payload doesn't deserialize to [`OperatorClaims`].
    #[error("claims malformed : {0}")]
    MalformedClaims(String),

    /// Header `alg` is not `HS256`.
    #[error("unsupported algorithm : {0}")]
    UnsupportedAlgorithm(String),

    /// HMAC signature verification failed.
    #[error("signature mismatch")]
    SignatureMismatch,

    /// `exp` claim is in the past.
    #[error("token expired (exp={exp}, now={now})")]
    Expired { exp: i64, now: i64 },

    /// `iat` claim is in the future (clock skew).
    #[error("token issued in the future (iat={iat}, now={now})")]
    IssuedInFuture { iat: i64, now: i64 },

    /// Internal error during validation.
    #[error("internal : {0}")]
    Internal(String),
}

// ─────────────────────────────────────────────────────────────────────
//  JwtValidator trait
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N16** : Validate a bearer JWT and return its claims.
#[async_trait]
pub trait JwtValidator: Send + Sync + 'static {
    /// **N16** : Validate the token and return the operator claims.
    ///
    /// Implementations MUST :
    /// - Verify signature.
    /// - Reject expired tokens.
    /// - Reject tokens issued in the future (clock skew tolerance ≤ 60s).
    async fn validate(&self, token: &str) -> Result<OperatorClaims, JwtError>;
}

// ─────────────────────────────────────────────────────────────────────
//  Hs256JwtValidator — shared-secret HS256 implementation
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N16** : HS256 JWT validator backed by a shared secret.
///
/// Operationally simple ; suitable for Sprint 9 ship.  Sprint 10 will
/// add an RS256 / Vault Transit implementation.
pub struct Hs256JwtValidator {
    secret: Vec<u8>,
    /// Clock-skew tolerance (seconds) for `iat` checks.  Default 60.
    clock_skew_tolerance_seconds: i64,
    /// Optional issuer pin.  When `Some`, tokens with a different
    /// `iss` are rejected.
    expected_issuer: Option<String>,
}

impl std::fmt::Debug for Hs256JwtValidator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hs256JwtValidator")
            .field("secret_len", &self.secret.len())
            .field("clock_skew_tolerance_seconds", &self.clock_skew_tolerance_seconds)
            .field("expected_issuer", &self.expected_issuer)
            .finish()
    }
}

impl Hs256JwtValidator {
    /// **N16** : Construct a validator with the given shared secret.
    /// `secret.len() >= 32` is enforced for cryptographic safety.
    ///
    /// # Panics
    ///
    /// Panics if `secret.len() < 32`.
    pub fn new(secret: impl Into<Vec<u8>>) -> Self {
        let secret = secret.into();
        assert!(
            secret.len() >= 32,
            "Hs256JwtValidator secret must be ≥ 32 bytes (got {})",
            secret.len()
        );
        Self {
            secret,
            clock_skew_tolerance_seconds: 60,
            expected_issuer: None,
        }
    }

    /// **N16** : Pin the expected issuer.  Tokens with a different
    /// `iss` claim are rejected.
    pub fn with_expected_issuer(mut self, issuer: impl Into<String>) -> Self {
        self.expected_issuer = Some(issuer.into());
        self
    }

    /// **N16** : Override the clock-skew tolerance (default 60 s).
    pub fn with_clock_skew_tolerance(mut self, tolerance_seconds: i64) -> Self {
        self.clock_skew_tolerance_seconds = tolerance_seconds.max(0);
        self
    }

    /// **N16** : Sign + encode a `OperatorClaims` payload into a JWT
    /// string.  Used by tests + dev-mode token minting.
    pub fn sign(&self, claims: &OperatorClaims) -> Result<String, JwtError> {
        let header = serde_json::json!({"alg": "HS256", "typ": "JWT"});
        let header_b64 = URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&header).map_err(|e| JwtError::Internal(e.to_string()))?);
        let payload_b64 = URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(claims).map_err(|e| JwtError::Internal(e.to_string()))?);
        let signing_input = format!("{header_b64}.{payload_b64}");
        let mut mac = HmacSha256::new_from_slice(&self.secret)
            .map_err(|e| JwtError::Internal(e.to_string()))?;
        mac.update(signing_input.as_bytes());
        let sig = mac.finalize().into_bytes();
        let sig_b64 = URL_SAFE_NO_PAD.encode(sig);
        Ok(format!("{signing_input}.{sig_b64}"))
    }

    /// Decode a base64url segment into bytes.
    fn decode_segment(segment: &str) -> Result<Vec<u8>, JwtError> {
        URL_SAFE_NO_PAD
            .decode(segment)
            .map_err(|e| JwtError::Base64(e.to_string()))
    }

    fn now_epoch_seconds() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }
}

#[async_trait]
impl JwtValidator for Hs256JwtValidator {
    async fn validate(&self, token: &str) -> Result<OperatorClaims, JwtError> {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err(JwtError::MalformedToken(parts.len()));
        }
        let (header_b64, payload_b64, sig_b64) = (parts[0], parts[1], parts[2]);

        // 1. Header check : alg = HS256.
        let header_bytes = Self::decode_segment(header_b64)?;
        let header: serde_json::Value = serde_json::from_slice(&header_bytes)
            .map_err(|e| JwtError::MalformedHeader(e.to_string()))?;
        let alg = header
            .get("alg")
            .and_then(|v| v.as_str())
            .ok_or_else(|| JwtError::MalformedHeader("alg field missing".into()))?;
        if alg != "HS256" {
            return Err(JwtError::UnsupportedAlgorithm(alg.into()));
        }

        // 2. Signature verification.
        let signing_input = format!("{header_b64}.{payload_b64}");
        let sig_bytes = Self::decode_segment(sig_b64)?;
        let mut mac = HmacSha256::new_from_slice(&self.secret)
            .map_err(|e| JwtError::Internal(e.to_string()))?;
        mac.update(signing_input.as_bytes());
        mac.verify_slice(&sig_bytes)
            .map_err(|_| JwtError::SignatureMismatch)?;

        // 3. Payload decode.
        let payload_bytes = Self::decode_segment(payload_b64)?;
        let claims: OperatorClaims = serde_json::from_slice(&payload_bytes)
            .map_err(|e| JwtError::MalformedClaims(e.to_string()))?;

        // 4. Expiry + clock-skew check.
        let now = Self::now_epoch_seconds();
        if claims.is_expired(now) {
            return Err(JwtError::Expired {
                exp: claims.exp,
                now,
            });
        }
        if claims.iat > now + self.clock_skew_tolerance_seconds {
            return Err(JwtError::IssuedInFuture {
                iat: claims.iat,
                now,
            });
        }

        // 5. Issuer pin.
        if let Some(expected) = &self.expected_issuer {
            if claims.iss != *expected {
                return Err(JwtError::MalformedClaims(format!(
                    "issuer mismatch : expected {expected}, got {}",
                    claims.iss
                )));
            }
        }

        Ok(claims)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::TenantId;
    use uuid::Uuid;

    fn validator() -> Hs256JwtValidator {
        Hs256JwtValidator::new([42u8; 32])
    }

    fn now_seconds() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    fn fresh_claims() -> OperatorClaims {
        OperatorClaims {
            iss: "efe-operator-idp".into(),
            sub: "alice@efe.com".into(),
            operator_id: Uuid::new_v4(),
            tenant_id: TenantId::new(),
            scope: vec!["replay:execute".into()],
            ticket_id: "JIRA-1".into(),
            iat: now_seconds() - 60,
            exp: now_seconds() + 3_600,
            jti: "test-jti".into(),
        }
    }

    #[test]
    #[should_panic]
    fn validator_rejects_short_secret() {
        let _ = Hs256JwtValidator::new(b"too-short".to_vec());
    }

    #[tokio::test]
    async fn sign_validate_round_trip() {
        let v = validator();
        let claims = fresh_claims();
        let token = v.sign(&claims).unwrap();
        let decoded = v.validate(&token).await.unwrap();
        assert_eq!(decoded, claims);
    }

    #[tokio::test]
    async fn malformed_token_returns_typed_error() {
        let v = validator();
        let err = v.validate("not-a-jwt").await.unwrap_err();
        assert!(matches!(err, JwtError::MalformedToken(_)));
    }

    #[tokio::test]
    async fn token_with_wrong_signature_rejected() {
        let v = validator();
        let claims = fresh_claims();
        let mut token = v.sign(&claims).unwrap();
        // Corrupt the signature segment.
        let len = token.len();
        token.replace_range((len - 4)..len, "AAAA");
        let err = v.validate(&token).await.unwrap_err();
        assert!(matches!(err, JwtError::SignatureMismatch));
    }

    #[tokio::test]
    async fn expired_token_rejected() {
        let v = validator();
        let mut claims = fresh_claims();
        claims.exp = now_seconds() - 1; // already expired
        claims.iat = now_seconds() - 3_600;
        let token = v.sign(&claims).unwrap();
        let err = v.validate(&token).await.unwrap_err();
        assert!(matches!(err, JwtError::Expired { .. }));
    }

    #[tokio::test]
    async fn token_issued_in_future_rejected() {
        let v = validator();
        let mut claims = fresh_claims();
        claims.iat = now_seconds() + 600; // 10 min in future ; beyond 60s tolerance
        claims.exp = now_seconds() + 3_600;
        let token = v.sign(&claims).unwrap();
        let err = v.validate(&token).await.unwrap_err();
        assert!(matches!(err, JwtError::IssuedInFuture { .. }));
    }

    #[tokio::test]
    async fn issuer_pin_enforced() {
        let v = validator().with_expected_issuer("efe-operator-idp");
        let mut claims = fresh_claims();
        claims.iss = "rogue-idp".into();
        let token = v.sign(&claims).unwrap();
        let err = v.validate(&token).await.unwrap_err();
        assert!(matches!(err, JwtError::MalformedClaims(_)));
    }

    #[tokio::test]
    async fn issuer_pin_allows_matching() {
        let v = validator().with_expected_issuer("efe-operator-idp");
        let claims = fresh_claims();
        let token = v.sign(&claims).unwrap();
        assert!(v.validate(&token).await.is_ok());
    }

    #[tokio::test]
    async fn different_secret_rejects_token() {
        let v1 = Hs256JwtValidator::new([1u8; 32]);
        let v2 = Hs256JwtValidator::new([2u8; 32]);
        let claims = fresh_claims();
        let token = v1.sign(&claims).unwrap();
        let err = v2.validate(&token).await.unwrap_err();
        assert!(matches!(err, JwtError::SignatureMismatch));
    }

    #[tokio::test]
    async fn wrong_algorithm_in_header_rejected() {
        // Manually craft a token with alg=none.
        let header = serde_json::json!({"alg": "none", "typ": "JWT"});
        let claims = fresh_claims();
        let h_b64 = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).unwrap());
        let p_b64 = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap());
        let sig_b64 = URL_SAFE_NO_PAD.encode(b"");
        let token = format!("{h_b64}.{p_b64}.{sig_b64}");
        let v = validator();
        let err = v.validate(&token).await.unwrap_err();
        assert!(matches!(err, JwtError::UnsupportedAlgorithm(_)));
    }

    #[tokio::test]
    async fn clock_skew_tolerance_within_bounds_accepts_token() {
        let v = validator().with_clock_skew_tolerance(120);
        let mut claims = fresh_claims();
        claims.iat = now_seconds() + 30; // within 120s tolerance
        let token = v.sign(&claims).unwrap();
        assert!(v.validate(&token).await.is_ok());
    }

    #[tokio::test]
    async fn debug_format_safe_no_secret_leak() {
        let v = validator();
        let s = format!("{v:?}");
        // Secret should NOT appear in debug output ; only its length.
        assert!(s.contains("secret_len"));
        assert!(!s.contains("[42, 42"));
    }
}
