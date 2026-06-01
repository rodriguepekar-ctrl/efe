//! `FusionStrategy` trait — abstraction over backend implementations.
//!
//! - `MvpCpuFusion`  — pure Rust CPU, MVP (this sprint).
//! - `CudaFusion`    — GPU, Sprint 6+.
//! - `MockFusion`    — deterministic no-op for tests.
//!
//! The controller (M1) depends only on `Arc<dyn FusionStrategy>`, never
//! on concrete impls, so new backends are plug-and-play.

use async_trait::async_trait;

use crate::els::EpistemicLatentSpace;
use crate::errors::Result;
use crate::fusion::config::FusionConfig;
use crate::fusion::round::FusionRound;
use crate::memory::Arena;

/// Execute one or more fusion rounds over a swarm's ELS.
///
/// # Contract
///
/// - `run_round` must be *idempotent on observable state*: calling it
///   on an ELS with identical bytes must produce identical new bytes.
/// - Exclusive access: `els: &mut EpistemicLatentSpace` is required.
/// - Non-blocking: must never hold a lock beyond `fn` scope.
///
/// The async signature is for future compatibility (CUDA launches are
/// inherently async); MVP CPU impls can complete synchronously with a
/// trivial await-point.
#[async_trait]
pub trait FusionStrategy: Send + Sync + std::fmt::Debug {
    /// Execute exactly one round.
    ///
    /// # Errors
    /// - [`crate::EfeError::InvalidConfig`] on cfg validation.
    /// - [`crate::EfeError::Internal`] on ELS state issues.
    /// - [`crate::EfeError::NumericalExplosion`] on anomaly.
    async fn run_round(
        &self,
        els: &mut EpistemicLatentSpace,
        arena: &Arena,
        cfg: &FusionConfig,
        round: u32,
    ) -> Result<FusionRound>;

    /// Human-readable backend name for telemetry (e.g. `"mvp-cpu"`).
    fn backend_name(&self) -> &'static str;

    /// Whether this backend supports the `ReplayableFusion` operations.
    fn supports_replay(&self) -> bool {
        false
    }
}

/// A fusion strategy that can snapshot and restore latent state.
///
/// Required for M7 (collapse rollback). Strategies that don't implement
/// replay may skip this trait entirely.
#[async_trait]
pub trait ReplayableFusion: FusionStrategy {
    /// Record the current latent state under tag `round`.
    async fn record_snapshot(
        &self,
        round: u32,
        els: &EpistemicLatentSpace,
        arena: &Arena,
    ) -> Result<()>;

    /// Overwrite the current latent with the snapshot tagged `round`.
    async fn restore_snapshot(
        &self,
        round: u32,
        els: &mut EpistemicLatentSpace,
        arena: &Arena,
    ) -> Result<()>;

    /// Enumerate the rounds currently in the replay buffer,
    /// **newest-first**. Returns empty if replay is disabled or no
    /// snapshots have been recorded yet.
    ///
    /// Consumed by M7 (`SoftRollbackStrategy`) to pick a target round.
    fn available_replay_rounds(&self) -> Vec<u32>;
}
