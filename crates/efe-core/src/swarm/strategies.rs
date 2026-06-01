//! Strategy traits — the contracts M3, M5, M6, M7, M9, M11 implement.
//!
//! The controller (M1) sees the rest of the system exclusively through
//! these traits. No concrete type from M3-M11 leaks into the controller,
//! which enables:
//!
//! - **Testability**: any strategy can be replaced with a hand-rolled
//!   mock for unit / integration tests.
//! - **Evolution**: replacing a fusion strategy (dense → hierarchical)
//!   does not require touching the controller.
//! - **Multi-variant**: different configurations can pick different
//!   strategies at runtime.

use std::time::Duration;

use async_trait::async_trait;

use crate::els::EpistemicLatentSpace;
use crate::errors::{EfeError, Result};
use crate::fusion::FusionStats;
use crate::memory::NanoAgentBatch;
use crate::swarm::request::{DecodeMode, DecodedOutput, Request};
use crate::swarm::verdict::Verdict;
use crate::types::{AlignmentCone, SwarmId, SwarmSpec};

// =====================================================================
// M3 - Genesis
// =====================================================================

/// Creates a fresh swarm from a specification.
#[async_trait]
pub trait GenesisStrategy: Send + Sync {
    /// Build a batch and populate the latent space.
    ///
    /// # Errors
    ///
    /// - [`EfeError::HbmAllocFailed`] when the memory pool cannot provide
    ///   a large-enough arena.
    /// - [`EfeError::TemplateMissing`] when a template referenced by the
    ///   [`SwarmSpec`] is absent from the registry.
    async fn spawn(&self, spec: &SwarmSpec) -> Result<(NanoAgentBatch, EpistemicLatentSpace)>;

    /// Respawn a fraction of the batch for mode-collapse recovery.
    ///
    /// # Errors
    ///
    /// Propagates genesis failures.
    async fn respawn_diversify(&self, batch: &mut NanoAgentBatch, count: u32) -> Result<()>;
}

// =====================================================================
// M5 - Fusion
// =====================================================================

/// Executes one fusion round (Swarm Attention + Langevin + stats).
#[async_trait]
pub trait FusionStrategy: Send + Sync {
    /// Run round `round` and return its statistics.
    ///
    /// # Errors
    ///
    /// - [`EfeError::KernelFailure`] on CUDA errors.
    /// - [`EfeError::NumericalExplosion`] when stats are not finite.
    async fn run_round(
        &self,
        els: &mut EpistemicLatentSpace,
        batch: &NanoAgentBatch,
        round: u32,
    ) -> Result<FusionStats>;

    /// Inject isotropic noise into the ELS (used for mode-collapse recovery).
    ///
    /// # Errors
    ///
    /// Propagates kernel failures.
    async fn inject_noise(&self, els: &mut EpistemicLatentSpace, sigma: f32) -> Result<()>;
}

// =====================================================================
// M6 - Consensus
// =====================================================================

/// Stateful consensus detector.
///
/// Implementations maintain rolling history (variance, gradient norm,
/// centroid movement) and emit a verdict per round.
pub trait ConsensusDetector: Send + Sync {
    /// Evaluate the round stats and decide.
    fn evaluate(&mut self, round: u32, stats: &FusionStats) -> Verdict;

    /// Reset internal state (used when the same detector instance is
    /// reused across requests).
    fn reset(&mut self);
}

// =====================================================================
// M7 - Dissolution
// =====================================================================

/// Extracts a centroid and decodes it into a user-facing output.
#[async_trait]
pub trait Dissolver: Send + Sync {
    /// Extract the consensus centroid (Frechet mean) from the ELS.
    ///
    /// # Errors
    ///
    /// Propagates kernel failures.
    async fn extract_centroid(&self, els: &EpistemicLatentSpace) -> Result<Vec<f32>>;

    /// Decode the (aligned) centroid into a user-facing output.
    ///
    /// # Errors
    ///
    /// Propagates tokenizer / schema errors.
    async fn decode(&self, centroid: &[f32], mode: &DecodeMode) -> Result<DecodedOutput>;
}

// =====================================================================
// M9 - Alignment
// =====================================================================

/// Projects a centroid onto the alignment cone.
pub trait AlignmentProjector: Send + Sync {
    /// Return the projected centroid and the L2 distance between input and output.
    ///
    /// # Errors
    ///
    /// The controller converts an over-threshold `delta` into
    /// [`EfeError::AlignmentBreach`]; implementations should not raise
    /// an error themselves for exceeding the threshold.
    fn project(&self, centroid: &[f32], cone: &AlignmentCone) -> Result<(Vec<f32>, f32)>;
}

// =====================================================================
// M11 - Telemetry
// =====================================================================

/// Produces a per-swarm [`TelemetryGuard`] that accumulates signals and
/// flushes on drop.
pub trait TelemetryCollector: Send + Sync {
    /// Open a telemetry session for a swarm execution.
    fn begin_swarm(&self, id: SwarmId, req: &Request) -> Box<dyn TelemetryGuard>;
}

/// Drop-guard holding an open telemetry session. Flushes accumulated
/// signals when dropped (even on early error).
pub trait TelemetryGuard: Send {
    /// Record a phase completion (genesis / fusion / dissolution).
    fn record_phase_complete(&mut self, phase: &str, elapsed: Duration);

    /// Record that a phase errored out.
    fn record_phase_error(&mut self, phase: &str, err: &EfeError);

    /// Record the stats of a completed round.
    fn record_round(&mut self, round: u32, stats: &FusionStats);

    /// Record a structured event with a label.
    fn record_event(&mut self, round: u32, event: &str);

    /// Record the final outcome and total latency.
    fn finalize(&mut self, total_ms: f32, stats: &FusionStats);
}

// =====================================================================
// No-op implementations (for tests and minimal deployments)
// =====================================================================

/// A no-op telemetry collector.
///
/// Useful for tests and when the user disables telemetry entirely.
#[derive(Clone, Default)]
pub struct NullTelemetryCollector;

impl TelemetryCollector for NullTelemetryCollector {
    fn begin_swarm(&self, _id: SwarmId, _req: &Request) -> Box<dyn TelemetryGuard> {
        Box::new(NullTelemetryGuard)
    }
}

/// No-op telemetry guard.
pub struct NullTelemetryGuard;

impl TelemetryGuard for NullTelemetryGuard {
    fn record_phase_complete(&mut self, _phase: &str, _elapsed: Duration) {}
    fn record_phase_error(&mut self, _phase: &str, _err: &EfeError) {}
    fn record_round(&mut self, _round: u32, _stats: &FusionStats) {}
    fn record_event(&mut self, _round: u32, _event: &str) {}
    fn finalize(&mut self, _total_ms: f32, _stats: &FusionStats) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AlignmentCone, LatentDim, PopulationMix, SwarmSpec, TemplateCode};

    fn spec() -> SwarmSpec {
        SwarmSpec::builder()
            .problem_embedding(vec![0.0; 4])
            .dim_latent(LatentDim::new(4).expect("valid"))
            .population(PopulationMix::new(vec![(TemplateCode::NaSem, 4)]).expect("valid"))
            .alignment_cone(AlignmentCone::new(vec![vec![0.1; 4]], 0.15).expect("valid cone"))
            .build()
            .expect("valid spec")
    }

    #[test]
    fn null_telemetry_compiles_and_runs() {
        let tc = NullTelemetryCollector;
        let req = Request::for_tests(spec());
        let mut guard = tc.begin_swarm(SwarmId::new(), &req);
        guard.record_phase_complete("genesis", Duration::from_millis(1));
        guard.record_round(1, &FusionStats::synthetic_converging(1, 4));
        guard.record_event(1, "test");
        guard.finalize(2.0, &FusionStats::synthetic_converging(1, 4));
    }
}
