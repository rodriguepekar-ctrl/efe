// SPDX-License-Identifier: UNLICENSED
//! **Sprint 14 — N22** : JWT claims schema.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// **N22** : the canonical claims EFE gateway requires.
///
/// Standard fields :
/// - `iss` (issuer) — `"efe-gateway"` or operator IdP.
/// - `sub` (subject) — opaque user id.
/// - `iat` (issued at) — unix seconds.
/// - `exp` (expiry) — unix seconds.
///
/// EFE-specific :
/// - `tenant_id` — UUID identifying the tenant ; REQUIRED on customer
///   tokens.  Operator tokens may set `tenant_id = nil`.
/// - `scope` — Vec<String> of scopes ; presence of `"operator"`
///   unlocks `/v1/admin/*` routes.
/// - `operator_id` — Optional UUID identifying the human/system
///   operator (audit trail).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GatewayClaims {
    pub iss: String,
    pub sub: String,
    pub iat: i64,
    pub exp: i64,
    pub tenant_id: Uuid,
    #[serde(default)]
    pub scope: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_id: Option<Uuid>,
}

impl GatewayClaims {
    /// **N22** : True if this token carries the `"operator"` scope.
    pub fn is_operator(&self) -> bool {
        self.scope.iter().any(|s| s == "operator")
    }

    /// **N22** : True if `tenant_id` is non-zero (i.e. a real tenant,
    /// not an operator-only token).
    pub fn has_tenant(&self) -> bool {
        self.tenant_id != Uuid::nil()
    }

    /// **N22** : Cheap UTC seconds.
    pub fn now_secs() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }
}

/// **N22** : declarative scope check helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatorScope {
    /// Any authenticated caller (operator scope NOT required).
    Tenant,
    /// `scope` MUST contain `"operator"`.
    Operator,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims(tenant: Uuid, scope: Vec<String>) -> GatewayClaims {
        GatewayClaims {
            iss: "efe-gateway".into(),
            sub: "alice@efe.ai".into(),
            iat: 1_700_000_000,
            exp: 1_700_003_600,
            tenant_id: tenant,
            scope,
            operator_id: None,
        }
    }

    #[test]
    fn is_operator_returns_true_when_scope_present() {
        let c = claims(Uuid::new_v4(), vec!["operator".into()]);
        assert!(c.is_operator());
    }

    #[test]
    fn is_operator_returns_false_when_scope_absent() {
        let c = claims(Uuid::new_v4(), vec!["read".into()]);
        assert!(!c.is_operator());
    }

    #[test]
    fn has_tenant_true_for_real_uuid() {
        let c = claims(Uuid::new_v4(), vec![]);
        assert!(c.has_tenant());
    }

    #[test]
    fn has_tenant_false_for_nil_uuid() {
        let c = claims(Uuid::nil(), vec![]);
        assert!(!c.has_tenant());
    }

    #[test]
    fn serde_round_trip() {
        let c = claims(Uuid::new_v4(), vec!["operator".into(), "read".into()]);
        let json = serde_json::to_string(&c).unwrap();
        let back: GatewayClaims = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn now_secs_is_positive() {
        let n = GatewayClaims::now_secs();
        assert!(n > 1_700_000_000);
    }
}
