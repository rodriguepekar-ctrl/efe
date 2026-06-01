//! `efe-bench-harness` — public-bench harness for the EFE meta-swarm
//! L2 surface.
//!
//! PR #21 deliverable.  See `docs/EFE_A1_PHASE2_PR21_ARCHITECTURE.md`
//! for the full architecture (45 ADs, 35 ACs, 10 scenarios, 6 chaos
//! faults).
//!
//! ## Quick start
//!
//! ```no_run
//! # async fn doc() -> anyhow::Result<()> {
//! use efe_bench_harness::{scenario::Scenario, engine};
//! let s = Scenario::load("bench/scenarios/S01_baseline_100qps_5min.yaml")?;
//! // kubeconfig=None ⇒ chaos uses mock applier ; baseline_root=None ⇒
//! // skip baseline compare ; target_override=None ⇒ use scenario URLs.
//! let result = engine::run_one(&s, None, None, None).await?;
//! println!("verdict = {:?}", result.verdict);
//! # Ok(())
//! # }
//! ```
//!
//! ## Module map
//!
//! | Module | Purpose |
//! |---|---|
//! | [`scenario`]   | YAML loader + schema v1 |
//! | [`cluster`]    | kind / k3s / external-kubeconfig provisioner |
//! | [`chaos`]      | chaos-mesh CRD applier + cleanup |
//! | [`loadgen`]    | PR #20 integration (in-process library mode) |
//! | [`result`]     | Result aggregator + summary serializer |
//! | [`compare`]    | Per-metric tolerance + verdict engine |
//! | [`baseline`]   | Versioned baseline store + manifest |
//! | [`publish`]    | GitHub Pages mdbook + SVG badge generator |
//! | [`pr_comment`] | PR comment formatter (markdown) |
//! | [`engine`]     | Top-level orchestrator |
//! | [`cli`]        | clap CLI |
//! | [`error`]      | `BenchError` + exit codes |

#![warn(rust_2018_idioms, clippy::all)]
#![allow(
    clippy::module_name_repetitions,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::too_many_lines,
    missing_docs
)]

pub mod baseline;
pub mod chaos;
pub mod cli;
pub mod cluster;
pub mod compare;
pub mod engine;
pub mod error;
pub mod loadgen;
pub mod pr_comment;
pub mod publish;
pub mod result;
pub mod scenario;

pub use crate::error::{BenchError, BenchResult, ExitCode};

/// Schema version of the scenario YAML files this harness supports.
///
/// Committed in STABILITY_CONTRACT v1.3 § 21.1 — frozen for ≥ 6
/// chart-minor versions.  Future schema bumps emit `version: 2` +
/// a migration helper.
pub const SCENARIO_SCHEMA_VERSION: u32 = 1;

/// Schema version of the baseline JSON files.
pub const BASELINE_SCHEMA_VERSION: u32 = 1;

/// Default tolerance defaults applied when a scenario YAML omits
/// the `tolerance:` block.  Mirrors Appendix D § 33.
pub mod tolerance_defaults {
    /// p50 fail threshold (% above baseline).
    pub const P50_FAIL_PCT: f64 = 15.0;
    pub const P50_WARN_PCT: f64 = 8.0;
    pub const P95_FAIL_PCT: f64 = 25.0;
    pub const P95_WARN_PCT: f64 = 12.0;
    pub const P99_FAIL_PCT: f64 = 35.0;
    pub const P99_WARN_PCT: f64 = 15.0;
    pub const ERROR_RATE_FAIL_PP: f64 = 1.0;
    pub const ERROR_RATE_WARN_PP: f64 = 0.2;
    pub const THROUGHPUT_FAIL_PCT: f64 = -15.0;
    pub const THROUGHPUT_WARN_PCT: f64 = -5.0;
    pub const RECOVERY_TIME_FAIL_S: f64 = 15.0;
    pub const RECOVERY_TIME_WARN_S: f64 = 5.0;
}

/// Build metadata exposed via `--version`.
pub mod build {
    pub const VERSION: &str = env!("CARGO_PKG_VERSION");
    pub const GIT_SHA: &str = match option_env!("EFE_GIT_SHA") {
        Some(s) => s,
        None => "unknown",
    };
    pub const RUSTC_VERSION: &str = match option_env!("EFE_RUSTC_VERSION") {
        Some(s) => s,
        None => "unknown",
    };
    pub const BUILD_DATE: &str = match option_env!("EFE_BUILD_DATE") {
        Some(s) => s,
        None => "unknown",
    };

    #[must_use]
    pub fn long() -> String {
        format!(
            "efe-bench-harness {VERSION}\n  git_sha       : {GIT_SHA}\n  rustc_version : {RUSTC_VERSION}\n  build_date    : {BUILD_DATE}"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_versions_stable() {
        assert_eq!(SCENARIO_SCHEMA_VERSION, 1);
        assert_eq!(BASELINE_SCHEMA_VERSION, 1);
    }

    // Constant relationship between fail / warn thresholds — checked at
    // compile time so a future contributor cannot break the invariant
    // without a build error.  (clippy::assertions_on_constants would
    // optimise the runtime version out — these `const _` are the right
    // pattern for compile-time assertions on `const` values.)
    const _P50_OK: () = assert!(tolerance_defaults::P50_FAIL_PCT > tolerance_defaults::P50_WARN_PCT);
    const _P95_OK: () = assert!(tolerance_defaults::P95_FAIL_PCT > tolerance_defaults::P95_WARN_PCT);
    const _P99_OK: () = assert!(tolerance_defaults::P99_FAIL_PCT > tolerance_defaults::P99_WARN_PCT);
    const _ER_OK: ()  = assert!(tolerance_defaults::ERROR_RATE_FAIL_PP > tolerance_defaults::ERROR_RATE_WARN_PP);
    // Throughput direction inverted : a more negative delta is worse.
    const _TP_OK: ()  = assert!(tolerance_defaults::THROUGHPUT_FAIL_PCT < tolerance_defaults::THROUGHPUT_WARN_PCT);

    #[test]
    fn tolerance_defaults_const_assertions_compiled() {
        // If the const _ assertions above compiled, the invariants hold.
        // Reference them to make the contract reader-visible at runtime too.
        let _ = (_P50_OK, _P95_OK, _P99_OK, _ER_OK, _TP_OK);
    }
}
