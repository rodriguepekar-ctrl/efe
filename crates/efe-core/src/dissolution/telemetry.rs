//! `ExtractionTelemetry` — per-phase timing and diagnostics emitted by M8.

use serde::{Deserialize, Serialize};

/// Per-phase timing and metadata of one dissolution run.
///
/// Embedded in every [`crate::dissolution::OutputBundle`]. Consumed by
/// M11 for observability dashboards and by post-hoc analysis.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ExtractionTelemetry {
    /// Round index of the final fusion round that led to convergence.
    pub source_round: u32,
    /// Human-readable name of the `CentroidExtractor`.
    pub extractor_name: String,
    /// Human-readable name of the `AlignmentProjector`.
    pub projector_name: String,
    /// Human-readable name of the `OutputDecoder`.
    pub decoder_name: String,
    /// Time spent extracting the raw centroid (μs).
    pub extract_us: u32,
    /// Time spent projecting onto the alignment cone (μs).
    pub project_us: u32,
    /// Time spent in the decoder (μs).
    pub decode_us: u32,
    /// Total wallclock duration of `extract()` (μs).
    pub total_us: u32,
    /// Active agents counted during extraction.
    pub n_active_agents: u32,
    /// Number of anchors used during projection.
    pub n_anchors: u32,
}

impl ExtractionTelemetry {
    /// Sum of the three per-phase timings. Should approximate `total_us`.
    #[must_use]
    pub fn phases_us(&self) -> u32 {
        self.extract_us
            .saturating_add(self.project_us)
            .saturating_add(self.decode_us)
    }

    /// Fraction of the total spent in each phase as `(ext, proj, dec)`.
    /// Returns `(0, 0, 0)` on a zero total to avoid division by zero.
    #[must_use]
    pub fn phase_fractions(&self) -> (f32, f32, f32) {
        let total = self.total_us.max(1) as f32;
        (
            self.extract_us as f32 / total,
            self.project_us as f32 / total,
            self.decode_us as f32 / total,
        )
    }
}

impl std::fmt::Display for ExtractionTelemetry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "extraction[round={round} extract={ex}μs project={pr}μs decode={dc}μs total={tot}μs n={n} k={k}]",
            round = self.source_round,
            ex = self.extract_us,
            pr = self.project_us,
            dc = self.decode_us,
            tot = self.total_us,
            n = self.n_active_agents,
            k = self.n_anchors,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_zero() {
        let t = ExtractionTelemetry::default();
        assert_eq!(t.total_us, 0);
        assert_eq!(t.phases_us(), 0);
    }

    #[test]
    fn phases_us_sums_correctly() {
        let t = ExtractionTelemetry {
            extract_us: 10,
            project_us: 5,
            decode_us: 3,
            total_us: 20,
            ..Default::default()
        };
        assert_eq!(t.phases_us(), 18);
    }

    #[test]
    fn phase_fractions_cover_total() {
        let telemetry = ExtractionTelemetry {
            extract_us: 50,
            project_us: 30,
            decode_us: 20,
            total_us: 100,
            ..Default::default()
        };
        let (extract_f, project_f, decode_f) = telemetry.phase_fractions();
        assert!((extract_f - 0.5).abs() < 1e-6);
        assert!((project_f - 0.3).abs() < 1e-6);
        assert!((decode_f - 0.2).abs() < 1e-6);
    }

    #[test]
    fn phase_fractions_zero_total_no_panic() {
        let telemetry = ExtractionTelemetry::default();
        let _ = telemetry.phase_fractions();
    }

    #[test]
    fn display_readable() {
        let t = ExtractionTelemetry {
            source_round: 7,
            total_us: 5000,
            extract_us: 3000,
            n_active_agents: 128,
            n_anchors: 2,
            ..Default::default()
        };
        let s = format!("{t}");
        assert!(s.contains("round=7"));
        assert!(s.contains("5000"));
    }

    #[test]
    fn serde_round_trip() {
        let t = ExtractionTelemetry {
            source_round: 3,
            extractor_name: "weighted-mean".into(),
            projector_name: "strict-alignment".into(),
            decoder_name: "identity".into(),
            extract_us: 1000,
            project_us: 200,
            decode_us: 100,
            total_us: 1400,
            n_active_agents: 32,
            n_anchors: 4,
        };
        let s = serde_json::to_string(&t).unwrap();
        let back: ExtractionTelemetry = serde_json::from_str(&s).unwrap();
        assert_eq!(t, back);
    }
}
