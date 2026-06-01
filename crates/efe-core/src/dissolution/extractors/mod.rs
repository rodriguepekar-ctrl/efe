//! Centroid extractors — step 1 of the dissolution pipeline.

pub mod median;
pub mod weighted_mean;

pub use median::MedianCoordinateExtractor;
pub use weighted_mean::{WeightedMeanExtractor, WeightingScheme};

use crate::els::views::{LatentView, MaskView};
use crate::errors::Result;

/// Extract a single centroid vector from the swarm's active agents.
///
/// Implementations:
/// - [`WeightedMeanExtractor`] — simple / attention-weighted mean.
/// - [`MedianCoordinateExtractor`] — per-coordinate median (robust).
pub trait CentroidExtractor: Send + Sync + std::fmt::Debug {
    /// Produce an `f32` centroid of length `latent.dim()`.
    ///
    /// # Preconditions
    /// - `latent.n_agents() == mask.n_agents()`.
    /// - At least one mask bit is set.
    ///
    /// # Errors
    /// - [`crate::EfeError::Internal`] on pre-condition violation.
    fn extract(&self, latent: &LatentView<'_>, mask: &MaskView<'_>) -> Result<Vec<f32>>;

    /// Stable short name for telemetry.
    fn name(&self) -> &'static str;
}
