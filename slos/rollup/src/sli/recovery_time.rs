//! PR #22 § 7.3 — SLO-3 `SWARM_RECOVERY_TIME` kernel.
//!
//! Event-driven (NOT time-windowed).  Reads discrete recovery events
//! from the source and emits the fraction-within-budget over the
//! enclosing window.

use async_trait::async_trait;

use crate::error::SloResult;
use crate::sli::{ratio_verdict, SliContext, SliKernel, SliPoint, SliVerdict};
use crate::source::EventQuery;
use crate::thresholds::{LOW_EVENT_COUNT_MIN, SWARM_RECOVERY_TIME_TARGET_S, SWARM_RECOVERY_TIME_COMPLIANCE};
use crate::{SliAggregation, SloId};

pub struct RecoveryTimeKernel;

#[async_trait]
impl SliKernel for RecoveryTimeKernel {
    fn slo_id(&self) -> SloId {
        SloId::SwarmRecoveryTime
    }

    fn aggregation(&self) -> SliAggregation {
        SliAggregation::RatioOfSums
    }

    async fn compute(&self, ctx: &SliContext<'_>) -> SloResult<SliPoint> {
        let events = ctx
            .source
            .recovery_events(EventQuery {
                metric: "efe_swarm_recovery_event_total".into(),
                window_seconds: ctx.window.as_seconds(),
                window_start_unix: ctx.window_start_unix,
            })
            .await?;

        if (events.len() as u64) < LOW_EVENT_COUNT_MIN {
            return Ok(SliPoint::indeterminate(
                self.slo_id(),
                ctx.window,
                ctx.window_start_unix,
                &ctx.source_label,
                &ctx.region,
            ));
        }

        let total = events.len() as u64;
        let within = events
            .iter()
            .filter(|e| e.duration_seconds <= SWARM_RECOVERY_TIME_TARGET_S)
            .count() as u64;

        let value = within as f64 / total as f64;
        let verdict = ratio_verdict(value, SWARM_RECOVERY_TIME_COMPLIANCE);

        Ok(SliPoint {
            slo_id: self.slo_id(),
            window: ctx.window,
            window_start_unix: ctx.window_start_unix,
            value,
            numerator: within,
            denominator: total,
            compliance_bit: 255,
            coverage_pct: 1.0, // events are point-in-time ; coverage not applicable
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
    use crate::source::{MockSource, RecoveryEvent};
    use crate::window::WindowSpec;

    fn ctx(mock: &MockSource) -> SliContext<'_> {
        SliContext {
            window: WindowSpec::TwentyFourHours,
            window_start_unix: 0,
            source: mock,
            source_label: "prod".into(),
            region: "global".into(),
            chart_version: "0.6.0".into(),
        }
    }

    #[tokio::test]
    async fn all_within_budget_passes() {
        let mut mock = MockSource::default();
        for i in 0..15 {
            mock.push_recovery_event(RecoveryEvent { duration_seconds: 30.0 + (i as f64), at_unix: 0 });
        }
        let p = RecoveryTimeKernel.compute(&ctx(&mock)).await.unwrap();
        assert_eq!(p.numerator, 15);
        assert_eq!(p.denominator, 15);
        assert!((p.value - 1.0).abs() < 1e-12);
        assert_eq!(p.verdict, SliVerdict::Pass);
    }

    #[tokio::test]
    async fn one_long_recovery_in_100_still_passes() {
        let mut mock = MockSource::default();
        // 99 events @ 30s, 1 event @ 120s : 99/100 = 0.99 == compliance target.
        for _ in 0..99 {
            mock.push_recovery_event(RecoveryEvent { duration_seconds: 30.0, at_unix: 0 });
        }
        mock.push_recovery_event(RecoveryEvent { duration_seconds: 120.0, at_unix: 0 });
        let p = RecoveryTimeKernel.compute(&ctx(&mock)).await.unwrap();
        assert_eq!(p.numerator, 99);
        assert_eq!(p.denominator, 100);
        assert_eq!(p.verdict, SliVerdict::Pass); // >= target
    }

    #[tokio::test]
    async fn below_compliance_fails() {
        let mut mock = MockSource::default();
        // 50/100 within → 0.5 < 0.99
        for _ in 0..50 {
            mock.push_recovery_event(RecoveryEvent { duration_seconds: 30.0, at_unix: 0 });
        }
        for _ in 0..50 {
            mock.push_recovery_event(RecoveryEvent { duration_seconds: 120.0, at_unix: 0 });
        }
        let p = RecoveryTimeKernel.compute(&ctx(&mock)).await.unwrap();
        assert_eq!(p.verdict, SliVerdict::Fail);
    }

    #[tokio::test]
    async fn under_10_events_yields_indeterminate() {
        let mut mock = MockSource::default();
        for _ in 0..5 {
            mock.push_recovery_event(RecoveryEvent { duration_seconds: 30.0, at_unix: 0 });
        }
        let p = RecoveryTimeKernel.compute(&ctx(&mock)).await.unwrap();
        assert_eq!(p.verdict, SliVerdict::Indeterminate);
        assert_eq!(p.denominator, 0); // indeterminate point zeros these
    }

    #[tokio::test]
    async fn zero_events_yields_indeterminate() {
        let mock = MockSource::default();
        let p = RecoveryTimeKernel.compute(&ctx(&mock)).await.unwrap();
        assert_eq!(p.verdict, SliVerdict::Indeterminate);
    }

    #[tokio::test]
    async fn exact_60s_boundary_is_compliant() {
        let mut mock = MockSource::default();
        for _ in 0..10 {
            mock.push_recovery_event(RecoveryEvent { duration_seconds: 60.0, at_unix: 0 });
        }
        let p = RecoveryTimeKernel.compute(&ctx(&mock)).await.unwrap();
        assert_eq!(p.numerator, 10);
        assert_eq!(p.verdict, SliVerdict::Pass);
    }

    #[tokio::test]
    async fn just_over_60s_is_not_compliant() {
        let mut mock = MockSource::default();
        for _ in 0..10 {
            // 60.001 > 60.0
            mock.push_recovery_event(RecoveryEvent { duration_seconds: 60.001, at_unix: 0 });
        }
        let p = RecoveryTimeKernel.compute(&ctx(&mock)).await.unwrap();
        assert_eq!(p.numerator, 0);
        assert_eq!(p.verdict, SliVerdict::Fail);
    }
}
