//! Semver pinning for prompt template versions (M22+ extension hook).

use std::cmp::Ordering;
use std::str::FromStr;
use thiserror::Error;

/// Pinned semver spec, e.g. `v1.0.0` (M22 extension).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SemverPin {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

#[derive(Debug, Error)]
#[error("invalid semver: {0}")]
pub struct InvalidSemver(String);

impl FromStr for SemverPin {
    type Err = InvalidSemver;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.strip_prefix('v').unwrap_or(s);
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 3 {
            return Err(InvalidSemver(s.into()));
        }
        let major = parts[0].parse().map_err(|_| InvalidSemver(s.into()))?;
        let minor = parts[1].parse().map_err(|_| InvalidSemver(s.into()))?;
        let patch = parts[2].parse().map_err(|_| InvalidSemver(s.into()))?;
        Ok(SemverPin {
            major,
            minor,
            patch,
        })
    }
}

impl PartialOrd for SemverPin {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SemverPin {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch))
    }
}

impl std::fmt::Display for SemverPin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "v{}.{}.{}", self.major, self.minor, self.patch)
    }
}
