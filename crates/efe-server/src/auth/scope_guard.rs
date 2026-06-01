//! Sprint **D22** (M4 § 10) — Per-RPC scope guard.
//!
//! ## Why a separate guard
//!
//! Pre-D22, scope enforcement was done by the higher-level
//! [`AuthGuard::check`] which takes a single statically-known scope
//! string. That works for the M9 cone-scoped surface (each RPC has
//! exactly one required scope) but the M4 GA targets a 3-scope hierarchy
//! (`data` / `admin` / `audit`) layered ON TOP of the existing
//! granular scopes (`swarm:execute`, `admin:catalog:read`, …).
//!
//! Concretely : `Mode1.Execute` requires the granular `swarm:execute`
//! scope (D9 contract) AND the broad `data` scope (D22 contract). A
//! handler that wants to enforce both opens a hole if it only checks
//! one.
//!
//! [`ScopeGuard`] addresses this by providing a uniform helper —
//! `ScopeGuard::require(&claims, scope)` — that handlers call as the
//! **first** line after `AuthGuard::check`. The pre-audit pattern was
//! to slip the broad scope into [`required_scope`] but that conflates
//! the granular and the broad scopes into one map and complicates the
//! D9-era tests.
//!
//! ## Audit fix S-A2 (recurring "wiring forgotten" pattern)
//!
//! The fix S-A2 documented in M4 § 10.3 surfaces the same recurring
//! defect we've seen in D16-C1 / D13bis-A20 / D17-AC1 / D18-A1 / D18-A3
//! / D28-A3 / K-A1 / F-A1 : a new auth dimension was declared in the
//! design (3-scope model) but no enforcement was wired into the handler
//! path. A pin test in `scripts/tests/test_d22_mtls_scopes.py`
//! verifies that **every** Admin / Mode-1/2/3 / Catalog handler calls
//! `ScopeGuard::require` exactly once.

use tonic::Status;

use crate::auth::Claims;

/// **Three canonical broad scopes** declared by the M4 architecture.
///
/// These are independent of the existing granular scopes
/// (`swarm:execute`, `admin:cone:*`, `admin:catalog:*`). A JWT typically
/// carries both : e.g. `"scopes": ["data", "swarm:execute"]` for a
/// data-plane principal, or `"scopes": ["admin", "admin:cone:write"]`
/// for a cone admin.
pub struct BroadScope;

impl BroadScope {
    /// Permits Mode-1/2/3 RPCs + CatalogService 4 RPCs.
    pub const DATA: &'static str = "data";

    /// Permits AdminService write RPCs (12 at GA).
    pub const ADMIN: &'static str = "admin";

    /// Permits AdminService read-only RPCs (BreachHistory +
    /// ListAuditEvents). Cannot perform writes nor data-plane calls.
    pub const AUDIT: &'static str = "audit";
}

/// Per-RPC scope guard.
///
/// Use the static helper [`ScopeGuard::require`] — there is intentionally
/// no struct state. The type exists purely to give handlers a clear
/// call-site (`ScopeGuard::require(...)?`) that grep'able pin tests can
/// detect.
pub struct ScopeGuard;

impl ScopeGuard {
    /// Require that `claims.scopes` contains the given token.
    ///
    /// Returns [`tonic::Status::permission_denied`] otherwise.
    ///
    /// # Implementation note
    ///
    /// We deliberately do NOT space-split a `claims.scope` string.
    /// `Claims` already deserialises into `scopes: Vec<String>`
    /// (cf. [`crate::auth::Claims`]), so callers don't have to care
    /// whether the JWT used the RFC 6749 space-delimited form or a
    /// JSON array. The OIDC validator handles both via serde.
    ///
    /// # Errors
    ///
    /// Returns [`tonic::Status::permission_denied`] when the scope is
    /// missing. The error message includes the missing scope so
    /// operators can correlate denied requests to misconfigured client
    /// credentials. PII-free : we do NOT log the full scope set.
    pub fn require(claims: &Claims, scope: &str) -> Result<(), Status> {
        if claims.has_scope(scope) {
            return Ok(());
        }
        tracing::warn!(
            target: "efe.auth.scope",
            tenant_id = %claims.tenant_id,
            requested = scope,
            "scope check failed"
        );
        Err(Status::permission_denied(format!(
            "missing required scope: {scope}"
        )))
    }

    /// Require ANY of `scopes` (logical OR).
    ///
    /// Use case : a handler that accepts either the granular scope
    /// (`admin:catalog:read`) OR the broad audit scope (`audit`).
    ///
    /// # Errors
    ///
    /// Returns [`tonic::Status::permission_denied`] when none of the
    /// requested scopes is granted. The error message lists the
    /// candidates.
    pub fn require_any(claims: &Claims, scopes: &[&str]) -> Result<(), Status> {
        if scopes.iter().any(|s| claims.has_scope(s)) {
            return Ok(());
        }
        tracing::warn!(
            target: "efe.auth.scope",
            tenant_id = %claims.tenant_id,
            candidates = ?scopes,
            "no candidate scope granted"
        );
        Err(Status::permission_denied(format!(
            "missing one of: {}",
            scopes.join(", ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims_with(scopes: &[&str]) -> Claims {
        Claims {
            iss: None,
            sub: "svc".into(),
            aud: "efe.v1".into(),
            exp: u64::MAX,
            iat: None,
            tenant_id: "acme".into(),
            scopes: scopes.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    #[test]
    fn require_data_pass() {
        let c = claims_with(&[BroadScope::DATA]);
        ScopeGuard::require(&c, BroadScope::DATA).unwrap();
    }

    #[test]
    fn require_admin_missing_returns_permission_denied() {
        let c = claims_with(&[BroadScope::DATA]);
        let err = ScopeGuard::require(&c, BroadScope::ADMIN).unwrap_err();
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert!(err.message().contains("admin"));
    }

    #[test]
    fn require_audit_separate_from_admin() {
        // A pure-audit principal (read-only) must NOT pass an admin check.
        let c = claims_with(&[BroadScope::AUDIT]);
        let err = ScopeGuard::require(&c, BroadScope::ADMIN).unwrap_err();
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
    }

    #[test]
    fn require_any_pass_with_first() {
        let c = claims_with(&[BroadScope::DATA]);
        ScopeGuard::require_any(&c, &[BroadScope::DATA, BroadScope::ADMIN]).unwrap();
    }

    #[test]
    fn require_any_pass_with_second() {
        let c = claims_with(&[BroadScope::ADMIN]);
        ScopeGuard::require_any(&c, &[BroadScope::DATA, BroadScope::ADMIN]).unwrap();
    }

    #[test]
    fn require_any_none_granted_denies() {
        let c = claims_with(&["something:else"]);
        let err = ScopeGuard::require_any(&c, &[BroadScope::ADMIN, BroadScope::AUDIT]).unwrap_err();
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
    }

    #[test]
    fn three_broad_scopes_are_distinct() {
        assert_ne!(BroadScope::DATA, BroadScope::ADMIN);
        assert_ne!(BroadScope::ADMIN, BroadScope::AUDIT);
        assert_ne!(BroadScope::DATA, BroadScope::AUDIT);
    }
}
