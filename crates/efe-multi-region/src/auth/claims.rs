// SPDX-License-Identifier: UNLICENSED
//! **Sprint 9 — N16** : Operator JWT claims.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use efe_nano_sdk::TenantId;

/// **Sprint 9 — N16** : Replay-relevant scopes carried by an operator JWT.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayScope {
    /// Operator can submit + cancel replays.
    #[serde(rename = "replay:execute")]
    Execute,
    /// Operator can list / read replay status + results.
    #[serde(rename = "replay:read")]
    Read,
}

impl ReplayScope {
    /// **N16** : Parse a JWT scope string into an enum.  Returns
    /// `None` for unrecognised scopes (forward-compatible).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "replay:execute" => Some(Self::Execute),
            "replay:read" => Some(Self::Read),
            _ => None,
        }
    }

    /// **N16** : Canonical wire-string label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Execute => "replay:execute",
            Self::Read => "replay:read",
        }
    }
}

/// **Sprint 9 — N16** : Decoded operator JWT claims.
///
/// All fields are mandatory ; missing fields → `JwtError::MalformedClaims`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OperatorClaims {
    /// Issuer — must be `"efe-operator-idp"` (or test issuer).
    pub iss: String,
    /// Subject — operator username / email.
    pub sub: String,
    /// Stable UUID identifying the operator account.
    pub operator_id: Uuid,
    /// Tenant the operator is acting on behalf of.
    pub tenant_id: TenantId,
    /// Granted replay scopes.
    pub scope: Vec<String>,
    /// Forensic ticket reference (e.g. `JIRA-12345`) — mandatory by
    /// architecture G14.
    pub ticket_id: String,
    /// Issued-at (epoch seconds).
    pub iat: i64,
    /// Expiry (epoch seconds).
    pub exp: i64,
    /// Unique JWT ID — used to fingerprint without storing the token.
    pub jti: String,
}

impl OperatorClaims {
    /// **N16** : True if the claims carry the given scope.
    pub fn has_scope(&self, scope: ReplayScope) -> bool {
        self.scope.iter().any(|s| s == scope.as_str())
    }

    /// **N16** : True if the JWT has expired against `now_epoch_seconds`.
    pub fn is_expired(&self, now_epoch_seconds: i64) -> bool {
        self.exp <= now_epoch_seconds
    }

    /// **N16** : Compute a stable SHA-256 fingerprint of the JWT
    /// identifier (`jti`).  Suitable for audit logs without storing
    /// the raw token.
    pub fn jwt_fingerprint(&self) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(self.jti.as_bytes());
        hasher.update([0u8]);
        hasher.update(self.iss.as_bytes());
        let digest = hasher.finalize();
        // 32-char hex prefix is enough for audit log linking.
        digest.iter().take(16).map(|b| format!("{b:02x}")).collect()
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_claims() -> OperatorClaims {
        OperatorClaims {
            iss: "efe-operator-idp".into(),
            sub: "alice@efe.com".into(),
            operator_id: Uuid::new_v4(),
            tenant_id: TenantId::new(),
            scope: vec!["replay:execute".into(), "replay:read".into()],
            ticket_id: "JIRA-12345".into(),
            iat: 1_716_200_000,
            exp: 1_716_203_600,
            jti: "abc-def-123".into(),
        }
    }

    #[test]
    fn replay_scope_as_str_stable() {
        assert_eq!(ReplayScope::Execute.as_str(), "replay:execute");
        assert_eq!(ReplayScope::Read.as_str(), "replay:read");
    }

    #[test]
    fn replay_scope_parse_round_trip() {
        for s in [ReplayScope::Execute, ReplayScope::Read] {
            assert_eq!(ReplayScope::parse(s.as_str()), Some(s));
        }
        assert_eq!(ReplayScope::parse("admin:foo"), None);
    }

    #[test]
    fn has_scope_finds_granted_scopes() {
        let claims = sample_claims();
        assert!(claims.has_scope(ReplayScope::Execute));
        assert!(claims.has_scope(ReplayScope::Read));
    }

    #[test]
    fn has_scope_returns_false_for_missing() {
        let mut claims = sample_claims();
        claims.scope = vec!["replay:read".into()];
        assert!(!claims.has_scope(ReplayScope::Execute));
        assert!(claims.has_scope(ReplayScope::Read));
    }

    #[test]
    fn is_expired_compares_exp() {
        let claims = sample_claims();
        assert!(claims.is_expired(claims.exp + 1));
        assert!(claims.is_expired(claims.exp));
        assert!(!claims.is_expired(claims.exp - 1));
    }

    #[test]
    fn jwt_fingerprint_stable_per_jti() {
        let claims = sample_claims();
        let f1 = claims.jwt_fingerprint();
        let f2 = claims.jwt_fingerprint();
        assert_eq!(f1, f2);
        assert!(!f1.is_empty());
        assert!(f1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn jwt_fingerprint_differs_with_jti() {
        let mut a = sample_claims();
        let mut b = sample_claims();
        a.jti = "first".into();
        b.jti = "second".into();
        assert_ne!(a.jwt_fingerprint(), b.jwt_fingerprint());
    }

    #[test]
    fn operator_claims_serde_round_trip() {
        let c = sample_claims();
        let json = serde_json::to_string(&c).unwrap();
        let c2: OperatorClaims = serde_json::from_str(&json).unwrap();
        assert_eq!(c, c2);
    }
}
