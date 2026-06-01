//! `OutputProjector` trait + `ExtractionContext` — top-level API of M8.

use async_trait::async_trait;

use crate::dissolution::bundle::OutputBundle;
use crate::dissolution::config::ExtractionConfig;
use crate::els::EpistemicLatentSpace;
use crate::errors::Result;
use crate::memory::Arena;
use crate::types::SwarmId;

/// Per-call context passed alongside the config and ELS.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtractionContext {
    /// Round index of the final fusion round that produced the
    /// converged state.
    pub final_round: u32,
    /// Confidence emitted by M6 alongside `Converged`.
    pub convergence_confidence: f32,
    /// Swarm identifier for telemetry correlation.
    pub swarm_id: SwarmId,
}

impl ExtractionContext {
    /// Convenience constructor.
    #[must_use]
    pub fn new(swarm_id: SwarmId, final_round: u32, convergence_confidence: f32) -> Self {
        Self {
            final_round,
            convergence_confidence,
            swarm_id,
        }
    }
}

/// Dissolve a converged swarm into a final output bundle.
///
/// Implementations:
/// - [`crate::dissolution::MvpProjector`] — reference MVP.
///
/// # Contract
///
/// - Called at most once per swarm, after M6 emits `Converged`.
/// - Read-only on the ELS — no mutation.
/// - Returns `Ok(bundle)` or one of:
///   - [`crate::EfeError::AlignmentBreach`] — projection violates cone.
///   - [`crate::EfeError::InvalidConfig`] — bad `ExtractionConfig`.
///   - [`crate::EfeError::Internal`] — ELS is a stub / low confidence /
///     post-condition violated.
#[async_trait]
pub trait OutputProjector: Send + Sync + std::fmt::Debug {
    /// Run the full extract → project → decode pipeline.
    ///
    /// # Errors
    /// See trait-level docs.
    async fn extract(
        &self,
        els: &EpistemicLatentSpace,
        arena: &Arena,
        config: &ExtractionConfig,
        ctx: &ExtractionContext,
    ) -> Result<OutputBundle>;

    /// Stable short name for telemetry.
    fn name(&self) -> &'static str;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_new_stores_fields() {
        let sid = SwarmId::new();
        let ctx = ExtractionContext::new(sid, 5, 0.87);
        assert_eq!(ctx.final_round, 5);
        assert!((ctx.convergence_confidence - 0.87).abs() < 1e-6);
        assert_eq!(ctx.swarm_id, sid);
    }

    #[test]
    fn context_eq_on_same_fields() {
        let sid = SwarmId::new();
        let a = ExtractionContext::new(sid, 3, 0.5);
        let b = ExtractionContext::new(sid, 3, 0.5);
        assert_eq!(a, b);
    }
}
