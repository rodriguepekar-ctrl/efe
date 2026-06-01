//! Shared extension types used by the auth guard and consumed by
//! service handlers + rate-limit helper.
//!
//! The gRPC-level interceptor pattern was replaced by per-handler
//! [`crate::auth::AuthGuard`] calls because `tonic::service::interceptor`
//! does not expose the RPC path — see `auth::guard` for the rationale.

use crate::auth::Claims;

/// Extension published by `AuthGuard::check` for handlers that want to
/// inspect the full authenticated claim set.
#[derive(Clone, Debug)]
pub struct AuthExtClaims(pub Claims);

/// Tenant id extracted from claims — the minimal subset that the
/// rate-limiter needs.
#[derive(Clone, Debug)]
pub struct RequestTenant(pub String);
