//! `StubDetector` — always-`Continue` implementation for tests.
//!
//! Useful when unit-testing the controller (M1) in isolation from
//! the real consensus logic. Records ingests in an internal history
//! (so tests can inspect what the controller sent), but never emits
//! a terminal verdict.

use crate::consensus::detector::ConsensusDetector;
use crate::consensus::history::{ConvergenceHistory, StatsRecord};
use crate::consensus::verdict::ConsensusVerdict;
use crate::errors::Result;
use crate::fusion::FusionStats;

/// No-op consensus detector. Always returns `Continue`, records
/// ingested stats for inspection.
pub struct StubDetector {
    history: ConvergenceHistory,
    verdict: ConsensusVerdict,
    confidence: f32,
}

impl Default for StubDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl StubDetector {
    /// Construct a new stub with a 16-round history (plenty for tests).
    #[must_use]
    pub fn new() -> Self {
        Self {
            history: ConvergenceHistory::new(16),
            verdict: ConsensusVerdict::Continue,
            confidence: 0.0,
        }
    }

    /// Number of ingests received.
    #[must_use]
    pub fn ingest_count(&self) -> usize {
        self.history.len()
    }
}

impl std::fmt::Debug for StubDetector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StubDetector")
            .field("ingests", &self.history.len())
            .finish_non_exhaustive()
    }
}

impl ConsensusDetector for StubDetector {
    fn ingest(&mut self, stats: &FusionStats) -> Result<ConsensusVerdict> {
        self.history
            .push(StatsRecord::from_stats(stats), &stats.centroid);
        Ok(self.verdict.clone())
    }

    fn verdict(&self) -> &ConsensusVerdict {
        &self.verdict
    }

    fn confidence(&self) -> f32 {
        self.confidence
    }

    fn history(&self) -> &ConvergenceHistory {
        &self.history
    }

    fn reset(&mut self) {
        self.history.clear();
        self.verdict = ConsensusVerdict::Continue;
        self.confidence = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fusion::FusionStats;

    fn mk_stats(round: u32) -> FusionStats {
        let mut s = FusionStats::synthetic_with(round, 4, 0.1, 0.3);
        s.n_active = 4;
        s
    }

    #[test]
    fn stub_always_continues() {
        let mut d = StubDetector::new();
        for r in 0..20 {
            let v = d.ingest(&mk_stats(r)).unwrap();
            assert_eq!(v, ConsensusVerdict::Continue);
        }
    }

    #[test]
    fn stub_records_ingests() {
        let mut d = StubDetector::new();
        d.ingest(&mk_stats(0)).unwrap();
        d.ingest(&mk_stats(1)).unwrap();
        assert_eq!(d.ingest_count(), 2);
    }

    #[test]
    fn stub_reset_clears_history() {
        let mut d = StubDetector::new();
        d.ingest(&mk_stats(0)).unwrap();
        d.reset();
        assert_eq!(d.ingest_count(), 0);
    }

    #[test]
    fn stub_default_is_same_as_new() {
        let a = StubDetector::default();
        let b = StubDetector::new();
        assert_eq!(a.ingest_count(), b.ingest_count());
        assert_eq!(a.verdict(), b.verdict());
    }
}
