//! PR #24 § 13 + § 14 — cross-link with PR #22 (SLO) + PR #23 (smoke).

pub mod slo;
pub mod smoke;

/// Frozen SLO badge URL prefix (PR #22 STABILITY_CONTRACT v1.4 § 22.7).
/// Used by [`slo::embed_badges`].
pub const SLO_BADGE_BASE_URL: &str = "https://efe.github.io/slos/badges";

/// Frozen SLO JSON API base URL (PR #22 § 12.2).
pub const SLO_JSON_API_BASE_URL: &str = "https://efe.github.io/slos";

/// Frozen smoke badge URL (PR #23 STABILITY_CONTRACT v1.5 § 23).
pub const SMOKE_BADGE_URL: &str = "https://efe.github.io/smoke/badge.svg";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slo_badge_base_url_is_pr22_frozen() {
        assert_eq!(SLO_BADGE_BASE_URL, "https://efe.github.io/slos/badges");
    }

    #[test]
    fn smoke_badge_url_is_pr23_frozen() {
        assert_eq!(SMOKE_BADGE_URL, "https://efe.github.io/smoke/badge.svg");
    }
}
