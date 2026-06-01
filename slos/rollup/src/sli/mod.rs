//! PR #22 § 7 + § 14.2 — SLI compute kernels.
//!
//! 4 SLOs, each as a [`SliKernel`] impl :
//!   * [`availability`]      → SLO-1 `L2_AVAILABILITY`
//!   * [`latency_p95`]       → SLO-2 `L2_LATENCY_P95`
//!   * [`recovery_time`]     → SLO-3 `SWARM_RECOVERY_TIME`
//!   * [`replication_lag`]   → SLO-4 `REPLICATION_LAG_P99`
//!
//! Each kernel implements the closed mathematical formula from
//! `docs/EFE_A1_PHASE2_PR22_ARCHITECTURE.md § 7`.  Discrepancies are
//! contract violations.

pub mod availability;
pub mod latency_p95;
pub mod recovery_time;
pub mod replication_lag;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::SloResult;
use crate::source::SourceProvider;
use crate::window::WindowSpec;
use crate::{SliAggregation, SloId};

/// Verdict for a single SLI point.  Mirrors the storage layer's
/// `verdict` field (3 string values frozen v1.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SliVerdict {
    /// SLI >= target.
    Pass,
    /// SLI < target.
    Fail,
    /// Insufficient signal (low traffic, low event count, coverage).
    Indeterminate,
}

impl SliVerdict {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Indeterminate => "indeterminate",
        }
    }
}

/// A single SLI measurement.  This is the canonical record stored in
/// Parquet (1 row per window per SLO per source).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SliPoint {
    pub slo_id: SloId,
    pub window: WindowSpec,
    /// UTC unix seconds (start of the window).
    pub window_start_unix: i64,
    /// 0.0 – 1.0 ratio for SLO-1/3 ; window-compliance mean for SLO-2/4.
    pub value: f64,
    /// Raw integer numerator (only meaningful for ratio-of-sums SLOs).
    pub numerator: u64,
    /// Raw integer denominator.
    pub denominator: u64,
    /// 0 or 1 (only for window-compliance SLOs).  255 = NaN/excluded.
    pub compliance_bit: u8,
    /// Scrape coverage in [0.0, 1.0].
    pub coverage_pct: f32,
    /// Source label : prod | synthetic_loadgen | bench.
    pub source: String,
    /// Optional region label (T104 cardinality-capped).
    pub region: String,
    pub verdict: SliVerdict,
}

impl SliPoint {
    /// Build an indeterminate point (low traffic / coverage / events).
    #[must_use]
    pub fn indeterminate(slo_id: SloId, window: WindowSpec, window_start_unix: i64, source: &str, region: &str) -> Self {
        Self {
            slo_id,
            window,
            window_start_unix,
            value: f64::NAN,
            numerator: 0,
            denominator: 0,
            compliance_bit: 255,
            coverage_pct: 0.0,
            source: source.into(),
            region: region.into(),
            verdict: SliVerdict::Indeterminate,
        }
    }
}

/// Context object passed to each kernel's `compute()`.
pub struct SliContext<'a> {
    pub window: WindowSpec,
    pub window_start_unix: i64,
    pub source: &'a dyn SourceProvider,
    /// Logical label (prod / synthetic_loadgen / bench) — see AD18.
    pub source_label: String,
    pub region: String,
    pub chart_version: String,
}

impl<'a> SliContext<'a> {
    #[must_use]
    pub fn for_window(window: WindowSpec, source: &'a dyn SourceProvider) -> Self {
        Self {
            window,
            window_start_unix: window.align(chrono::Utc::now().timestamp()),
            source,
            source_label: "prod".into(),
            region: "global".into(),
            chart_version: "0.0.0".into(),
        }
    }
}

/// The trait every SLO kernel implements (PR #22 § 14.2).
#[async_trait]
pub trait SliKernel: Send + Sync {
    fn slo_id(&self) -> SloId;
    fn aggregation(&self) -> SliAggregation;
    async fn compute(&self, ctx: &SliContext<'_>) -> SloResult<SliPoint>;
    fn verdict(&self, point: &SliPoint) -> SliVerdict;
}

/// Dispatch to the right kernel for a given SLO ID.
#[must_use]
pub fn dispatch_for(slo: SloId) -> Box<dyn SliKernel> {
    match slo {
        SloId::L2Availability => Box::new(availability::AvailabilityKernel),
        SloId::L2LatencyP95 => Box::new(latency_p95::LatencyP95Kernel),
        SloId::SwarmRecoveryTime => Box::new(recovery_time::RecoveryTimeKernel),
        SloId::ReplicationLagP99 => Box::new(replication_lag::ReplicationLagKernel),
    }
}

/// Helper : compute the verdict for an SLO-1/3 (ratio-of-sums) given
/// a target value.  Frozen — used by the storage layer too.
#[must_use]
pub fn ratio_verdict(sli: f64, target: f64) -> SliVerdict {
    if sli.is_nan() {
        SliVerdict::Indeterminate
    } else if sli >= target {
        SliVerdict::Pass
    } else {
        SliVerdict::Fail
    }
}

/// Helper : verdict for window-compliance SLOs (SLO-2/4).
#[must_use]
pub fn window_compliance_verdict(compliance_ratio: f64, target: f64) -> SliVerdict {
    if compliance_ratio.is_nan() {
        SliVerdict::Indeterminate
    } else if compliance_ratio >= target {
        SliVerdict::Pass
    } else {
        SliVerdict::Fail
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_returns_distinct_kernels() {
        for id in SloId::all() {
            let k = dispatch_for(id);
            assert_eq!(k.slo_id(), id);
        }
    }

    #[test]
    fn verdict_strings_frozen() {
        assert_eq!(SliVerdict::Pass.as_str(), "pass");
        assert_eq!(SliVerdict::Fail.as_str(), "fail");
        assert_eq!(SliVerdict::Indeterminate.as_str(), "indeterminate");
    }

    #[test]
    fn ratio_verdict_at_boundary_is_pass() {
        assert_eq!(ratio_verdict(0.995, 0.995), SliVerdict::Pass);
        assert_eq!(ratio_verdict(0.994, 0.995), SliVerdict::Fail);
        assert_eq!(ratio_verdict(f64::NAN, 0.995), SliVerdict::Indeterminate);
    }
}
