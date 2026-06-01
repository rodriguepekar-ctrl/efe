// SPDX-License-Identifier: UNLICENSED
//! **Sprint 10 — N18** : KYC provider abstraction.

use async_trait::async_trait;
use efe_nano_sdk::{KycTier, NewPublisherCandidate};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum KycError {
    #[error("KYC backend error : {message}")]
    Backend { message: String },

    #[error("dossier {dossier_reference} not found")]
    DossierNotFound { dossier_reference: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum KycVerdict {
    Passed {
        tier_granted: KycTier,
        dossier_reference: String,
    },
    Pending,
    Failed {
        reason: String,
    },
}

#[async_trait]
pub trait KycProvider: Send + Sync + 'static {
    async fn verify_identity(
        &self,
        candidate: &NewPublisherCandidate,
        target_tier: KycTier,
    ) -> Result<KycVerdict, KycError>;

    async fn refresh_status(
        &self,
        dossier_reference: &str,
    ) -> Result<KycVerdict, KycError>;
}

/// **Sprint 10 — N18** : stub that always passes.  Sprint 11 wires
/// real Persona / Onfido / Sumsub.
pub struct StubKycProvider {
    grant_tier: KycTier,
}

impl Default for StubKycProvider {
    fn default() -> Self {
        Self::permissive()
    }
}

impl StubKycProvider {
    pub fn permissive() -> Self {
        Self {
            grant_tier: KycTier::Tier3,
        }
    }

    pub fn grant_at(tier: KycTier) -> Self {
        Self { grant_tier: tier }
    }
}

#[async_trait]
impl KycProvider for StubKycProvider {
    async fn verify_identity(
        &self,
        _candidate: &NewPublisherCandidate,
        target_tier: KycTier,
    ) -> Result<KycVerdict, KycError> {
        if target_tier > self.grant_tier {
            return Ok(KycVerdict::Failed {
                reason: format!(
                    "stub only grants up to {:?}, requested {:?}",
                    self.grant_tier, target_tier
                ),
            });
        }
        Ok(KycVerdict::Passed {
            tier_granted: target_tier,
            dossier_reference: uuid::Uuid::new_v4().to_string(),
        })
    }

    async fn refresh_status(
        &self,
        dossier_reference: &str,
    ) -> Result<KycVerdict, KycError> {
        Ok(KycVerdict::Passed {
            tier_granted: self.grant_tier,
            dossier_reference: dossier_reference.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::{Iso3166, SignatureAlgorithm};

    fn candidate(tier: KycTier) -> NewPublisherCandidate {
        NewPublisherCandidate {
            display_name: "X".into(),
            target_tier: tier,
            jurisdiction: vec![Iso3166::new("US").unwrap()],
            initial_public_key_pem: "pem".into(),
            algorithm: SignatureAlgorithm::Ed25519,
            pseudonymous: false,
            kyc_dossier_reference: None,
        }
    }

    #[tokio::test]
    async fn stub_permissive_grants_requested_tier() {
        let p = StubKycProvider::permissive();
        let v = p.verify_identity(&candidate(KycTier::Tier2), KycTier::Tier2).await.unwrap();
        assert!(matches!(v, KycVerdict::Passed { tier_granted, .. } if tier_granted == KycTier::Tier2));
    }

    #[tokio::test]
    async fn stub_capped_rejects_above_grant() {
        let p = StubKycProvider::grant_at(KycTier::Tier1);
        let v = p.verify_identity(&candidate(KycTier::Tier3), KycTier::Tier3).await.unwrap();
        assert!(matches!(v, KycVerdict::Failed { .. }));
    }

    #[tokio::test]
    async fn refresh_returns_passed() {
        let p = StubKycProvider::permissive();
        let v = p.refresh_status("dossier-1").await.unwrap();
        assert!(matches!(v, KycVerdict::Passed { .. }));
    }

    #[test]
    fn verdict_serde_round_trip() {
        let cases = vec![
            KycVerdict::Passed {
                tier_granted: KycTier::Tier2,
                dossier_reference: "d".into(),
            },
            KycVerdict::Pending,
            KycVerdict::Failed {
                reason: "x".into(),
            },
        ];
        for v in cases {
            let json = serde_json::to_string(&v).unwrap();
            let v2: KycVerdict = serde_json::from_str(&json).unwrap();
            assert_eq!(v, v2);
        }
    }
}
