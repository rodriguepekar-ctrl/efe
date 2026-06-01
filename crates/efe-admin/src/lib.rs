//! Library entry point for `efe-admin`.
//!
//! Exposes parsing, context, error types and command dispatch as a
//! library to enable integration testing. Binary `main.rs` is a thin
//! wrapper around these.
//!
//! Cf. `docs/EFE_GO_LIVE_ONBOARDING_ARCHITECTURE.md` § 3.

#![allow(missing_docs)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::missing_errors_doc)]

pub mod auth;
pub mod cli;
pub mod commands;
pub mod config;
pub mod context;
pub mod error;
pub mod grpc;
pub mod mcp;
pub mod output;
pub mod prompts;
pub mod validation;

pub use error::AdminError;

/// Crate version, baked at compile time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Human-readable banner shown by `efe-admin version`.
pub fn banner() -> String {
    format!(
        "efe-admin v{} ({} {})",
        VERSION,
        std::env::consts::OS,
        std::env::consts::ARCH,
    )
}
