//! PR #22 § 8 — window taxonomy (8 tiers, UTC-aligned, frozen v1.4).
//!
//! Order is significant : it is the descending-coverage order used by
//! the rollup pipeline.

pub mod aggregate;
pub mod align;
pub mod boundary;

use serde::{Deserialize, Serialize};

/// 8 UTC-aligned window sizes (STABILITY_CONTRACT v1.4 § 22.3 — frozen).
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowSpec {
    /// 1 min (used internally for SLO-4 rate denominators).
    OneMin,
    /// 5 min (fast burn).
    FiveMin,
    /// 30 min (intermediate burn).
    ThirtyMin,
    /// 1 hour (recording-rule + T1 point).
    OneHour,
    /// 6 hours (operator dashboard).
    SixHours,
    /// 24 hours (daily compliance — Parquet row group day boundary).
    TwentyFourHours,
    /// 7 days (weekly trend).
    SevenDays,
    /// 30 days (monthly compliance).
    ThirtyDays,
}

impl WindowSpec {
    /// All 8 windows in stable order (`OneMin` first).
    #[must_use]
    pub const fn all() -> [Self; 8] {
        [
            Self::OneMin,
            Self::FiveMin,
            Self::ThirtyMin,
            Self::OneHour,
            Self::SixHours,
            Self::TwentyFourHours,
            Self::SevenDays,
            Self::ThirtyDays,
        ]
    }

    /// Window duration in seconds.
    #[must_use]
    pub const fn as_seconds(self) -> u64 {
        match self {
            Self::OneMin => 60,
            Self::FiveMin => 300,
            Self::ThirtyMin => 1_800,
            Self::OneHour => 3_600,
            Self::SixHours => 21_600,
            Self::TwentyFourHours => 86_400,
            Self::SevenDays => 604_800,
            Self::ThirtyDays => 2_592_000,
        }
    }

    /// Frozen string form (used in Prom recording-rule labels +
    /// Parquet column values).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OneMin => "1m",
            Self::FiveMin => "5m",
            Self::ThirtyMin => "30m",
            Self::OneHour => "1h",
            Self::SixHours => "6h",
            Self::TwentyFourHours => "24h",
            Self::SevenDays => "7d",
            Self::ThirtyDays => "30d",
        }
    }

    /// Parse from humantime-style suffix.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "1m" => Some(Self::OneMin),
            "5m" => Some(Self::FiveMin),
            "30m" => Some(Self::ThirtyMin),
            "1h" => Some(Self::OneHour),
            "6h" => Some(Self::SixHours),
            "24h" | "1d" => Some(Self::TwentyFourHours),
            "7d" => Some(Self::SevenDays),
            "30d" => Some(Self::ThirtyDays),
            _ => None,
        }
    }

    /// Round an instant down to the nearest aligned bucket start (UTC).
    /// Returns the unix-seconds boundary of the bucket containing `at`.
    #[must_use]
    pub const fn align(self, at_unix_secs: i64) -> i64 {
        let size = self.as_seconds() as i64;
        // Floor to the bucket : works for negative values via Euclidean
        // remainder (.rem_euclid).
        let rem = at_unix_secs.rem_euclid(size);
        at_unix_secs - rem
    }

    /// Whether this window is used by the burn-rate alert layer.
    #[must_use]
    pub const fn is_burn_window(self) -> bool {
        matches!(
            self,
            Self::FiveMin | Self::ThirtyMin | Self::OneHour | Self::SixHours
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taxonomy_has_8_tiers() {
        assert_eq!(WindowSpec::all().len(), 8);
    }

    #[test]
    fn seconds_are_canonical() {
        assert_eq!(WindowSpec::FiveMin.as_seconds(), 300);
        assert_eq!(WindowSpec::OneHour.as_seconds(), 3_600);
        assert_eq!(WindowSpec::TwentyFourHours.as_seconds(), 86_400);
        assert_eq!(WindowSpec::ThirtyDays.as_seconds(), 2_592_000);
    }

    #[test]
    fn string_form_is_frozen() {
        // STABILITY_CONTRACT v1.4 § 22.3.
        assert_eq!(WindowSpec::FiveMin.as_str(), "5m");
        assert_eq!(WindowSpec::OneHour.as_str(), "1h");
        assert_eq!(WindowSpec::ThirtyDays.as_str(), "30d");
    }

    #[test]
    fn parse_roundtrips() {
        for w in WindowSpec::all() {
            assert_eq!(WindowSpec::parse(w.as_str()), Some(w));
        }
        assert_eq!(WindowSpec::parse("1d"), Some(WindowSpec::TwentyFourHours));
    }

    #[test]
    fn align_floors_to_bucket() {
        // 5min window : 305s → 300, 600s → 600, 12345s → 12300
        assert_eq!(WindowSpec::FiveMin.align(305), 300);
        assert_eq!(WindowSpec::FiveMin.align(600), 600);
        assert_eq!(WindowSpec::FiveMin.align(12_345), 12_300);
    }

    #[test]
    fn align_is_idempotent() {
        for w in WindowSpec::all() {
            let aligned = w.align(1_234_567_890);
            assert_eq!(w.align(aligned), aligned);
        }
    }

    #[test]
    fn burn_windows_are_4() {
        let burn: Vec<_> = WindowSpec::all().into_iter().filter(|w| w.is_burn_window()).collect();
        assert_eq!(burn.len(), 4);
        assert!(burn.contains(&WindowSpec::FiveMin));
        assert!(burn.contains(&WindowSpec::ThirtyMin));
        assert!(burn.contains(&WindowSpec::OneHour));
        assert!(burn.contains(&WindowSpec::SixHours));
    }
}
