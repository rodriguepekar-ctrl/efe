//! PR #22 § 7.4 — SLO-4 `REPLICATION_LAG_P99` kernel.
//!
//! Per-window p99 lag is computed across participants ; the **minimum**
//! of those per-participant p99s is the SLI (AD16 — Raft-aware ; the
//! min participant is the most-up-to-date, which is the canonical
//! commitment index).

use async_trait::async_trait;

use crate::error::SloResult;
use crate::sli::{SliContext, SliKernel, SliPoint, SliVerdict};
use crate::source::HistogramQuery;
use crate::thresholds::{COVERAGE_MIN, REPLICATION_LAG_TARGET};
use crate::{SliAggregation, SloId};

pub struct ReplicationLagKernel;

#[async_trait]
impl SliKernel for ReplicationLagKernel {
    fn slo_id(&self) -> SloId {
        SloId::ReplicationLagP99
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

        // Get the list of participants in the window.
        let participants = ctx.source.participants(ctx.window_start_unix, ctx.window.as_seconds()).await?;

        if participants.is_empty() {
            return Ok(SliPoint::indeterminate(
                self.slo_id(),
                ctx.window,
                ctx.window_start_unix,
                &ctx.source_label,
                &ctx.region,
            ));
        }

        // Compute p99 lag for each participant ; take the min (AD16).
        let mut per_part_p99 = Vec::with_capacity(participants.len());
        for p in &participants {
            let v = ctx
                .source
                .histogram_quantile(HistogramQuery {
                    metric_bucket: "efe_replication_lag_bucket".into(),
                    quantile: 0.99,
                    window_seconds: ctx.window.as_seconds(),
                    window_start_unix: ctx.window_start_unix,
                    label_matchers: vec![("participant".into(), p.clone())],
                })
                .await?;
            if !v.is_nan() {
                per_part_p99.push(v);
            }
        }

        if per_part_p99.is_empty() {
            return Ok(SliPoint::indeterminate(
                self.slo_id(),
                ctx.window,
                ctx.window_start_unix,
                &ctx.source_label,
                &ctx.region,
            ));
        }

        let min_p99 = per_part_p99.iter().fold(f64::INFINITY, |a, b| a.min(*b));
        let compliant = min_p99 <= REPLICATION_LAG_TARGET as f64;
        let bit: u8 = if compliant { 1 } else { 0 };

        let verdict = if compliant { SliVerdict::Pass } else { SliVerdict::Fail };

        Ok(SliPoint {
            slo_id: self.slo_id(),
            window: ctx.window,
            window_start_unix: ctx.window_start_unix,
            value: min_p99,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::MockSource;
    use crate::window::WindowSpec;

    fn ctx(mock: &MockSource) -> SliContext<'_> {
        SliContext {
            window: WindowSpec::OneMin,
            window_start_unix: 0,
            source: mock,
            source_label: "prod".into(),
            region: "global".into(),
            chart_version: "0.6.0".into(),
        }
    }

    #[tokio::test]
    async fn min_across_participants_below_target_passes() {
        let mut mock = MockSource::default();
        mock.set_participants(vec!["p1".into(), "p2".into(), "p3".into()]);
        // p1=80, p2=120, p3=150 → min=80 ≤ 100 → compliant
        mock.set_participant_p99("p1", 80.0);
        mock.set_participant_p99("p2", 120.0);
        mock.set_participant_p99("p3", 150.0);
        mock.set_coverage(1.0);
        let p = ReplicationLagKernel.compute(&ctx(&mock)).await.unwrap();
        assert!((p.value - 80.0).abs() < 1e-9);
        assert_eq!(p.compliance_bit, 1);
        assert_eq!(p.verdict, SliVerdict::Pass);
    }

    #[tokio::test]
    async fn min_above_target_fails() {
        let mut mock = MockSource::default();
        mock.set_participants(vec!["p1".into(), "p2".into()]);
        mock.set_participant_p99("p1", 150.0);
        mock.set_participant_p99("p2", 200.0);
        mock.set_coverage(1.0);
        let p = ReplicationLagKernel.compute(&ctx(&mock)).await.unwrap();
        assert!((p.value - 150.0).abs() < 1e-9);
        assert_eq!(p.verdict, SliVerdict::Fail);
    }

    #[tokio::test]
    async fn missing_one_participant_still_computes() {
        let mut mock = MockSource::default();
        // Declare 3 participants but only set p99 for 2 of them.
        // The 3rd has NaN → ignored.
        mock.set_participants(vec!["p1".into(), "p2".into(), "p3-dead".into()]);
        mock.set_participant_p99("p1", 60.0);
        mock.set_participant_p99("p2", 100.0);
        mock.set_coverage(1.0);
        let p = ReplicationLagKernel.compute(&ctx(&mock)).await.unwrap();
        assert!((p.value - 60.0).abs() < 1e-9);
        assert_eq!(p.verdict, SliVerdict::Pass);
    }

    #[tokio::test]
    async fn no_participants_yields_indeterminate() {
        let mut mock = MockSource::default();
        mock.set_coverage(1.0);
        let p = ReplicationLagKernel.compute(&ctx(&mock)).await.unwrap();
        assert_eq!(p.verdict, SliVerdict::Indeterminate);
    }

    #[tokio::test]
    async fn exact_boundary_is_compliant() {
        let mut mock = MockSource::default();
        mock.set_participants(vec!["p1".into()]);
        mock.set_participant_p99("p1", 100.0);
        mock.set_coverage(1.0);
        let p = ReplicationLagKernel.compute(&ctx(&mock)).await.unwrap();
        assert_eq!(p.compliance_bit, 1);
    }
}
