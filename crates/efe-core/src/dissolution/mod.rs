//! Output Projection & Dissolution Engine (M8).
//!
//! M8 is the **final step** of the EFE pipeline. Once M6 emits
//! `Converged`, the controller (M1) hands the ELS to M8, which runs a
//! strict `extract → project → decode` pipeline and returns an
//! [`OutputBundle`] ready to be shipped to the client by M12.
//!
//! # Pipeline (3 mandatory stages)
//!
//! 1. [`CentroidExtractor`] — produce a raw centroid from the active
//!    latents. MVP ships [`WeightedMeanExtractor`] and
//!    [`MedianCoordinateExtractor`].
//! 2. [`AlignmentProjector`] — project the centroid onto the convex
//!    hull of the anchors, with strict breach detection. MVP ships
//!    [`StrictAlignmentProjector`].
//! 3. [`OutputDecoder`] — turn the projected centroid into a
//!    [`DecodedOutput`]. MVP ships [`IdentityDecoder`] and
//!    [`NearestAnchorDecoder`].
//!
//! # Example
//!
//! ```ignore
//! use efe_core::dissolution::{
//!     ExtractionConfig, ExtractionContext, MvpProjector, OutputProjector,
//! };
//!
//! let projector = MvpProjector::default();
//! let ctx = ExtractionContext::new(els.swarm_id(), final_round, confidence);
//! let bundle = projector.extract(&els, &arena, &ExtractionConfig::default(), &ctx).await?;
//! // bundle.validate().is_ok() holds if debug_assertions or validate_post=true.
//! ```

pub mod alignment;
pub mod bundle;
pub mod config;
pub mod decoders;
pub mod engine;
pub mod extractors;
pub mod projector;
pub mod telemetry;
pub mod validator;

pub use alignment::{AlignmentProjector, ProjectionOutcome, StrictAlignmentProjector};
pub use bundle::{DecodedOutput, OutputBundle};
pub use config::ExtractionConfig;
pub use decoders::{IdentityDecoder, NearestAnchorDecoder, OutputDecoder};
pub use engine::{DissolutionEngine, DissolutionEngineBuilder, MvpProjector};
pub use extractors::{
    CentroidExtractor, MedianCoordinateExtractor, WeightedMeanExtractor, WeightingScheme,
};
pub use projector::{ExtractionContext, OutputProjector};
pub use telemetry::ExtractionTelemetry;
pub use validator::{classify_bundle, validate_bundle, ExtractionValidation};
