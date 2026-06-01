//! PR #22 § 7.1 — SLO-1 `L2_AVAILABILITY` kernel.
//!
//! Math (closed form from architecture doc § 7.1) :
//!
//! ```text
//!   N_1(W) = Σ efe_l2_request_total{status="success"}
//!   D_1(W) = Σ efe_l2_request_total{status ∈ {success, server_error, transport_error}}
//!                  ← deliberately excludes "client_error" (T96 anti-abuse)
//!   SLI    = N / D   if D >= LOW_TRAFFIC_DENOMINATOR_MIN, else Indeterminate
//! ```

use async_trait::async_trait;

use crate::error::SloResult;
use crate::sli::{ratio_verdict, SliContext, SliKernel, SliPoint, SliVerdict};
use crate::source::MetricQuery;
use crate::thresholds::{L2_AVAILABILITY_TARGET, LOW_TRAFFIC_DENOMINATOR_MIN};
use crate::{SliAggregation, SloId};

pub struct AvailabilityKernel;

#[async_trait]
impl SliKernel for AvailabilityKernel {
    fn slo_id(&self) -> SloId {
        SloId::L2Availability
    }

    fn aggregation(&self) -> SliAggregation {
        SliAggregation::RatioOfSums
    }

    async fn compute(&self, ctx: &SliContext<'_>) -> SloResult<SliPoint> {
        // ── Numerator : successful requests over the window.
        let numerator = ctx
            .source
            .integer_counter(MetricQuery {
                metric: "efe_l2_request_total".into(),
                window_seconds: ctx.window.as_seconds(),
                window_start_unix: ctx.window_start_unix,
                label_matchers: vec![("status".into(), "success".into())],
            })
            .await?;

        // ── Denominator : success + server_error + transport_error.
        //    NOT client_error : an attacker spamming 4xx must not be
        //    able to inflate the denominator and game the SLI (T96).
        let mut denominator: u64 = 0;
        for status in ["success", "server_error", "transport_error"] {
            denominator += ctx
                .source
                .integer_counter(MetricQuery {
                    metric: "efe_l2_request_total".into(),
                    window_seconds: ctx.window.as_seconds(),
                    window_start_unix: ctx.window_start_unix,
                    label_matchers: vec![("status".into(), status.into())],
                })
                .await?;
        }

        let coverage = ctx.source.scrape_coverage(ctx.window_start_unix, ctx.window.as_seconds()).await?;

        // ── Low-traffic guard (AC8 / AD14).
        if denominator < LOW_TRAFFIC_DENOMINATOR_MIN || coverage < crate::thresholds::COVERAGE_MIN {
            return Ok(SliPoint::indeterminate(
                self.slo_id(),
                ctx.window,
                ctx.window_start_unix,
                &ctx.source_label,
                &ctx.region,
            ));
        }

        let value = numerator as f64 / denominator as f64;
        let verdict = ratio_verdict(value, L2_AVAILABILITY_TARGET);

        Ok(SliPoint {
            slo_id: self.slo_id(),
            window: ctx.window,
            window_start_unix: ctx.window_start_unix,
            value,
            numerator,
            denominator,
            compliance_bit: 255, // not applicable for ratio-of-sums
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

    fn make_ctx_for(window: WindowSpec, mock: &MockSource) -> SliContext<'_> {
        SliContext {
            window,
            window_start_unix: 0,
            source: mock,
            source_label: "prod".into(),
            region: "global".into(),
            chart_version: "0.6.0".into(),
        }
    }

    #[tokio::test]
    async fn nominal_above_target_passes() {
        let mut mock = MockSource::default();
        mock.set_counter("efe_l2_request_total", "status", "success", 99_500);
        mock.set_counter("efe_l2_request_total", "status", "server_error", 400);
        mock.set_counter("efe_l2_request_total", "status", "transport_error", 100);
        mock.set_coverage(1.0);

        let k = AvailabilityKernel;
        let ctx = make_ctx_for(WindowSpec::FiveMin, &mock);
        let p = k.compute(&ctx).await.unwrap();

        assert_eq!(p.numerator, 99_500);
        assert_eq!(p.denominator, 100_000);
        assert!((p.value - 0.995).abs() < 1e-9);
        assert_eq!(p.verdict, SliVerdict::Pass);
    }

    #[tokio::test]
    async fn below_target_fails() {
        let mut mock = MockSource::default();
        mock.set_counter("efe_l2_request_total", "status", "success", 99_000);
        mock.set_counter("efe_l2_request_total", "status", "server_error", 1_000);
        mock.set_counter("efe_l2_request_total", "status", "transport_error", 0);
        mock.set_coverage(1.0);

        let k = AvailabilityKernel;
        let ctx = make_ctx_for(WindowSpec::FiveMin, &mock);
        let p = k.compute(&ctx).await.unwrap();

        assert_eq!(p.denominator, 100_000);
        assert_eq!(p.verdict, SliVerdict::Fail);
    }

    #[tokio::test]
    async fn client_errors_do_not_inflate_denominator_t96() {
        let mut mock = MockSource::default();
        mock.set_counter("efe_l2_request_total", "status", "success", 99_500);
        mock.set_counter("efe_l2_request_total", "status", "server_error", 500);
        mock.set_counter("efe_l2_request_total", "status", "transport_error", 0);
        // 4xx-spam attempt : 100_000 client errors. These MUST NOT
        // affect the SLI (anti-T96 — see § 7.1).
        mock.set_counter("efe_l2_request_total", "status", "client_error", 100_000);
        mock.set_coverage(1.0);

        let k = AvailabilityKernel;
        let ctx = make_ctx_for(WindowSpec::FiveMin, &mock);
        let p = k.compute(&ctx).await.unwrap();

        assert_eq!(p.denominator, 100_000); // NOT 200_000
        assert_eq!(p.verdict, SliVerdict::Pass);
    }

    #[tokio::test]
    async fn low_traffic_yields_indeterminate() {
        let mut mock = MockSource::default();
        mock.set_counter("efe_l2_request_total", "status", "success", 99);
        mock.set_counter("efe_l2_request_total", "status", "server_error", 0);
        mock.set_counter("efe_l2_request_total", "status", "transport_error", 0);
        mock.set_coverage(1.0);

        let k = AvailabilityKernel;
        let ctx = make_ctx_for(WindowSpec::FiveMin, &mock);
        let p = k.compute(&ctx).await.unwrap();
        // 99 < LOW_TRAFFIC_DENOMINATOR_MIN(100) → indeterminate
        assert_eq!(p.verdict, SliVerdict::Indeterminate);
    }

    #[tokio::test]
    async fn low_coverage_yields_indeterminate() {
        let mut mock = MockSource::default();
        mock.set_counter("efe_l2_request_total", "status", "success", 100_000);
        mock.set_counter("efe_l2_request_total", "status", "server_error", 0);
        mock.set_counter("efe_l2_request_total", "status", "transport_error", 0);
        mock.set_coverage(0.5);

        let k = AvailabilityKernel;
        let ctx = make_ctx_for(WindowSpec::FiveMin, &mock);
        let p = k.compute(&ctx).await.unwrap();
        assert_eq!(p.verdict, SliVerdict::Indeterminate);
    }

    #[tokio::test]
    async fn exact_target_is_pass() {
        let mut mock = MockSource::default();
        mock.set_counter("efe_l2_request_total", "status", "success", 995);
        mock.set_counter("efe_l2_request_total", "status", "server_error", 5);
        mock.set_counter("efe_l2_request_total", "status", "transport_error", 0);
        mock.set_coverage(1.0);
        let k = AvailabilityKernel;
        let ctx = make_ctx_for(WindowSpec::FiveMin, &mock);
        let p = k.compute(&ctx).await.unwrap();
        assert!((p.value - 0.995).abs() < 1e-12);
        assert_eq!(p.verdict, SliVerdict::Pass); // >= is pass
    }

    #[tokio::test]
    async fn zero_traffic_yields_indeterminate_not_div_by_zero() {
        let mock = MockSource::default();
        let k = AvailabilityKernel;
        let ctx = make_ctx_for(WindowSpec::FiveMin, &mock);
        let p = k.compute(&ctx).await.unwrap();
        assert_eq!(p.verdict, SliVerdict::Indeterminate);
        assert!(p.value.is_nan());
    }
}
