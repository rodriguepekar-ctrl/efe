//! PR #22 § 7.2 — SLO-2 `L2_LATENCY_P95` kernel.
//!
//! Per-window p95 latency (seconds) is computed by the source.  This
//! kernel converts it into a per-window compliance bit (1 if
//! p95 ≤ 0.050s, 0 otherwise) and exposes it for later rollup.
//!
//! At the 30d compute stage, the rollup aggregates these bits into a
//! mean compliance ratio (cf. § 8.2 mean-of-bits rule).

use async_trait::async_trait;

use crate::error::SloResult;
use crate::sli::{window_compliance_verdict, SliContext, SliKernel, SliPoint, SliVerdict};
use crate::source::HistogramQuery;
use crate::thresholds::{L2_LATENCY_P95_TARGET_S, COVERAGE_MIN};
use crate::{SliAggregation, SloId};

pub struct LatencyP95Kernel;

#[async_trait]
impl SliKernel for LatencyP95Kernel {
    fn slo_id(&self) -> SloId {
        SloId::L2LatencyP95
    }

    fn aggregation(&self) -> SliAggregation {
        SliAggregation::WindowCompliance
    }

    async fn compute(&self, ctx: &SliContext<'_>) -> SloResult<SliPoint> {
        let coverage = ctx.source.scrape_coverage(ctx.window_start_unix, ctx.window.as_seconds()).await?;
        if coverage < COVERAGE_MIN {
            return Ok(SliPoint::indeterminate(
                self.slo_id(),
                ctx.window,
                ctx.window_start_unix,
                &ctx.source_label,
                &ctx.region,
            ));
        }

        let p95 = ctx
            .source
            .histogram_quantile(HistogramQuery {
                metric_bucket: "efe_l2_request_duration_seconds_bucket".into(),
                quantile: 0.95,
                window_seconds: ctx.window.as_seconds(),
                window_start_unix: ctx.window_start_unix,
                label_matchers: vec![],
            })
            .await?;

        if p95.is_nan() {
            return Ok(SliPoint::indeterminate(
                self.slo_id(),
                ctx.window,
                ctx.window_start_unix,
                &ctx.source_label,
                &ctx.region,
            ));
        }

        let compliant = p95 <= L2_LATENCY_P95_TARGET_S;
        let bit: u8 = if compliant { 1 } else { 0 };

        // For a single-point compute, "value" is the raw p95 (seconds).
        // Aggregation across windows is done elsewhere (rollup mean).
        let verdict = if compliant { SliVerdict::Pass } else { SliVerdict::Fail };

        Ok(SliPoint {
            slo_id: self.slo_id(),
            window: ctx.window,
            window_start_unix: ctx.window_start_unix,
            value: p95,
            numerator: bit as u64,
            denominator: 1,
            compliance_bit: bit,
            coverage_pct: coverage as f32,
            source: ctx.source_label.clone(),
            region: ctx.region.clone(),
            verdict,
        })
    }

    fn verdict(&self, point: &SliPoint) -> SliVerdict {
        point.verdict
    }
}

/// Aggregate a vec of per-window compliance bits into a 30d SLI value
/// (mean of bits, excluding NaN — see § 8.2 + AD24).  Used by the
/// monthly rollup stage T2.
#[must_use]
pub fn aggregate_window_compliance(bits: &[Option<u8>], target: f64) -> (f64, SliVerdict) {
    let valid: Vec<u8> = bits.iter().filter_map(|b| *b).filter(|&b| b != 255).collect();
    if valid.is_empty() {
        return (f64::NAN, SliVerdict::Indeterminate);
    }
    let sum: u64 = valid.iter().map(|&b| b as u64).sum();
    let ratio = sum as f64 / valid.len() as f64;
    (ratio, window_compliance_verdict(ratio, target))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::MockSource;
    use crate::window::WindowSpec;

    fn ctx(mock: &MockSource) -> SliContext<'_> {
        SliContext {
            window: WindowSpec::FiveMin,
            window_start_unix: 0,
            source: mock,
            source_label: "prod".into(),
            region: "global".into(),
            chart_version: "0.6.0".into(),
        }
    }

    #[tokio::test]
    async fn below_threshold_is_compliant() {
        let mut mock = MockSource::default();
        mock.set_p95("efe_l2_request_duration_seconds_bucket", 0.040);
        mock.set_coverage(1.0);
        let p = LatencyP95Kernel.compute(&ctx(&mock)).await.unwrap();
        assert_eq!(p.compliance_bit, 1);
        assert_eq!(p.verdict, SliVerdict::Pass);
    }

    #[tokio::test]
    async fn at_threshold_is_compliant() {
        let mut mock = MockSource::default();
        mock.set_p95("efe_l2_request_duration_seconds_bucket", 0.050);
        mock.set_coverage(1.0);
        let p = LatencyP95Kernel.compute(&ctx(&mock)).await.unwrap();
        assert_eq!(p.compliance_bit, 1);
    }

    #[tokio::test]
    async fn above_threshold_is_noncompliant() {
        let mut mock = MockSource::default();
        mock.set_p95("efe_l2_request_duration_seconds_bucket", 0.060);
        mock.set_coverage(1.0);
        let p = LatencyP95Kernel.compute(&ctx(&mock)).await.unwrap();
        assert_eq!(p.compliance_bit, 0);
        assert_eq!(p.verdict, SliVerdict::Fail);
    }

    #[tokio::test]
    async fn low_coverage_excludes_window() {
        let mut mock = MockSource::default();
        mock.set_p95("efe_l2_request_duration_seconds_bucket", 0.040);
        mock.set_coverage(0.1);
        let p = LatencyP95Kernel.compute(&ctx(&mock)).await.unwrap();
        assert_eq!(p.compliance_bit, 255);
        assert_eq!(p.verdict, SliVerdict::Indeterminate);
    }

    #[test]
    fn aggregate_mean_of_bits() {
        let bits = vec![Some(1), Some(1), Some(1), Some(1), Some(0)]; // 4/5 = 0.80
        let (v, verdict) = aggregate_window_compliance(&bits, 0.99);
        assert!((v - 0.80).abs() < 1e-12);
        assert_eq!(verdict, SliVerdict::Fail);
    }

    #[test]
    fn aggregate_excludes_nan_bits() {
        let bits = vec![Some(1), Some(1), Some(1), Some(255), Some(255)]; // 3/3 valid
        let (v, _) = aggregate_window_compliance(&bits, 0.99);
        assert!((v - 1.0).abs() < 1e-12);
    }

    #[test]
    fn aggregate_all_nan_is_indeterminate() {
        let bits = vec![Some(255), Some(255)];
        let (v, verdict) = aggregate_window_compliance(&bits, 0.99);
        assert!(v.is_nan());
        assert_eq!(verdict, SliVerdict::Indeterminate);
    }
}
