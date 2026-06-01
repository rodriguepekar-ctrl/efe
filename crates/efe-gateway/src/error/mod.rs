// SPDX-License-Identifier: UNLICENSED
//! **Sprint 14 — N22** : RFC 7807 Problem Details error model — § 7.4
//! + ADR-3 + G7.
//!
//! Every 4xx/5xx response from the gateway is a `application/problem
//! +json` body matching the [RFC 7807] schema, extended with
//! `trace_id`.
//!
//! [RFC 7807]: https://datatracker.ietf.org/doc/html/rfc7807

pub mod problem_details;

pub use problem_details::ProblemDetails;

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// **N22** : the public error taxonomy.  Every handler returns
/// `Result<_, ApiError>` ; `IntoResponse` lifts to a Problem
/// Details 4xx/5xx body.
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
pub enum ApiError {
    // ─── 400 Bad Request ────────────────────────────────────────
    #[error("bad json : {0}")]
    BadJson(String),

    #[error("bad request : {0}")]
    BadRequest(String),

    // ─── 401 Unauthorized ───────────────────────────────────────
    #[error("missing authorization header")]
    MissingAuth,

    #[error("invalid jwt : {0}")]
    InvalidJwt(String),

    #[error("expired jwt")]
    ExpiredJwt,

    // ─── 403 Forbidden ──────────────────────────────────────────
    #[error("tenant isolation violation : jwt {jwt_tenant} vs body {body_tenant}")]
    TenantIsolationViolation {
        jwt_tenant: String,
        body_tenant: String,
    },

    #[error("operator scope required")]
    OperatorScopeRequired,

    // ─── 404 Not Found ──────────────────────────────────────────
    #[error("not found : {0}")]
    NotFound(String),

    // ─── 409 Conflict ───────────────────────────────────────────
    #[error("conflict : {0}")]
    Conflict(String),

    // ─── 413 Payload Too Large ──────────────────────────────────
    #[error("payload too large : {0} bytes (cap {1})")]
    PayloadTooLarge(usize, usize),

    // ─── 422 Unprocessable Entity ───────────────────────────────
    #[error("unprocessable : {0}")]
    Unprocessable(String),

    // ─── 429 Too Many Requests ──────────────────────────────────
    #[error("rate limited")]
    RateLimited,

    // ─── 500 Internal Server Error ──────────────────────────────
    #[error("internal : {0}")]
    Internal(String),

    // ─── 503 Service Unavailable ────────────────────────────────
    #[error("dependency unavailable : {0}")]
    DependencyUnavailable(String),

    // ─── 504 Gateway Timeout ────────────────────────────────────
    #[error("upstream timeout : {0}")]
    UpstreamTimeout(String),
}

impl ApiError {
    /// **N22** : HTTP status for this error.
    pub fn status(&self) -> StatusCode {
        match self {
            Self::BadJson(_) | Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::MissingAuth | Self::InvalidJwt(_) | Self::ExpiredJwt => StatusCode::UNAUTHORIZED,
            Self::TenantIsolationViolation { .. } | Self::OperatorScopeRequired => {
                StatusCode::FORBIDDEN
            }
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::PayloadTooLarge(_, _) => StatusCode::PAYLOAD_TOO_LARGE,
            Self::Unprocessable(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::DependencyUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::UpstreamTimeout(_) => StatusCode::GATEWAY_TIMEOUT,
        }
    }

    /// **N22** : RFC 7807 `type` URI fragment.
    pub fn type_slug(&self) -> &'static str {
        match self {
            Self::BadJson(_) => "bad-json",
            Self::BadRequest(_) => "bad-request",
            Self::MissingAuth => "missing-auth",
            Self::InvalidJwt(_) => "invalid-jwt",
            Self::ExpiredJwt => "expired-jwt",
            Self::TenantIsolationViolation { .. } => "tenant-isolation-violation",
            Self::OperatorScopeRequired => "operator-scope-required",
            Self::NotFound(_) => "not-found",
            Self::Conflict(_) => "conflict",
            Self::PayloadTooLarge(_, _) => "payload-too-large",
            Self::Unprocessable(_) => "unprocessable",
            Self::RateLimited => "rate-limited",
            Self::Internal(_) => "internal-server-error",
            Self::DependencyUnavailable(_) => "dependency-unavailable",
            Self::UpstreamTimeout(_) => "upstream-timeout",
        }
    }

    /// **N22** : RFC 7807 `title` (short human-readable).
    pub fn title(&self) -> &'static str {
        match self {
            Self::BadJson(_) => "Bad JSON",
            Self::BadRequest(_) => "Bad Request",
            Self::MissingAuth => "Missing Authorization",
            Self::InvalidJwt(_) => "Invalid JWT",
            Self::ExpiredJwt => "Expired JWT",
            Self::TenantIsolationViolation { .. } => "Tenant Isolation Violation",
            Self::OperatorScopeRequired => "Operator Scope Required",
            Self::NotFound(_) => "Not Found",
            Self::Conflict(_) => "Conflict",
            Self::PayloadTooLarge(_, _) => "Payload Too Large",
            Self::Unprocessable(_) => "Unprocessable Entity",
            Self::RateLimited => "Rate Limited",
            Self::Internal(_) => "Internal Server Error",
            Self::DependencyUnavailable(_) => "Dependency Unavailable",
            Self::UpstreamTimeout(_) => "Upstream Timeout",
        }
    }

    /// **N22** : Build the Problem Details body for this error.
    /// Caller provides the `trace_id` + `instance` (path).
    pub fn to_problem_details(&self, trace_id: &str, instance: &str) -> ProblemDetails {
        let mut pd = ProblemDetails::new(
            self.status().as_u16(),
            self.title(),
            self.type_slug(),
            instance,
            trace_id,
        );
        pd.detail = Some(self.to_string());
        // Add extensions for specific variants.
        if let Self::TenantIsolationViolation { jwt_tenant, body_tenant } = self {
            pd.extensions.insert(
                "jwt_tenant_id".into(),
                serde_json::Value::String(jwt_tenant.clone()),
            );
            pd.extensions.insert(
                "body_tenant_id".into(),
                serde_json::Value::String(body_tenant.clone()),
            );
        }
        if let Self::PayloadTooLarge(actual, cap) = self {
            pd.extensions
                .insert("size_bytes".into(), serde_json::json!(actual));
            pd.extensions
                .insert("cap_bytes".into(), serde_json::json!(cap));
        }
        pd
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        // Without a trace context, generate a placeholder.  Real
        // handlers should always have a trace_id via extension.
        let trace_id = "00000000000000000000000000000000".to_string();
        let pd = self.to_problem_details(&trace_id, "");
        pd.into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_codes_correct() {
        assert_eq!(
            ApiError::MissingAuth.status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            ApiError::TenantIsolationViolation {
                jwt_tenant: "a".into(),
                body_tenant: "b".into()
            }
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            ApiError::PayloadTooLarge(100, 50).status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        assert_eq!(
            ApiError::RateLimited.status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(
            ApiError::Internal("x".into()).status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            ApiError::DependencyUnavailable("x".into()).status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn type_slug_stable() {
        assert_eq!(ApiError::MissingAuth.type_slug(), "missing-auth");
        assert_eq!(ApiError::ExpiredJwt.type_slug(), "expired-jwt");
        assert_eq!(
            ApiError::TenantIsolationViolation {
                jwt_tenant: "a".into(),
                body_tenant: "b".into()
            }
            .type_slug(),
            "tenant-isolation-violation"
        );
    }

    #[test]
    fn problem_details_carries_extensions_for_tenant_violation() {
        let e = ApiError::TenantIsolationViolation {
            jwt_tenant: "acme".into(),
            body_tenant: "globex".into(),
        };
        let pd = e.to_problem_details("trace-xyz", "/v1/swarms/run");
        assert_eq!(pd.status, 403);
        assert_eq!(pd.trace_id, "trace-xyz");
        assert_eq!(pd.instance, "/v1/swarms/run");
        assert_eq!(
            pd.extensions.get("jwt_tenant_id"),
            Some(&serde_json::json!("acme"))
        );
        assert_eq!(
            pd.extensions.get("body_tenant_id"),
            Some(&serde_json::json!("globex"))
        );
    }

    #[test]
    fn problem_details_for_payload_too_large_carries_sizes() {
        let e = ApiError::PayloadTooLarge(20 * 1024 * 1024, 10 * 1024 * 1024);
        let pd = e.to_problem_details("t", "/x");
        assert_eq!(
            pd.extensions.get("size_bytes"),
            Some(&serde_json::json!(20 * 1024 * 1024))
        );
        assert_eq!(
            pd.extensions.get("cap_bytes"),
            Some(&serde_json::json!(10 * 1024 * 1024))
        );
    }

    #[test]
    fn into_response_uses_correct_status() {
        let e = ApiError::RateLimited;
        let resp = e.into_response();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    }
}
