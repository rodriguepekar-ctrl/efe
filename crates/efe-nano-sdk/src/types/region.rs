//! **Sprint 9 — N16** : Region taxonomy + data residency policy.
//!
//! See `docs/architecture/EFE_M50_N16_MULTI_REGION_REPLAY_ARCHITECTURE.md`.
//!
//! # Module surface
//!
//! - [`RegionCode`] — closed enum of supported deployment regions.
//! - [`ResidencyPolicy`] — per-tenant data-residency rules.
//! - [`ResidencyViolation`] — typed error for policy violations.
//!
//! # Region taxonomy (architecture ADR-9)
//!
//! Production regions are a **closed set** of four AWS regions :
//! `us-east-1`, `us-west-2`, `eu-west-1`, `ap-southeast-1`.  Adding a
//! region requires an SDK release — deliberate friction matching the
//! operational cost of regional expansion.

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ─────────────────────────────────────────────────────────────────────
//  RegionCode
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N16** : a deployment region.  Closed enum ; serde
/// wire form is the canonical AWS region code (`us-east-1`,
/// `eu-west-1`, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RegionCode {
    /// US East (Northern Virginia).
    #[serde(rename = "us-east-1")]
    UsEast1,
    /// US West (Oregon).
    #[serde(rename = "us-west-2")]
    UsWest2,
    /// EU West (Ireland) — GDPR-aligned EEA region.
    #[serde(rename = "eu-west-1")]
    EuWest1,
    /// Asia-Pacific Southeast (Singapore).
    #[serde(rename = "ap-southeast-1")]
    ApSoutheast1,
}

impl RegionCode {
    /// **N16** : canonical AWS region code string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UsEast1 => "us-east-1",
            Self::UsWest2 => "us-west-2",
            Self::EuWest1 => "eu-west-1",
            Self::ApSoutheast1 => "ap-southeast-1",
        }
    }

    /// **N16** : Parse from canonical AWS region code.  Returns
    /// `None` for unknown regions.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "us-east-1" => Some(Self::UsEast1),
            "us-west-2" => Some(Self::UsWest2),
            "eu-west-1" => Some(Self::EuWest1),
            "ap-southeast-1" => Some(Self::ApSoutheast1),
            _ => None,
        }
    }

    /// **N16** : True if this region is in the US jurisdiction.
    pub fn is_us(self) -> bool {
        matches!(self, Self::UsEast1 | Self::UsWest2)
    }

    /// **N16** : True if this region is in the EU jurisdiction
    /// (GDPR-applicable).
    pub fn is_eu(self) -> bool {
        matches!(self, Self::EuWest1)
    }

    /// **N16** : True if this region is in the APAC jurisdiction.
    pub fn is_apac(self) -> bool {
        matches!(self, Self::ApSoutheast1)
    }

    /// **N16** : Iterate every production region.  Order is stable
    /// (used as a canonical iteration order in tests).
    pub fn all() -> [Self; 4] {
        [
            Self::UsEast1,
            Self::UsWest2,
            Self::EuWest1,
            Self::ApSoutheast1,
        ]
    }
}

impl std::fmt::Display for RegionCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ─────────────────────────────────────────────────────────────────────
//  ResidencyPolicy
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N16** : per-tenant data-residency policy.
///
/// The coordinator validates this policy against the requested
/// `target_region` BEFORE any chain bytes leave the source region
/// (architecture G1, I15).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "config")]
pub enum ResidencyPolicy {
    /// **N16** : Data may reside in any region.  Default for
    /// non-regulated tenants.
    Global,

    /// **N16** : Data must stay in US regions
    /// (`us-east-1`, `us-west-2`).
    UsOnly,

    /// **N16** : Data must stay in EU regions (`eu-west-1`).
    /// GDPR-aligned ; rejects any cross-EEA transfer.
    EuOnly,

    /// **N16** : Data must stay in APAC regions
    /// (`ap-southeast-1`).
    ApacOnly,

    /// **N16** : Custom allow-list of regions.
    Strict { allowed: Vec<RegionCode> },

    /// **N16** : Like `Strict` but allows transfer to any region
    /// provided the body is PII-redacted in the source before
    /// shipment (architecture G5).
    RedactOnTransfer { allowed: Vec<RegionCode> },
}

impl ResidencyPolicy {
    /// **N16** : Validate that `target_region` is permitted by this
    /// policy.  Returns `Err(ResidencyViolation)` on violation.
    pub fn validate(&self, target_region: RegionCode) -> Result<(), ResidencyViolation> {
        let allowed = match self {
            Self::Global => return Ok(()),
            Self::UsOnly => target_region.is_us(),
            Self::EuOnly => target_region.is_eu(),
            Self::ApacOnly => target_region.is_apac(),
            Self::Strict { allowed } | Self::RedactOnTransfer { allowed } => {
                allowed.contains(&target_region)
            }
        };
        if allowed {
            Ok(())
        } else {
            Err(ResidencyViolation {
                policy: self.clone(),
                target_region,
            })
        }
    }

    /// **N16** : True if this policy requires PII redaction before
    /// any cross-boundary shipment.
    pub fn requires_redaction(&self) -> bool {
        matches!(self, Self::RedactOnTransfer { .. })
    }

    /// **N16** : Static label for metric dimensions.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::UsOnly => "us_only",
            Self::EuOnly => "eu_only",
            Self::ApacOnly => "apac_only",
            Self::Strict { .. } => "strict",
            Self::RedactOnTransfer { .. } => "redact_on_transfer",
        }
    }
}

impl Default for ResidencyPolicy {
    /// Default policy is `Global` — backward-compatible with pre-N16
    /// tenants where no policy was set.
    fn default() -> Self {
        Self::Global
    }
}

// ─────────────────────────────────────────────────────────────────────
//  ResidencyViolation
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N16** : returned when a `ResidencyPolicy` denies a
/// requested target region.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Error)]
#[error("data residency violation : policy {} forbids replay in {}", policy.label(), target_region)]
pub struct ResidencyViolation {
    pub policy: ResidencyPolicy,
    pub target_region: RegionCode,
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_code_as_str_stable() {
        assert_eq!(RegionCode::UsEast1.as_str(), "us-east-1");
        assert_eq!(RegionCode::UsWest2.as_str(), "us-west-2");
        assert_eq!(RegionCode::EuWest1.as_str(), "eu-west-1");
        assert_eq!(RegionCode::ApSoutheast1.as_str(), "ap-southeast-1");
    }

    #[test]
    fn region_code_round_trip_parse() {
        for r in RegionCode::all() {
            assert_eq!(RegionCode::parse(r.as_str()), Some(r));
        }
        assert_eq!(RegionCode::parse("xx-yy-99"), None);
    }

    #[test]
    fn region_code_jurisdictions_disjoint() {
        for r in RegionCode::all() {
            let count = u8::from(r.is_us()) + u8::from(r.is_eu()) + u8::from(r.is_apac());
            assert_eq!(count, 1, "region {r:?} must belong to exactly one jurisdiction");
        }
    }

    #[test]
    fn region_code_display_matches_as_str() {
        for r in RegionCode::all() {
            assert_eq!(r.to_string(), r.as_str());
        }
    }

    #[test]
    fn region_code_serde_round_trip() {
        for r in RegionCode::all() {
            let s = serde_json::to_string(&r).unwrap();
            let r2: RegionCode = serde_json::from_str(&s).unwrap();
            assert_eq!(r, r2);
            // Wire form is kebab-case quoted string.
            assert_eq!(s, format!("\"{}\"", r.as_str()));
        }
    }

    #[test]
    fn residency_policy_global_accepts_all() {
        let p = ResidencyPolicy::Global;
        for r in RegionCode::all() {
            assert!(p.validate(r).is_ok());
        }
    }

    #[test]
    fn residency_policy_us_only_accepts_us_regions() {
        let p = ResidencyPolicy::UsOnly;
        assert!(p.validate(RegionCode::UsEast1).is_ok());
        assert!(p.validate(RegionCode::UsWest2).is_ok());
        assert!(p.validate(RegionCode::EuWest1).is_err());
        assert!(p.validate(RegionCode::ApSoutheast1).is_err());
    }

    #[test]
    fn residency_policy_eu_only_blocks_non_eu() {
        let p = ResidencyPolicy::EuOnly;
        assert!(p.validate(RegionCode::EuWest1).is_ok());
        assert!(p.validate(RegionCode::UsEast1).is_err());
        assert!(p.validate(RegionCode::UsWest2).is_err());
        assert!(p.validate(RegionCode::ApSoutheast1).is_err());
    }

    #[test]
    fn residency_policy_strict_uses_allow_list() {
        let p = ResidencyPolicy::Strict {
            allowed: vec![RegionCode::EuWest1, RegionCode::ApSoutheast1],
        };
        assert!(p.validate(RegionCode::EuWest1).is_ok());
        assert!(p.validate(RegionCode::ApSoutheast1).is_ok());
        assert!(p.validate(RegionCode::UsEast1).is_err());
    }

    #[test]
    fn residency_policy_violation_carries_target_region() {
        let p = ResidencyPolicy::EuOnly;
        let err = p.validate(RegionCode::UsEast1).unwrap_err();
        assert_eq!(err.target_region, RegionCode::UsEast1);
        assert!(err.to_string().contains("us-east-1"));
        assert!(err.to_string().contains("eu_only"));
    }

    #[test]
    fn residency_policy_redact_on_transfer_flag() {
        let p = ResidencyPolicy::RedactOnTransfer {
            allowed: vec![RegionCode::UsEast1],
        };
        assert!(p.requires_redaction());
        assert!(!ResidencyPolicy::Global.requires_redaction());
    }

    #[test]
    fn residency_policy_serde_round_trip() {
        let cases = [
            ResidencyPolicy::Global,
            ResidencyPolicy::UsOnly,
            ResidencyPolicy::EuOnly,
            ResidencyPolicy::Strict {
                allowed: vec![RegionCode::UsEast1, RegionCode::EuWest1],
            },
            ResidencyPolicy::RedactOnTransfer {
                allowed: vec![RegionCode::UsEast1],
            },
        ];
        for p in &cases {
            let s = serde_json::to_string(p).unwrap();
            let p2: ResidencyPolicy = serde_json::from_str(&s).unwrap();
            assert_eq!(*p, p2);
        }
    }

    #[test]
    fn residency_policy_label_stable() {
        assert_eq!(ResidencyPolicy::Global.label(), "global");
        assert_eq!(ResidencyPolicy::EuOnly.label(), "eu_only");
        assert_eq!(
            ResidencyPolicy::Strict { allowed: vec![] }.label(),
            "strict"
        );
    }

    #[test]
    fn residency_policy_default_is_global() {
        let p = ResidencyPolicy::default();
        assert!(matches!(p, ResidencyPolicy::Global));
    }
}
