//! PR #22 § 7.5 — boundary semantics (gap / restart / re-clock).
//!
//! When the source layer (Prom or otherwise) reports a discontinuity,
//! the affected window's compliance bit is set to 255 and the point is
//! marked `Indeterminate` instead of `Fail`.  This avoids false-fails
//! that come from monitoring gaps rather than service degradation.

use serde::{Deserialize, Serialize};

/// Why a window was rejected from compliance.  Stored alongside the
/// SliPoint when the verdict is `Indeterminate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndeterminateReason {
    /// Scrape coverage below 80% (AD14).
    CoverageLow,
    /// Counter reset detected (negative rate).
    CounterReset,
    /// Denominator below 100 (AC8).
    LowTraffic,
    /// Recovery events below 10 (AC11).
    LowEventCount,
    /// All participants missing (SLO-4).
    NoParticipants,
}

impl IndeterminateReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CoverageLow => "coverage_low",
            Self::CounterReset => "counter_reset",
            Self::LowTraffic => "low_traffic",
            Self::LowEventCount => "low_event_count",
            Self::NoParticipants => "no_participants",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasons_have_frozen_strings() {
        assert_eq!(IndeterminateReason::CoverageLow.as_str(), "coverage_low");
        assert_eq!(IndeterminateReason::CounterReset.as_str(), "counter_reset");
        assert_eq!(IndeterminateReason::LowTraffic.as_str(), "low_traffic");
        assert_eq!(IndeterminateReason::LowEventCount.as_str(), "low_event_count");
        assert_eq!(IndeterminateReason::NoParticipants.as_str(), "no_participants");
    }
}
