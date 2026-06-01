//! `ExtractionConfig` — tunable policy of the M8 dissolution engine.

use serde::{Deserialize, Serialize};

use crate::errors::{EfeError, Result};

/// Tunable config of the dissolution engine.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct ExtractionConfig {
    /// Blend factor between raw centroid and anchor-centroid.
    ///
    /// - `0.0` → return raw (no projection).
    /// - `1.0` → fully projected onto the anchor centroid.
    /// - Default `0.5` — half-way blend, standard policy.
    pub alignment_strength: f32,
    /// Tolerance multiplier applied to the cone radius: an alignment
    /// breach fires iff
    /// `delta > cone_radius * (1 + breach_tolerance)`. Default `0.05`
    /// — 5% slack for numerical noise / bf16 precision.
    pub breach_tolerance: f32,
    /// Minimum consensus confidence required to attempt extraction.
    /// If `ctx.convergence_confidence < min_confidence`, the engine
    /// returns `EfeError::Internal` without running any extractor.
    pub min_confidence: f32,
    /// Wallclock budget for one `extract()` call (ms).
    pub deadline_ms: u32,
    /// When `true`, run `OutputBundle::validate()` at the end of
    /// extraction. Defaults to debug-only — expensive in hot-path.
    pub validate_post: bool,
}

impl Default for ExtractionConfig {
    fn default() -> Self {
        Self {
            alignment_strength: 0.5,
            breach_tolerance: 0.05,
            min_confidence: 0.5,
            deadline_ms: 20,
            validate_post: cfg!(debug_assertions),
        }
    }
}

impl ExtractionConfig {
    /// Validate configuration invariants.
    ///
    /// # Errors
    ///
    /// - [`EfeError::InvalidConfig`] if any field is out of range.
    pub fn validate(&self) -> Result<()> {
        if !self.alignment_strength.is_finite() || !(0.0..=1.0).contains(&self.alignment_strength) {
            return Err(EfeError::InvalidConfig(format!(
                "ExtractionConfig: alignment_strength {} must be finite and in [0, 1]",
                self.alignment_strength
            )));
        }
        if !self.breach_tolerance.is_finite() || !(0.0..=1.0).contains(&self.breach_tolerance) {
            return Err(EfeError::InvalidConfig(format!(
                "ExtractionConfig: breach_tolerance {} must be finite and in [0, 1]",
                self.breach_tolerance
            )));
        }
        if !self.min_confidence.is_finite() || !(0.0..=1.0).contains(&self.min_confidence) {
            return Err(EfeError::InvalidConfig(format!(
                "ExtractionConfig: min_confidence {} must be finite and in [0, 1]",
                self.min_confidence
            )));
        }
        if self.deadline_ms == 0 {
            return Err(EfeError::InvalidConfig(
                "ExtractionConfig: deadline_ms must be ≥ 1".into(),
            ));
        }
        Ok(())
    }

    /// Fluent setter for alignment strength.
    #[must_use]
    pub fn with_alignment_strength(mut self, a: f32) -> Self {
        self.alignment_strength = a;
        self
    }

    /// Fluent setter for breach tolerance.
    #[must_use]
    pub fn with_breach_tolerance(mut self, t: f32) -> Self {
        self.breach_tolerance = t;
        self
    }

    /// Fluent setter for min confidence.
    #[must_use]
    pub fn with_min_confidence(mut self, c: f32) -> Self {
        self.min_confidence = c;
        self
    }

    /// Fluent setter for deadline (ms).
    #[must_use]
    pub fn with_deadline_ms(mut self, ms: u32) -> Self {
        self.deadline_ms = ms;
        self
    }

    /// Fluent setter for validate_post.
    #[must_use]
    pub fn with_validate_post(mut self, v: bool) -> Self {
        self.validate_post = v;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_valid() {
        ExtractionConfig::default().validate().unwrap();
    }

    #[test]
    fn rejects_alignment_strength_out_of_range() {
        let c = ExtractionConfig::default().with_alignment_strength(1.5);
        assert!(c.validate().is_err());
        let c = ExtractionConfig::default().with_alignment_strength(-0.1);
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_nan_alignment_strength() {
        let c = ExtractionConfig::default().with_alignment_strength(f32::NAN);
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_breach_tolerance_out_of_range() {
        let c = ExtractionConfig::default().with_breach_tolerance(2.0);
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_min_confidence_out_of_range() {
        let c = ExtractionConfig::default().with_min_confidence(1.5);
        assert!(c.validate().is_err());
        let c = ExtractionConfig::default().with_min_confidence(-0.1);
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_zero_deadline() {
        let c = ExtractionConfig::default().with_deadline_ms(0);
        assert!(c.validate().is_err());
    }

    #[test]
    fn accepts_zero_alignment_strength_and_tolerance() {
        let c = ExtractionConfig::default()
            .with_alignment_strength(0.0)
            .with_breach_tolerance(0.0);
        c.validate().unwrap();
    }

    #[test]
    fn fluent_setters_compose() {
        let c = ExtractionConfig::default()
            .with_alignment_strength(0.7)
            .with_breach_tolerance(0.1)
            .with_min_confidence(0.8)
            .with_deadline_ms(50)
            .with_validate_post(true);
        assert!((c.alignment_strength - 0.7).abs() < 1e-6);
        assert!((c.breach_tolerance - 0.1).abs() < 1e-6);
        assert!((c.min_confidence - 0.8).abs() < 1e-6);
        assert_eq!(c.deadline_ms, 50);
        assert!(c.validate_post);
        c.validate().unwrap();
    }

    #[test]
    fn serde_round_trip() {
        let c = ExtractionConfig::default();
        let s = serde_json::to_string(&c).unwrap();
        let back: ExtractionConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(c, back);
    }
}
