//! `efe-meta-swarm-server` — Tonic-based gRPC servers for the EFE
//! meta-swarm coordinator.
//!
//! This crate ships **two binaries** under one Cargo crate :
//!
//!   * `controller` — implements `MetaSwarmService` ; called by
//!     `efe-server` when `EFE_META_SWARM_ENABLED=true`.
//!   * `participant` — implements `MetaParticipantService` ; called
//!     by the controller.  Three instances run side by side in the
//!     reference Phase-2 topology (T2 single-node).
//!
//! See [`docs/EFE_A1_PHASE2_ARCHITECTURE.md`][doc] for the full
//! design.
//!
//! [doc]: https://example.invalid/docs/EFE_A1_PHASE2_ARCHITECTURE.md
//!
//! # Layout
//!
//!   * [`config`]        — env-var parsing (one struct per role)
//!   * [`manifest`]      — YAML manifest reader (stub in PR #6)
//!   * [`discovery`]     — `trait Discovery` + impls (Static only in PR #6)
//!   * [`observability`] — health, metrics, tracing, shutdown
//!   * [`auth`]          — `trait Authorizer` + stubs (real in PR #16)
//!   * [`controller`]    — `MetaSwarmService` impl
//!   * [`participant`]   — `MetaParticipantService` impl
//!   * [`error`]         — crate-wide error type

#![forbid(unsafe_code)]
#![deny(missing_debug_implementations)]
#![warn(rust_2018_idioms)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::must_use_candidate)]

pub mod auth;
pub mod client;
pub mod config;
pub mod controller;
pub mod discovery;
pub mod error;
pub mod manifest;
pub mod observability;
pub mod participant;
pub mod tls;
pub mod tracing_otel;

/// Re-export the proto crate's surface under our own namespace so
/// downstream callers can write `efe_meta_swarm_server::proto::Foo`
/// without depending on `efe-meta-swarm-proto` directly.
pub mod proto {
    pub use efe_meta_swarm_proto::{
        controller::{MetaSwarmService, MetaSwarmServiceServer},
        meta_swarm,
        participant::{MetaParticipantService, MetaParticipantServiceServer},
        PROTO_PACKAGE, PROTO_VERSION, RPC_NAMES,
    };
}

/// The crate version, taken from `Cargo.toml`.  Used in
/// `InfoResponse.build_sha` + Prometheus `meta_swarm_build_info`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The git short SHA at compile time.  Set by `build.rs`.
/// Falls back to `"nogit"` outside a git checkout, or
/// `"invalid-sha"` if a hostile `git` is on PATH.
pub const GIT_SHA: &str = env!("GIT_SHA");

/// The build profile : "release" or "debug".
pub const PROFILE: &str = if cfg!(debug_assertions) {
    "debug"
} else {
    "release"
};

/// Crate-wide `Result` alias.
pub type Result<T> = std::result::Result<T, error::ServerError>;
