//! `efe-slo-rollup` — 4 SLOs + multi-tier rollup compute (PR #22).
//!
//! See `docs/EFE_A1_PHASE2_PR22_ARCHITECTURE.md` for the architecture
//! (52 ADs, 38 ACs, 8-tier windows, 4-stage rollup, signed quarterly
//! evidence).
//!
//! ## Quick start
//!
//! ```no_run
//! # async fn doc() -> anyhow::Result<()> {
//! use efe_slo_rollup::{sli, source, window, SloId};
//!
//! // Compute the SLI for SLO-1 (L2_AVAILABILITY) over the last hour
//! // using a Prometheus HTTP source.
//! let src = source::prom_query::PromHttpSource::new("http://prom:9090")?;
//! let ctx = sli::SliContext::for_window(window::WindowSpec::OneHour, &src);
//! let kernel = sli::dispatch_for(SloId::L2Availability);
//! let point = kernel.compute(&ctx).await?;
//! println!("SLI = {:.4} ({})", point.value, point.verdict.as_str());
//! # Ok(())
//! # }
//! ```
//!
//! ## Module map
//!
//! | Module | Purpose |
//! |---|---|
//! | [`sli`]       | 4 SLO compute kernels — math from § 7 |
//! | [`window`]    | 8-tier window taxonomy + UTC alignment |
//! | [`source`]    | PromQL fetcher + bench-baseline reader + mock |
//! | [`rollup`]    | T1/T2/T3 stage orchestrator |
//! | [`storage`]   | Parquet daily + JSON monthly + PDF quarterly |
//! | [`burn_rate`] | Google-SRE 2-window math + PrometheusRule emit |
//! | [`badge`]     | 4 deterministic SVG generators |
//! | [`evidence`]  | quarterly evidence chain + offline verifier |
//! | [`publish`]   | mdbook site + JSON API + badge directory |
//! | [`cli`]       | clap CLI (7 subcommands frozen v1.4) |
//! | [`error`]     | `SloError` + 7 exit codes |
//! | [`build`]     | git_sha + version metadata |

#![warn(rust_2018_idioms, clippy::all)]
#![allow(
    clippy::module_name_repetitions,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::too_many_lines,
    missing_docs
)]

pub mod badge;
pub mod build;
pub mod burn_rate;
pub mod cli;
pub mod error;
pub mod evidence;
pub mod publish;
pub mod rollup;
pub mod sli;
pub mod source;
pub mod storage;
pub mod window;

pub use crate::error::{ExitCode, SloError, SloResult};
pub use crate::sli::{SliContext, SliKernel, SliPoint, SliVerdict};

// ────────────────────────────────────────────────────────────────────
//                STABILITY_CONTRACT v1.4 § 22 — frozen
// ────────────────────────────────────────────────────────────────────

/// The 4 SLO IDs.  Adding a 5th requires a major-version bump of
/// STABILITY_CONTRACT (PR #22 § 4.1 AD1).
///
/// Order is significant : it is the order in which SLOs appear in
/// the public page, in the quarterly PDF, in the monthly JSON keys.
/// Locked by `tests/stability_contract_v14.rs`.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SloId {
    L2Availability,
    L2LatencyP95,
    SwarmRecoveryTime,
    ReplicationLagP99,
}

impl SloId {
    /// All 4 SLO IDs in stable order.
    #[must_use]
    pub const fn all() -> [Self; 4] {
        [
            Self::L2Availability,
            Self::L2LatencyP95,
            Self::SwarmRecoveryTime,
            Self::ReplicationLagP99,
        ]
    }

    /// Frozen string form — used as the `slo:` recording-rule namespace
    /// suffix, in JSON keys, in PDF section titles.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::L2Availability => "L2_AVAILABILITY",
            Self::L2LatencyP95 => "L2_LATENCY_P95",
            Self::SwarmRecoveryTime => "SWARM_RECOVERY_TIME",
            Self::ReplicationLagP99 => "REPLICATION_LAG_P99",
        }
    }

    /// Parse from a frozen string.
    pub fn parse(s: &str) -> Result<Self, error::SloError> {
        Ok(match s {
            "L2_AVAILABILITY" => Self::L2Availability,
            "L2_LATENCY_P95" => Self::L2LatencyP95,
            "SWARM_RECOVERY_TIME" => Self::SwarmRecoveryTime,
            "REPLICATION_LAG_P99" => Self::ReplicationLagP99,
            other => {
                return Err(error::SloError::Config(format!(
                    "unknown SLO id '{other}' — must be one of {}",
                    Self::all().iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
                )));
            }
        })
    }

    /// Whether the SLI aggregation is a ratio-of-sums (SLO-1, SLO-3)
    /// or a mean of window-compliance bits (SLO-2, SLO-4).
    /// See PR #22 § 8.2 + AD24.
    #[must_use]
    pub const fn aggregation(self) -> SliAggregation {
        match self {
            Self::L2Availability | Self::SwarmRecoveryTime => SliAggregation::RatioOfSums,
            Self::L2LatencyP95 | Self::ReplicationLagP99 => SliAggregation::WindowCompliance,
        }
    }
}

/// SLI aggregation pattern (§ 8.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SliAggregation {
    /// $\text{SLI}^{30d} = (\sum N_i) / (\sum D_i)$ — sum the integer
    /// counters first, divide once at the end.
    RatioOfSums,
    /// $\text{SLI}^{30d} = \text{mean}(C_i)$ — mean of per-window
    /// compliance bits (1 / 0), excluding NaN windows.
    WindowCompliance,
}

/// Frozen numerical thresholds (STABILITY_CONTRACT v1.4 § 22.2).
///
/// **Mutating any value here is a major-version-bumping change.**
/// Locked by `tests/stability_contract_v14.rs`.
pub mod thresholds {
    /// SLO-1 target : success / (success + server + transport).
    pub const L2_AVAILABILITY_TARGET: f64 = 0.995;

    /// SLO-2 latency budget (seconds).
    pub const L2_LATENCY_P95_TARGET_S: f64 = 0.050;
    /// SLO-2 window-compliance target.
    pub const L2_LATENCY_P95_COMPLIANCE: f64 = 0.99;

    /// SLO-3 per-event budget (seconds).
    pub const SWARM_RECOVERY_TIME_TARGET_S: f64 = 60.0;
    /// SLO-3 event-compliance target.
    pub const SWARM_RECOVERY_TIME_COMPLIANCE: f64 = 0.99;

    /// SLO-4 max replication-lag in entries.
    pub const REPLICATION_LAG_TARGET: u64 = 100;
    /// SLO-4 window-compliance target.
    pub const REPLICATION_LAG_COMPLIANCE: f64 = 0.99;

    /// Below this scrape coverage % a window is excluded (AD14).
    pub const COVERAGE_MIN: f64 = 0.80;
    /// Below this denominator, SLO-1 is undefined (AC8).
    pub const LOW_TRAFFIC_DENOMINATOR_MIN: u64 = 100;
    /// Below this event count, SLO-3 returns INSUFFICIENT_EVENTS.
    pub const LOW_EVENT_COUNT_MIN: u64 = 10;

    /// Burn-rate fast-window multiplier (Google-SRE).
    pub const BURN_RATE_FAST: f64 = 14.4;
    /// Burn-rate slow-window multiplier.
    pub const BURN_RATE_SLOW: f64 = 6.0;
}

/// Schema version of the daily Parquet column set (PR #22 § 10.1).
pub const DAILY_PARQUET_SCHEMA_VERSION: u32 = 1;
/// Schema version of the monthly JSON file (PR #22 § 10.2).
pub const MONTHLY_JSON_SCHEMA_VERSION: u32 = 1;
/// Schema version of the quarterly evidence JSON (PR #22 § 10.3).
pub const QUARTERLY_EVIDENCE_SCHEMA_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slo_ids_match_v1_4() {
        // STABILITY_CONTRACT v1.4 § 22.1.
        assert_eq!(
            SloId::all().map(|s| s.as_str()),
            [
                "L2_AVAILABILITY",
                "L2_LATENCY_P95",
                "SWARM_RECOVERY_TIME",
                "REPLICATION_LAG_P99",
            ]
        );
    }

    #[test]
    fn slo_id_roundtrips() {
        for id in SloId::all() {
            assert_eq!(SloId::parse(id.as_str()).unwrap(), id);
        }
    }

    #[test]
    fn slo_id_rejects_unknown() {
        assert!(SloId::parse("DOES_NOT_EXIST").is_err());
    }

    #[test]
    fn aggregation_taxonomy_is_2() {
        assert_eq!(SloId::L2Availability.aggregation(), SliAggregation::RatioOfSums);
        assert_eq!(SloId::SwarmRecoveryTime.aggregation(), SliAggregation::RatioOfSums);
        assert_eq!(SloId::L2LatencyP95.aggregation(), SliAggregation::WindowCompliance);
        assert_eq!(SloId::ReplicationLagP99.aggregation(), SliAggregation::WindowCompliance);
    }

    #[test]
    fn thresholds_locked() {
        // STABILITY_CONTRACT v1.4 § 22.2.
        assert!((thresholds::L2_AVAILABILITY_TARGET - 0.995).abs() < f64::EPSILON);
        assert!((thresholds::L2_LATENCY_P95_TARGET_S - 0.050).abs() < f64::EPSILON);
        assert!((thresholds::SWARM_RECOVERY_TIME_TARGET_S - 60.0).abs() < f64::EPSILON);
        assert_eq!(thresholds::REPLICATION_LAG_TARGET, 100);
        assert!((thresholds::BURN_RATE_FAST - 14.4).abs() < f64::EPSILON);
        assert!((thresholds::BURN_RATE_SLOW - 6.0).abs() < f64::EPSILON);
    }

    #[test]
    fn schema_versions_locked_v1() {
        assert_eq!(DAILY_PARQUET_SCHEMA_VERSION, 1);
        assert_eq!(MONTHLY_JSON_SCHEMA_VERSION, 1);
        assert_eq!(QUARTERLY_EVIDENCE_SCHEMA_VERSION, 1);
    }
}
