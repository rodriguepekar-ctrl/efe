//! [`NanoAgentVersion`] (concrete semver) and [`VersionReq`] (constraint).

use semver::Version;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::error::{NanoAgentVersionError, VersionReqError};

/// A concrete semver version of a nano agent.
///
/// Wraps [`semver::Version`].
///
/// # Examples
///
/// ```
/// use efe_nano_sdk::types::NanoAgentVersion;
///
/// let v = NanoAgentVersion::new("1.0.0").unwrap();
/// assert_eq!(v.as_str(), "1.0.0");
///
/// assert!(NanoAgentVersion::new("1.0.0-beta.1").is_ok());
/// assert!(NanoAgentVersion::new("1.x").is_err());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct NanoAgentVersion(String);

impl NanoAgentVersion {
    /// Construct a new version, validating semver.
    pub fn new(s: impl Into<String>) -> Result<Self, NanoAgentVersionError> {
        let s = s.into();
        if s.is_empty() {
            return Err(NanoAgentVersionError::Empty);
        }
        Version::parse(&s).map_err(|e| NanoAgentVersionError::InvalidSemver {
            got: s.clone(),
            reason: e.to_string(),
        })?;
        Ok(Self(s))
    }

    /// Returns the underlying string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes self, returning the underlying String.
    pub fn into_inner(self) -> String {
        self.0
    }

    /// Returns the parsed [`semver::Version`].
    ///
    /// This is always valid because construction validates.
    pub fn as_semver(&self) -> Version {
        Version::parse(&self.0).expect("validated at construction")
    }
}

impl fmt::Display for NanoAgentVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for NanoAgentVersion {
    type Err = NanoAgentVersionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl AsRef<str> for NanoAgentVersion {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for NanoAgentVersion {
    type Error = NanoAgentVersionError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl From<NanoAgentVersion> for String {
    fn from(v: NanoAgentVersion) -> String {
        v.0
    }
}

// ─────────────────────────────────────────────────────────────────────
//  VersionReq — semver constraint (e.g. "^1.0.0", "1.x", "=1.2.3")
// ─────────────────────────────────────────────────────────────────────

/// A semver constraint used to resolve a nano agent reference to a
/// concrete version.
///
/// Wraps [`semver::VersionReq`].
///
/// # Examples
///
/// ```
/// use efe_nano_sdk::types::{NanoAgentVersion, VersionReq};
///
/// let req = VersionReq::new("^1.0").unwrap();
/// let v1 = NanoAgentVersion::new("1.2.3").unwrap();
/// let v2 = NanoAgentVersion::new("2.0.0").unwrap();
///
/// assert!(req.matches(&v1));
/// assert!(!req.matches(&v2));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct VersionReq(String);

impl VersionReq {
    /// Construct a new VersionReq, validating the semver requirement string.
    pub fn new(s: impl Into<String>) -> Result<Self, VersionReqError> {
        let s = s.into();
        if s.is_empty() {
            return Err(VersionReqError::Empty);
        }
        semver::VersionReq::parse(&s).map_err(|e| VersionReqError::InvalidVersionReq {
            got: s.clone(),
            reason: e.to_string(),
        })?;
        Ok(Self(s))
    }

    /// Returns the underlying string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the parsed [`semver::VersionReq`].
    pub fn as_semver_req(&self) -> semver::VersionReq {
        semver::VersionReq::parse(&self.0).expect("validated at construction")
    }

    /// Returns true if `version` satisfies this requirement.
    pub fn matches(&self, version: &NanoAgentVersion) -> bool {
        self.as_semver_req().matches(&version.as_semver())
    }

    /// Construct a "match-any" version requirement (`*`).
    pub fn any() -> Self {
        Self("*".to_string())
    }

    /// **Sprint 8 — N13** : Is this requirement an exact-version pin
    /// (e.g. `=1.2.3` or just `1.2.3` with no operator) ?
    ///
    /// Used by `Catalog::resolve_for_dispatch` to bypass the rollout
    /// policy when the caller explicitly pinned a version.
    pub fn is_exact_pin(&self) -> bool {
        let req = self.as_semver_req();
        // Semver "=" or naked specific version (no operator) → single comparator.
        if req.comparators.len() != 1 {
            return false;
        }
        let c = &req.comparators[0];
        // `Op::Exact` is the `=` operator.  Naked "1.2.3" parses as
        // `^1.2.3` (Caret) in semver, NOT as exact — semantically that
        // matches a range, so we treat it as non-pin.  Only `=X.Y.Z`
        // (and the equivalent serde-parsed `Op::Exact`) is a pin.
        matches!(c.op, semver::Op::Exact)
            && c.major != 0_u64.saturating_sub(1) // sanity
            && c.minor.is_some()
            && c.patch.is_some()
    }
}

impl fmt::Display for VersionReq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for VersionReq {
    type Err = VersionReqError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl AsRef<str> for VersionReq {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for VersionReq {
    type Error = VersionReqError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl From<VersionReq> for String {
    fn from(r: VersionReq) -> String {
        r.0
    }
}

impl Default for VersionReq {
    fn default() -> Self {
        Self::any()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_valid_versions() {
        assert!(NanoAgentVersion::new("0.1.0").is_ok());
        assert!(NanoAgentVersion::new("1.0.0").is_ok());
        assert!(NanoAgentVersion::new("1.2.3-beta.1").is_ok());
        assert!(NanoAgentVersion::new("1.2.3-rc.1+build.42").is_ok());
    }

    #[test]
    fn invalid_versions_rejected() {
        assert!(NanoAgentVersion::new("").is_err());
        assert!(NanoAgentVersion::new("1").is_err());
        assert!(NanoAgentVersion::new("1.0").is_err());
        assert!(NanoAgentVersion::new("1.x.0").is_err());
        assert!(NanoAgentVersion::new("v1.0.0").is_err());
    }

    #[test]
    fn version_serde_round_trip() {
        let v = NanoAgentVersion::new("1.2.3").unwrap();
        let s = serde_json::to_string(&v).unwrap();
        assert_eq!(s, r#""1.2.3""#);
        let v2: NanoAgentVersion = serde_json::from_str(&s).unwrap();
        assert_eq!(v, v2);
    }

    #[test]
    fn version_req_matches() {
        let req = VersionReq::new("^1.0").unwrap();
        assert!(req.matches(&NanoAgentVersion::new("1.0.0").unwrap()));
        assert!(req.matches(&NanoAgentVersion::new("1.2.3").unwrap()));
        assert!(req.matches(&NanoAgentVersion::new("1.99.99").unwrap()));
        assert!(!req.matches(&NanoAgentVersion::new("2.0.0").unwrap()));
        assert!(!req.matches(&NanoAgentVersion::new("0.9.9").unwrap()));
    }

    #[test]
    fn version_req_exact() {
        let req = VersionReq::new("=1.2.3").unwrap();
        assert!(req.matches(&NanoAgentVersion::new("1.2.3").unwrap()));
        assert!(!req.matches(&NanoAgentVersion::new("1.2.4").unwrap()));
    }

    #[test]
    fn version_req_any_matches_all() {
        let req = VersionReq::any();
        assert!(req.matches(&NanoAgentVersion::new("0.0.1").unwrap()));
        assert!(req.matches(&NanoAgentVersion::new("99.99.99").unwrap()));
    }

    #[test]
    fn version_req_default_is_any() {
        let req: VersionReq = Default::default();
        assert_eq!(req.as_str(), "*");
    }

    #[test]
    fn version_req_invalid_rejected() {
        assert!(VersionReq::new("").is_err());
        assert!(VersionReq::new("not-a-version").is_err());
    }
}
