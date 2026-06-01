//! Request-scoped shared types and the in-handler rate-limit helper.
//!
//! The original tonic interceptor design for auth has been removed in
//! favour of [`crate::auth::AuthGuard`] (see that module's docs for
//! the rationale — tonic strips the RPC path before interceptors run,
//! preventing per-RPC scope enforcement). What remains here is the
//! small set of extension markers populated by the guard and the
//! tenant-aware rate-limit helper.

pub mod auth;
pub mod ratelimit;

pub use auth::{AuthExtClaims, RequestTenant};
