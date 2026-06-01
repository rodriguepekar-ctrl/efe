// SPDX-License-Identifier: UNLICENSED
//! # `efe-gateway` — M50 Sprint 14 N22 HTTP gateway.
//!
//! See `docs/architecture/EFE_M50_N22_HTTP_GATEWAY_EXPOSURE_ARCHITECTURE.md`.
//!
//! # Module map
//!
//! | Module | Purpose |
//! |---|---|
//! | [`config`] | `GatewayConfig` from env vars + TOML. |
//! | [`state`] | `AppState` holding subsystem `Arc<dyn ...>` deps. |
//! | [`error`] | RFC 7807 Problem Details + subsystem-error conversions. |
//! | [`auth`] | JWT bearer auth (HS256 default ; Sprint 14). |
//! | [`tenancy`] | `tenant_id` JWT-vs-body enforcement. |
//! | [`tracing_layer`] | W3C Trace Context + request_id. |
//! | [`observability`] | `/healthz`, `/readyz`, `/metrics`. |
//! | [`handlers`] | 21 endpoint handlers. |
//! | [`router`] | `RouterBuilder` that assembles everything. |
//! | [`shutdown`] | SIGTERM/SIGINT graceful drain. |

#![deny(unsafe_code)]
#![warn(rust_2018_idioms)]

pub mod auth;
pub mod config;
pub mod dag_registry;
pub mod error;
pub mod handlers;
pub mod observability;
pub mod router;
pub mod shutdown;
pub mod state;
pub mod tenancy;
pub mod tracing_layer;

pub use auth::{AuthConfig, GatewayClaims, Hs256JwtValidator, JwtValidator};
pub use config::GatewayConfig;
pub use error::{ApiError, ProblemDetails};
pub use observability::N22GatewayMetrics;
pub use router::{build_router, RouterBuilder};
pub use shutdown::shutdown_signal;
pub use state::AppState;

/// **N22** : Crate semver.
pub const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// **N22** : Default body size cap (10 MB).
pub const DEFAULT_BODY_LIMIT_BYTES: usize = 10 * 1024 * 1024;

/// **N22** : Default graceful shutdown budget (30 seconds).
pub const DEFAULT_GRACEFUL_SHUTDOWN_SECS: u64 = 30;

/// **N22** : Default per-tenant rate limit (100 req/s).
pub const DEFAULT_RATE_LIMIT_RPS: u32 = 100;
