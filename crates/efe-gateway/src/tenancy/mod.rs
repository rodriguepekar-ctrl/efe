// SPDX-License-Identifier: UNLICENSED
//! **Sprint 14 — N22** : tenant isolation helpers — § 7.4 + G6.
//!
//! Sprint 14 enforces tenant isolation at the handler layer via the
//! `check_tenant` helper called explicitly by handlers that take a
//! `tenant_id` in their body.  Sprint 15 moves to Postgres RLS for
//! defense-in-depth.

use uuid::Uuid;

use crate::auth::GatewayClaims;
use crate::error::ApiError;

/// **N22 + G6** : enforce `jwt.tenant_id == body.tenant_id`.
///
/// Operator tokens (with `scope=operator`) bypass the check — they
/// have implicit cross-tenant access.
pub fn check_tenant(claims: &GatewayClaims, body_tenant: Uuid) -> Result<(), ApiError> {
    if claims.is_operator() {
        return Ok(());
    }
    if claims.tenant_id == body_tenant {
        return Ok(());
    }
    Err(ApiError::TenantIsolationViolation {
        jwt_tenant: claims.tenant_id.to_string(),
        body_tenant: body_tenant.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims(tenant: Uuid, op: bool) -> GatewayClaims {
        GatewayClaims {
            iss: "x".into(),
            sub: "a".into(),
            iat: 0,
            exp: 1,
            tenant_id: tenant,
            scope: if op { vec!["operator".into()] } else { vec![] },
            operator_id: None,
        }
    }

    #[test]
    fn matching_tenant_ok() {
        let id = Uuid::new_v4();
        assert!(check_tenant(&claims(id, false), id).is_ok());
    }

    #[test]
    fn mismatched_tenant_rejected() {
        let err = check_tenant(&claims(Uuid::new_v4(), false), Uuid::new_v4()).unwrap_err();
        assert!(matches!(err, ApiError::TenantIsolationViolation { .. }));
    }

    #[test]
    fn operator_bypasses_check() {
        // Operator with nil tenant accessing tenant X.
        assert!(check_tenant(&claims(Uuid::nil(), true), Uuid::new_v4()).is_ok());
    }
}
