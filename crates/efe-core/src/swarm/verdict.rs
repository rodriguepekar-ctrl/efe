//! Consensus verdict emitted after each fusion round.

/// Decision made by [`crate::swarm::strategies::ConsensusDetector`] after
/// inspecting a [`crate::fusion::FusionStats`].
///
/// The controller interprets each verdict:
/// - [`Verdict::Converged`] → exit the round loop with a final answer.
/// - [`Verdict::Continue`] → run another round.
/// - [`Verdict::Divergence`] → abort the swarm with an error.
/// - [`Verdict::ModeCollapse`] → trigger internal recovery (inject noise +
///   respawn), then retry.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Verdict {
    /// Consensus reached; `confidence` in `[0, 1]`.
    Converged {
        /// Confidence score, `1 - min(variance, 1)`.
        confidence: f32,
    },
    /// No verdict yet; keep iterating.
    Continue,
    /// Divergence detected (variance increasing for consecutive rounds).
    Divergence {
        /// Static reason string for telemetry.
        reason: &'static str,
    },
    /// Swarm collapsed into a single point too early.
    ModeCollapse,
}

impl Verdict {
    /// Short code for telemetry labels.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Converged { .. } => "converged",
            Self::Continue => "continue",
            Self::Divergence { .. } => "divergence",
            Self::ModeCollapse => "mode_collapse",
        }
    }

    /// True iff this verdict ends the round loop (with or without error).
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_codes_are_stable() {
        assert_eq!(Verdict::Converged { confidence: 0.9 }.code(), "converged");
        assert_eq!(Verdict::Continue.code(), "continue");
        assert_eq!(Verdict::Divergence { reason: "x" }.code(), "divergence");
        assert_eq!(Verdict::ModeCollapse.code(), "mode_collapse");
    }

    #[test]
    fn continue_is_not_terminal() {
        assert!(!Verdict::Continue.is_terminal());
        assert!(Verdict::Converged { confidence: 1.0 }.is_terminal());
        assert!(Verdict::Divergence { reason: "x" }.is_terminal());
        assert!(Verdict::ModeCollapse.is_terminal());
    }
}
