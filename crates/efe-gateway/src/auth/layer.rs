// SPDX-License-Identifier: UNLICENSED
//! **Sprint 14 — N22** : Auth middleware extractor.
//!
//! Sprint 14 ships a lightweight axum extractor pattern (not a
//! full Tower service layer) — handlers declare
//! `Claims(claims): Claims` in their signature and the extractor
//! does the validation + 401.  This keeps the surface simple and
//! testable.


use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts},
};

use crate::auth::claims::GatewayClaims;
use crate::auth::jwt::JwtError;
use crate::error::ApiError;
use crate::state::AppState;

/// **N22** : the authenticated claims handed to authenticated
/// handlers via the axum extractor pattern.
///
/// ```ignore
/// async fn my_handler(
///     Claims(claims): Claims,
///     /* ... */
/// ) { /* ... */ }
/// ```
#[derive(Debug, Clone)]
pub struct Claims(pub GatewayClaims);

#[axum::async_trait]
impl FromRequestParts<AppState> for Claims {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let auth_header = parts
            .headers
            .get(header::AUTHORIZATION)
            .ok_or(ApiError::MissingAuth)?
            .to_str()
            .map_err(|_| ApiError::InvalidJwt("non-ascii auth header".into()))?;

        let token = auth_header
            .strip_prefix("Bearer ")
            .ok_or_else(|| ApiError::InvalidJwt("missing 'Bearer ' prefix".into()))?;

        let claims = state
            .jwt_validator
            .validate(token)
            .await
            .map_err(|e| match e {
                JwtError::Expired => ApiError::ExpiredJwt,
                JwtError::IssuerMismatch { .. } => {
                    ApiError::InvalidJwt(format!("{e}"))
                }
                _ => ApiError::InvalidJwt(format!("{e}")),
            })?;

        Ok(Claims(claims))
    }
}

/// **N22** : tower-style auth layer (stub) — Sprint 14 wires via
/// the extractor pattern above.  This struct exists so the doc's
/// AuthLayer reference resolves.
#[derive(Debug, Clone, Default)]
pub struct AuthLayer;

impl AuthLayer {
    pub fn new() -> Self {
        Self
    }
}

/// **N22 — test helper** : convenient claims for operator scope.
pub fn operator_only_or_403(claims: &GatewayClaims) -> Result<(), ApiError> {
    if claims.is_operator() {
        Ok(())
    } else {
        Err(ApiError::OperatorScopeRequired)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op_claims() -> GatewayClaims {
        GatewayClaims {
            iss: "x".into(),
            sub: "alice".into(),
            iat: 0,
            exp: 1,
            tenant_id: uuid::Uuid::nil(),
            scope: vec!["operator".into()],
            operator_id: None,
        }
    }

    fn tenant_claims() -> GatewayClaims {
        GatewayClaims {
            iss: "x".into(),
            sub: "alice".into(),
            iat: 0,
            exp: 1,
            tenant_id: uuid::Uuid::new_v4(),
            scope: vec!["read".into()],
            operator_id: None,
        }
    }

    #[test]
    fn operator_check_accepts_operator() {
        assert!(operator_only_or_403(&op_claims()).is_ok());
    }

    #[test]
    fn operator_check_rejects_tenant() {
        let err = operator_only_or_403(&tenant_claims()).unwrap_err();
        assert!(matches!(err, ApiError::OperatorScopeRequired));
    }

    #[test]
    fn layer_default_constructs() {
        let _ = AuthLayer;
        let _ = AuthLayer::new();
    }

    #[test]
    fn missing_auth_status_is_401() {
        use axum::http::StatusCode;
        assert_eq!(ApiError::MissingAuth.status(), StatusCode::UNAUTHORIZED);
    }
}
