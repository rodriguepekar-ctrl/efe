//! `ConsensusDetector` trait — the contract for M6 impls.
//!
//! Implementations:
//! - [`MultiCriteriaDetector`](crate::consensus::MultiCriteriaDetector)
//!   — reference MVP, 6-signal weighted.
//! - [`StubDetector`](crate::consensus::StubDetector) — no-op for tests.
//! - Future: `BayesianDetector` (Sprint 7+).

use crate::consensus::history::ConvergenceHistory;
use crate::consensus::verdict::ConsensusVerdict;
use crate::errors::Result;
use crate::fusion::FusionStats;

/// Observe `FusionStats` emitted by M5 and emit a verdict.
///
/// # Contract
///
/// - `ingest` is the only mutating method — all others are read-only.
/// - Implementations are `Send + Sync` so the controller (M1) can share
///   one behind `Arc<Mutex<Box<dyn ConsensusDetector>>>`.
/// - `reset()` clears state; the detector can then be reused for a new
///   campaign.
///
/// # Errors
///
/// - `ingest` returns [`crate::EfeError::InvalidConfig`] when the
///   FusionStats is structurally malformed (e.g. centroid empty).
pub trait ConsensusDetector: Send + Sync + std::fmt::Debug {
    /// Ingest a round's stats and return the updated verdict.
    ///
    /// # Errors
    ///
    /// - [`crate::EfeError::InvalidConfig`] if `stats` is structurally
    ///   malformed (e.g. `centroid.is_empty()`).
    fn ingest(&mut self, stats: &FusionStats) -> Result<ConsensusVerdict>;

    /// Latest verdict. Safe to call without a prior ingest — returns
    /// `ConsensusVerdict::Continue` by default.
    fn verdict(&self) -> &ConsensusVerdict;

    /// Latest confidence score ∈ `[0, 1]`. `0.0` if no ingest yet.
    fn confidence(&self) -> f32;

    /// Borrow the convergence history (read-only view).
    fn history(&self) -> &ConvergenceHistory;

    /// Reset all state to permit reuse with a new swarm.
    fn reset(&mut self);

    /// Convenience: `true` iff the latest verdict is terminal (controller
    /// should stop the fusion loop).
    ///
    /// Default impl delegates to [`ConsensusVerdict::is_terminal`];
    /// implementations typically don't need to override.
    fn is_done(&self) -> bool {
        self.verdict().is_terminal()
    }

    /// Convenience: `true` iff the latest verdict demands a rollback
    /// via M7 (collapse + replay). Default impl delegates to
    /// [`ConsensusVerdict::should_rollback`].
    fn needs_rollback(&self) -> bool {
        self.verdict().should_rollback()
    }
}
