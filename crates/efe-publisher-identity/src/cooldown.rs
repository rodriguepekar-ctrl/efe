// SPDX-License-Identifier: UNLICENSED
//! **Sprint 10 — N18** : Cooldown enforcement for Tier 2+ rollout
//! promotion.
//!
//! Architecture G5 + F14 + ADR-4.

use std::time::{Duration, SystemTime};

use efe_nano_sdk::{KycTier, PublisherIdentity};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum CooldownError {
    #[error("cooldown not elapsed : {remaining_seconds} s remaining")]
    NotElapsed { remaining_seconds: u64 },
}

/// **Sprint 10 — N18** : enforce per-tier cooldown.
pub struct CooldownEnforcer;

impl CooldownEnforcer {
    /// **N18** : Check whether a Tier 2+ version may be promoted to
    /// Stable at `now` given its `published_at` and the publisher's
    /// tier.
    ///
    /// Tier 0 / Tier 1 : no Stable promotion regardless ; returns
    /// `Ok(())` here (the rollout layer is responsible for the
    /// catalog-scope check).
    pub fn check(
        publisher: &PublisherIdentity,
        published_at: SystemTime,
        now: SystemTime,
    ) -> Result<(), CooldownError> {
        let cooldown_hours = publisher.kyc_tier.cooldown_hours();
        if cooldown_hours == 0 {
            return Ok(());
        }
        let elapsed = now.duration_since(published_at).unwrap_or_default();
        let required = Duration::from_secs(u64::from(cooldown_hours) * 3_600);
        if elapsed >= required {
            return Ok(());
        }
        let remaining = required.saturating_sub(elapsed);
        Err(CooldownError::NotElapsed {
            remaining_seconds: remaining.as_secs(),
        })
    }

    /// **N18** : Compute the earliest promote-allowed timestamp for
    /// `published_at` + `tier`.  Used by ops dashboards.
    pub fn allowed_at(
        publisher: &PublisherIdentity,
        published_at: SystemTime,
    ) -> SystemTime {
        let cooldown = u64::from(publisher.kyc_tier.cooldown_hours()) * 3_600;
        published_at + Duration::from_secs(cooldown)
    }
}

// Force `KycTier` to remain in scope even if unused above (compile-
// time check).
#[allow(dead_code)]
fn _kyctier_used() -> Option<KycTier> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::{
        Iso3166, KeyId, PublisherId, PublisherKey, PublisherStatus, SignatureAlgorithm,
    };

    fn identity(tier: KycTier) -> PublisherIdentity {
        PublisherIdentity {
            publisher_id: PublisherId::new(),
            display_name: "X".into(),
            kyc_tier: tier,
            status: PublisherStatus::Active,
            jurisdiction: vec![Iso3166::new("US").unwrap()],
            registered_at: SystemTime::now(),
            last_sanctions_check_at: None,
            active_key: PublisherKey {
                key_id: KeyId::new(),
                algorithm: SignatureAlgorithm::Ed25519,
                public_key_pem: "pem".into(),
                valid_from: SystemTime::now(),
                valid_until: None,
            },
            historical_keys: vec![],
            kyc_dossier_reference: None,
            revocation: None,
            forks_of: None,
        }
    }

    #[test]
    fn tier_2_blocked_within_24h() {
        let i = identity(KycTier::Tier2);
        let now = SystemTime::now();
        let pub_time = now - Duration::from_secs(60 * 60); // 1h ago
        let err = CooldownEnforcer::check(&i, pub_time, now).unwrap_err();
        assert!(matches!(err, CooldownError::NotElapsed { .. }));
    }

    #[test]
    fn tier_2_passes_after_24h() {
        let i = identity(KycTier::Tier2);
        let now = SystemTime::now();
        let pub_time = now - Duration::from_secs(25 * 3_600);
        assert!(CooldownEnforcer::check(&i, pub_time, now).is_ok());
    }

    #[test]
    fn tier_3_blocked_within_1h() {
        let i = identity(KycTier::Tier3);
        let now = SystemTime::now();
        let pub_time = now - Duration::from_secs(10);
        let err = CooldownEnforcer::check(&i, pub_time, now).unwrap_err();
        assert!(matches!(err, CooldownError::NotElapsed { .. }));
    }

    #[test]
    fn tier_3_passes_after_1h() {
        let i = identity(KycTier::Tier3);
        let now = SystemTime::now();
        let pub_time = now - Duration::from_secs(3_700);
        assert!(CooldownEnforcer::check(&i, pub_time, now).is_ok());
    }

    #[test]
    fn tier_0_no_cooldown() {
        let i = identity(KycTier::Tier0);
        let now = SystemTime::now();
        assert!(CooldownEnforcer::check(&i, now, now).is_ok());
    }

    #[test]
    fn tier_1_no_cooldown() {
        let i = identity(KycTier::Tier1);
        let now = SystemTime::now();
        assert!(CooldownEnforcer::check(&i, now, now).is_ok());
    }

    #[test]
    fn allowed_at_returns_published_plus_window() {
        let i = identity(KycTier::Tier2);
        let pub_time = SystemTime::now();
        let allowed = CooldownEnforcer::allowed_at(&i, pub_time);
        let diff = allowed.duration_since(pub_time).unwrap();
        assert_eq!(diff.as_secs(), 24 * 3_600);
    }

    #[test]
    fn remaining_seconds_decreases_over_time() {
        let i = identity(KycTier::Tier2);
        let now = SystemTime::now();
        let pub_time = now - Duration::from_secs(60); // 1 min ago
        let err = CooldownEnforcer::check(&i, pub_time, now).unwrap_err();
        match err {
            CooldownError::NotElapsed { remaining_seconds } => {
                assert!(remaining_seconds > 60);
                assert!(remaining_seconds < 24 * 3_600);
            }
        }
    }
}
