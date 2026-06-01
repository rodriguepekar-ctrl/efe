//! Runtime state of a swarm execution.
//!
//! This module exposes the enum used for telemetry and error reporting.
//! The controller enforces transitions procedurally in MVP; a typestate
//! upgrade is planned for P2 (see ADR-002).

/// Execution state of a swarm as observed externally.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum SwarmState {
    /// Request received, not yet started.
    Pending,
    /// Genesis phase running.
    Genesis,
    /// Fusion rounds running.
    Fusion {
        /// 1-based round index currently executing.
        round: u32,
    },
    /// Dissolution phase running.
    Dissolution,
    /// Completed normally.
    Done,
    /// Terminated with an error.
    Failed,
}

impl SwarmState {
    /// Stable label for telemetry.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Genesis => "genesis",
            Self::Fusion { .. } => "fusion",
            Self::Dissolution => "dissolution",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

    /// True iff this state is terminal (no further transitions).
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done | Self::Failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_stable_snake_case() {
        for s in [
            SwarmState::Pending,
            SwarmState::Genesis,
            SwarmState::Fusion { round: 1 },
            SwarmState::Dissolution,
            SwarmState::Done,
            SwarmState::Failed,
        ] {
            let l = s.label();
            assert!(l.chars().all(|c| c.is_ascii_lowercase() || c == '_'));
        }
    }

    #[test]
    fn terminal_states() {
        assert!(SwarmState::Done.is_terminal());
        assert!(SwarmState::Failed.is_terminal());
        assert!(!SwarmState::Pending.is_terminal());
        assert!(!SwarmState::Fusion { round: 3 }.is_terminal());
    }
}
