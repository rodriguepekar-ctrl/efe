//! PR #20 § 17.3 — until-SLO-breach.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;

use super::{StopReason, TerminationPolicy};
use crate::config::SloThreshold;
use crate::stats::RunStats;

/// Watches the loadgen-side stats every tick and trips on the first SLO breach.
pub struct SloPolicy {
    thresholds: Vec<SloThreshold>,
    /// Minimum number of requests before SLO evaluation starts.  Avoids
    /// false positives from a small initial sample.
    min_samples: u64,
}

impl SloPolicy {
    #[must_use]
    pub fn new(thresholds: Vec<SloThreshold>) -> Self {
        Self { thresholds, min_samples: 50 }
    }

    /// Lower the minimum-samples floor (testing).
    #[cfg(test)]
    pub const fn with_min_samples(mut self, n: u64) -> Self {
        self.min_samples = n;
        self
    }
}

#[async_trait]
impl TerminationPolicy for SloPolicy {
    async fn evaluate(&self, _start: Instant, stats: &Arc<RunStats>) -> Option<StopReason> {
        let snap = stats.snapshot();
        let total: u64 = snap.by_verb.values().map(|v| v.total).sum();
        if total < self.min_samples {
            return None;
        }
        stats.first_slo_breach(&self.thresholds).map(StopReason::SloBreach)
    }
    fn name(&self) -> &'static str { "slo" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SloDirection;
    use crate::stats::{RequestResult, Verb};
    use std::time::Duration;

    #[tokio::test]
    async fn fires_when_availability_below_target() {
        let stats = Arc::new(RunStats::new(0));
        for _ in 0..70 {
            stats.record(Verb::Iterate, RequestResult::Ok, Duration::from_millis(10));
        }
        for _ in 0..30 {
            stats.record(Verb::Iterate, RequestResult::Error, Duration::from_millis(10));
        }
        let p = SloPolicy::new(vec![SloThreshold {
            slo_id: "iterate-availability".into(),
            target: 0.99,
            direction: SloDirection::AtLeast,
        }])
        .with_min_samples(50);
        let stop = p.evaluate(Instant::now(), &stats).await;
        match stop {
            Some(StopReason::SloBreach(b)) => assert_eq!(b.slo_id, "iterate-availability"),
            other => panic!("expected SloBreach, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn does_not_fire_below_min_samples() {
        let stats = Arc::new(RunStats::new(0));
        stats.record(Verb::Iterate, RequestResult::Error, Duration::from_millis(10));
        let p = SloPolicy::new(vec![SloThreshold {
            slo_id: "iterate-availability".into(),
            target: 0.99,
            direction: SloDirection::AtLeast,
        }]);
        assert!(p.evaluate(Instant::now(), &stats).await.is_none());
    }
}
