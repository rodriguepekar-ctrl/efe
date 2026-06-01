//! `efe-meta-swarm-loadgen` — production-grade L2 meta-swarm load generator.
//!
//! PR #20 deliverable.  See `docs/EFE_A1_PHASE2_PR20_ARCHITECTURE.md`
//! for the full architecture (40 ADs, 32 ACs, 5 patterns, 4 workloads,
//! 3 rate controllers, 5 termination policies).
//!
//! ## Quick start
//!
//! ```no_run
//! use efe_meta_swarm_loadgen::{run, LoadgenConfig};
//!
//! # async fn doc() -> anyhow::Result<()> {
//! let config = LoadgenConfig::builder()
//!     .target("grpc://controller.efe.svc:50051")
//!     .qps(50.0)
//!     .duration_secs(60)
//!     .build()?;
//! let summary = run(config).await?;
//! println!("requests = {}, p95 = {:?}", summary.total_requests, summary.p95);
//! # Ok(())
//! # }
//! ```
//!
//! ## Module map
//!
//! | Module | Purpose |
//! |---|---|
//! | [`cli`]          | clap argument parsing (40 flags) |
//! | [`config`]       | `LoadgenConfig` — CLI ⊕ profile YAML ⊕ env vars |
//! | [`client`]       | gRPC connection pool + mTLS bootstrap |
//! | [`pattern`]      | 5 traffic pattern engines |
//! | [`workload`]     | 4 workload shapes (iterate/step/reset/state) + mixed |
//! | [`rate`]         | 3 rate controllers (open / closed / hybrid) |
//! | [`metrics`]      | 5 STABLE Prometheus self-instrumentation series |
//! | [`otel`]         | OpenTelemetry SDK init + W3C trace context |
//! | [`output`]       | 4 output formatters (terminal/csv/ndjson/parquet) |
//! | [`termination`]  | 5 termination policies (duration/count/SLO/convergence/signal) |
//! | [`determinism`]  | xoshiro256+ seed propagation |
//! | [`profile`]      | YAML profile schema v1 + loader |
//! | [`replay`]       | Record + replay engine |
//! | [`engine`]       | Main run loop orchestration |
//! | [`stats`]        | Stats aggregation + SLO evaluation |
//! | [`error`]        | `LoadgenError` + exit codes |

#![warn(rust_2018_idioms, clippy::all)]
// `clippy::pedantic` is left as allow (not warn) — too noisy for a
// pragmatic codebase ; the AC32 doc-parity requirement plus the
// explicit AD audit doc compensates for the lints we'd otherwise
// chase down (cast_precision_loss, doc_markdown, etc.).
#![allow(
    clippy::module_name_repetitions,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::too_many_lines,
    missing_docs
)]

pub mod cli;
pub mod client;
pub mod config;
pub mod determinism;
pub mod engine;
pub mod error;
pub mod metrics;
pub mod otel;
pub mod output;
pub mod pattern;
pub mod profile;
pub mod rate;
pub mod replay;
pub mod stats;
pub mod termination;
pub mod workload;

// ── Public surface for embedded use (tests, future SDK integration) ─

pub use crate::config::LoadgenConfig;
pub use crate::engine::{run, RunSummary};
pub use crate::error::{ExitCode, LoadgenError, LoadgenResult};
pub use crate::profile::Profile;
pub use crate::stats::{RunStats, VerbStats};

/// The version of the loadgen profile schema currently supported.
pub const PROFILE_SCHEMA_VERSION: u32 = 1;

/// Hard cap on per-pod QPS regardless of `--i-am-sure`.  Architecture
/// § AD5 + § 25 T83 — prevents a runaway loadgen from saturating the
/// controller beyond what the cluster can physically absorb.
pub const QPS_HARD_CAP: f64 = 100_000.0;

/// Default QPS cap without `--i-am-sure`.  Architecture § AD5 — safe
/// default for accidental invocations.
pub const QPS_SAFE_DEFAULT_CAP: f64 = 1_000.0;

/// Cargo-injected build metadata, exposed via `--version` and the
/// `efe_msw_loadgen_build_info` Prometheus metric (STABILITY_CONTRACT
/// v1.2 § 9.10).
pub mod build {
    /// Crate version from `Cargo.toml`.
    pub const VERSION: &str = env!("CARGO_PKG_VERSION");

    /// Best-effort git sha — set by the CI build via `EFE_GIT_SHA` env var.
    /// Falls back to "unknown" in local dev.
    pub const GIT_SHA: &str = match option_env!("EFE_GIT_SHA") {
        Some(s) => s,
        None => "unknown",
    };

    /// Rustc version — set by the CI build via `EFE_RUSTC_VERSION`.
    pub const RUSTC_VERSION: &str = match option_env!("EFE_RUSTC_VERSION") {
        Some(s) => s,
        None => "unknown",
    };

    /// Build date — set by the CI build via `EFE_BUILD_DATE` (ISO-8601).
    pub const BUILD_DATE: &str = match option_env!("EFE_BUILD_DATE") {
        Some(s) => s,
        None => "unknown",
    };

    /// Long-form version string for `--version`.
    #[must_use]
    pub fn long() -> String {
        format!(
            "efe-meta-swarm-loadgen {VERSION}\n  git_sha       : {GIT_SHA}\n  rustc_version : {RUSTC_VERSION}\n  build_date    : {BUILD_DATE}"
        )
    }
}

#[cfg(test)]
#[allow(clippy::assertions_on_constants, clippy::float_cmp, clippy::const_is_empty)]
mod tests {
    use super::*;

    #[test]
    fn profile_schema_version_constant() {
        // STABILITY_CONTRACT v1.2 § 9.10.4 — schema v1 frozen for ≥ 6 minors.
        assert_eq!(PROFILE_SCHEMA_VERSION, 1);
    }

    #[test]
    fn qps_caps_are_sensible() {
        // AD5 hard cap.
        assert!(QPS_HARD_CAP > QPS_SAFE_DEFAULT_CAP);
        assert_eq!(QPS_HARD_CAP, 100_000.0);
        assert_eq!(QPS_SAFE_DEFAULT_CAP, 1_000.0);
    }

    #[test]
    fn build_constants_compile() {
        // const string lengths are known at compile time ; the
        // `clippy::const_is_empty` lint flags these as always-false
        // — we allow it at the module level.
        let _v = build::VERSION;
        let _g = build::GIT_SHA;
        let long = build::long();
        assert!(long.contains("efe-meta-swarm-loadgen"));
    }
}
