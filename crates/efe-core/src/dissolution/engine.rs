//! `DissolutionEngine` + `MvpProjector` — orchestration of the extract
//! → project → decode pipeline.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use crate::dissolution::alignment::{AlignmentProjector, StrictAlignmentProjector};
use crate::dissolution::bundle::OutputBundle;
use crate::dissolution::config::ExtractionConfig;
use crate::dissolution::decoders::{IdentityDecoder, OutputDecoder};
use crate::dissolution::extractors::{CentroidExtractor, WeightedMeanExtractor};
use crate::dissolution::projector::{ExtractionContext, OutputProjector};
use crate::dissolution::telemetry::ExtractionTelemetry;
use crate::els::EpistemicLatentSpace;
use crate::errors::{EfeError, Result};
use crate::memory::Arena;

/// Dependency bundle carrying the three pipeline stages. Built via
/// [`DissolutionEngineBuilder`]. Consumed by [`MvpProjector`].
#[derive(Clone)]
pub struct DissolutionEngine {
    /// Stage 1 — extract a raw centroid from the ELS.
    pub extractor: Arc<dyn CentroidExtractor>,
    /// Stage 2 — project the centroid onto the alignment cone.
    pub projector: Arc<dyn AlignmentProjector>,
    /// Stage 3 — decode the projected centroid.
    pub decoder: Arc<dyn OutputDecoder>,
}

impl std::fmt::Debug for DissolutionEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DissolutionEngine")
            .field("extractor", &self.extractor.name())
            .field("projector", &self.projector.name())
            .field("decoder", &self.decoder.name())
            .finish()
    }
}

impl DissolutionEngine {
    /// Start the builder.
    #[must_use]
    pub fn builder() -> DissolutionEngineBuilder {
        DissolutionEngineBuilder::default()
    }

    /// MVP defaults : uniform weighted mean + strict projector + identity decoder.
    #[must_use]
    pub fn mvp_default() -> Self {
        Self {
            extractor: Arc::new(WeightedMeanExtractor::default()),
            projector: Arc::new(StrictAlignmentProjector),
            decoder: Arc::new(IdentityDecoder),
        }
    }
}

/// Builder for [`DissolutionEngine`].
#[derive(Default)]
pub struct DissolutionEngineBuilder {
    extractor: Option<Arc<dyn CentroidExtractor>>,
    projector: Option<Arc<dyn AlignmentProjector>>,
    decoder: Option<Arc<dyn OutputDecoder>>,
}

impl DissolutionEngineBuilder {
    /// Set the centroid extractor.
    #[must_use]
    pub fn with_extractor(mut self, e: Arc<dyn CentroidExtractor>) -> Self {
        self.extractor = Some(e);
        self
    }

    /// Set the alignment projector.
    #[must_use]
    pub fn with_projector(mut self, p: Arc<dyn AlignmentProjector>) -> Self {
        self.projector = Some(p);
        self
    }

    /// Set the output decoder.
    #[must_use]
    pub fn with_decoder(mut self, d: Arc<dyn OutputDecoder>) -> Self {
        self.decoder = Some(d);
        self
    }

    /// Build, using MVP defaults for any unspecified stage.
    #[must_use]
    pub fn build(self) -> DissolutionEngine {
        DissolutionEngine {
            extractor: self
                .extractor
                .unwrap_or_else(|| Arc::new(WeightedMeanExtractor::default())),
            projector: self
                .projector
                .unwrap_or_else(|| Arc::new(StrictAlignmentProjector)),
            decoder: self.decoder.unwrap_or_else(|| Arc::new(IdentityDecoder)),
        }
    }
}

/// MVP implementation of [`OutputProjector`] — wraps a
/// [`DissolutionEngine`] and orchestrates the 3-stage pipeline.
#[derive(Clone, Debug)]
pub struct MvpProjector {
    engine: DissolutionEngine,
}

impl Default for MvpProjector {
    fn default() -> Self {
        Self::with_engine(DissolutionEngine::mvp_default())
    }
}

impl MvpProjector {
    /// Construct with a pre-built engine.
    #[must_use]
    pub fn with_engine(engine: DissolutionEngine) -> Self {
        Self { engine }
    }

    /// Access the underlying engine (for inspection / tests).
    #[must_use]
    pub fn engine(&self) -> &DissolutionEngine {
        &self.engine
    }
}

#[async_trait]
impl OutputProjector for MvpProjector {
    #[allow(clippy::too_many_lines)]
    async fn extract(
        &self,
        els: &EpistemicLatentSpace,
        arena: &Arena,
        config: &ExtractionConfig,
        ctx: &ExtractionContext,
    ) -> Result<OutputBundle> {
        let start = Instant::now();

        // 0. Validate config.
        config.validate()?;
        let deadline = start + Duration::from_millis(u64::from(config.deadline_ms));

        // 1. min_confidence gate.
        if !ctx.convergence_confidence.is_finite()
            || ctx.convergence_confidence < config.min_confidence
        {
            return Err(EfeError::Internal(
                "MvpProjector: convergence_confidence below min_confidence",
            ));
        }

        // 2. Acquire ELS views (read-only).
        let latent = els
            .latent_view(arena)?
            .ok_or(EfeError::Internal("MvpProjector: ELS has no latent"))?;
        let mask = els
            .mask_view(arena)?
            .ok_or(EfeError::Internal("MvpProjector: ELS has no mask"))?;
        let anchors_view = els
            .anchors_view(arena)?
            .ok_or(EfeError::Internal("MvpProjector: ELS has no anchors"))?;
        let anchors_host = anchors_view.to_host();
        let cone_radius = els.alignment_cone_radius();
        let n_anchors = anchors_view.n_anchors();
        let n_active = mask.active_count();

        // 3. Stage 1 — extract the raw centroid.
        let t_extract = Instant::now();
        let centroid_raw = self.engine.extractor.extract(&latent, &mask)?;
        let extract_us = u32::try_from(t_extract.elapsed().as_micros()).unwrap_or(u32::MAX);
        if Instant::now() >= deadline {
            return Err(EfeError::BudgetExhausted {
                kind: "dissolution_deadline",
            });
        }

        // 4. Stage 2 — project onto the alignment cone with strict breach
        //    detection.
        let t_project = Instant::now();
        let projection =
            self.engine
                .projector
                .project(&centroid_raw, &anchors_host, cone_radius, config)?;
        let project_us = u32::try_from(t_project.elapsed().as_micros()).unwrap_or(u32::MAX);
        if Instant::now() >= deadline {
            return Err(EfeError::BudgetExhausted {
                kind: "dissolution_deadline",
            });
        }

        // 5. Stage 3 — decode. Decoders may be async (HF tokenizer,
        //    ONNX, etc.); we still enforce the wallclock deadline
        //    *after* the call — strict enforcement during the decoder
        //    itself is the decoder's own responsibility (it holds
        //    `ctx.deadline` via config if needed).
        let t_decode = Instant::now();
        let decoded = self
            .engine
            .decoder
            .decode(&projection.projected_centroid, &projection.anchor_weights)
            .await?;
        let decode_us = u32::try_from(t_decode.elapsed().as_micros()).unwrap_or(u32::MAX);
        if Instant::now() >= deadline {
            return Err(EfeError::BudgetExhausted {
                kind: "dissolution_deadline",
            });
        }

        // 6. Assemble the bundle.
        let total_us = u32::try_from(start.elapsed().as_micros()).unwrap_or(u32::MAX);
        let bundle = OutputBundle {
            swarm_id: ctx.swarm_id,
            centroid_raw,
            centroid_projected: projection.projected_centroid,
            confidence: ctx.convergence_confidence,
            alignment_distance: projection.distance_from_raw,
            anchor_weights: projection.anchor_weights,
            decoded,
            telemetry: ExtractionTelemetry {
                source_round: ctx.final_round,
                extractor_name: self.engine.extractor.name().into(),
                projector_name: self.engine.projector.name().into(),
                decoder_name: self.engine.decoder.name().into(),
                extract_us,
                project_us,
                decode_us,
                total_us,
                n_active_agents: n_active,
                n_anchors,
            },
        };

        // 7. Post-validation.
        if config.validate_post {
            bundle.validate()?;
        }

        Ok(bundle)
    }

    fn name(&self) -> &'static str {
        "mvp-projector"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_default_has_mvp_names() {
        let e = DissolutionEngine::mvp_default();
        assert_eq!(e.extractor.name(), "weighted-mean");
        assert_eq!(e.projector.name(), "strict-alignment");
        assert_eq!(e.decoder.name(), "identity");
    }

    #[test]
    fn builder_uses_defaults_when_unset() {
        let e = DissolutionEngine::builder().build();
        assert_eq!(e.extractor.name(), "weighted-mean");
        assert_eq!(e.projector.name(), "strict-alignment");
        assert_eq!(e.decoder.name(), "identity");
    }

    #[test]
    fn builder_overrides_extractor() {
        use crate::dissolution::extractors::MedianCoordinateExtractor;
        let e = DissolutionEngine::builder()
            .with_extractor(Arc::new(MedianCoordinateExtractor))
            .build();
        assert_eq!(e.extractor.name(), "median-coord");
        assert_eq!(e.projector.name(), "strict-alignment");
    }

    #[test]
    fn debug_lists_stage_names() {
        let e = DissolutionEngine::mvp_default();
        let s = format!("{e:?}");
        assert!(s.contains("weighted-mean"));
        assert!(s.contains("strict-alignment"));
        assert!(s.contains("identity"));
    }

    #[test]
    fn mvp_projector_name_is_stable() {
        assert_eq!(MvpProjector::default().name(), "mvp-projector");
    }
}
