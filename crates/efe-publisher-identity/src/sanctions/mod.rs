// SPDX-License-Identifier: UNLICENSED
//! **Sprint 10 — N18** : Sanctions screening abstraction.

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::SystemTime;

use async_trait::async_trait;
use efe_nano_sdk::PublisherIdentity;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum SanctionsError {
    #[error("sanctions backend error : {message}")]
    Backend { message: String },
}

/// **Sprint 10 — N18** : sanctions list taxonomy (annex A1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "label")]
pub enum SanctionsList {
    OfacSdn,
    UnSecurityCouncil,
    EuConsolidated,
    UkOfsi,
    Other(String),
}

impl SanctionsList {
    pub fn label(&self) -> &str {
        match self {
            Self::OfacSdn => "ofac_sdn",
            Self::UnSecurityCouncil => "un_security_council",
            Self::EuConsolidated => "eu_consolidated",
            Self::UkOfsi => "uk_ofsi",
            Self::Other(s) => s,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SanctionsVerdict {
    Clear,
    Flagged {
        source: SanctionsList,
        matched_at: SystemTime,
    },
    Inconclusive,
}

impl SanctionsVerdict {
    pub fn is_clear(&self) -> bool {
        matches!(self, Self::Clear)
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Clear => "clear",
            Self::Flagged { .. } => "flagged",
            Self::Inconclusive => "inconclusive",
        }
    }
}

#[async_trait]
pub trait SanctionsScreener: Send + Sync + 'static {
    async fn check(
        &self,
        identity: &PublisherIdentity,
    ) -> Result<SanctionsVerdict, SanctionsError>;
}

/// **Sprint 10 — N18** : stub screener with operator-injected flags.
pub struct StubSanctionsScreener {
    flagged_names: Mutex<HashSet<String>>,
    /// When `true`, every check returns `Inconclusive` (simulates the
    /// service-down scenario for F4 testing).
    force_inconclusive: std::sync::atomic::AtomicBool,
}

impl Default for StubSanctionsScreener {
    fn default() -> Self {
        Self::clear()
    }
}

impl std::fmt::Debug for StubSanctionsScreener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StubSanctionsScreener")
            .field(
                "flagged_count",
                &self.flagged_names.lock().map(|l| l.len()).unwrap_or(0),
            )
            .finish()
    }
}

impl StubSanctionsScreener {
    pub fn clear() -> Self {
        Self {
            flagged_names: Mutex::new(HashSet::new()),
            force_inconclusive: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn flag(&self, display_name: impl Into<String>) {
        if let Ok(mut s) = self.flagged_names.lock() {
            s.insert(display_name.into());
        }
    }

    pub fn unflag(&self, display_name: &str) {
        if let Ok(mut s) = self.flagged_names.lock() {
            s.remove(display_name);
        }
    }

    pub fn set_force_inconclusive(&self, on: bool) {
        self.force_inconclusive
            .store(on, std::sync::atomic::Ordering::SeqCst);
    }
}

#[async_trait]
impl SanctionsScreener for StubSanctionsScreener {
    async fn check(
        &self,
        identity: &PublisherIdentity,
    ) -> Result<SanctionsVerdict, SanctionsError> {
        if self
            .force_inconclusive
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Ok(SanctionsVerdict::Inconclusive);
        }
        let flagged = self
            .flagged_names
            .lock()
            .map(|s| s.contains(&identity.display_name))
            .unwrap_or(false);
        if flagged {
            Ok(SanctionsVerdict::Flagged {
                source: SanctionsList::OfacSdn,
                matched_at: SystemTime::now(),
            })
        } else {
            Ok(SanctionsVerdict::Clear)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::{
        Iso3166, KeyId, KycTier, PublisherId, PublisherKey, PublisherStatus,
        SignatureAlgorithm,
    };

    fn identity(name: &str) -> PublisherIdentity {
        PublisherIdentity {
            publisher_id: PublisherId::new(),
            display_name: name.into(),
            kyc_tier: KycTier::Tier2,
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

    #[tokio::test]
    async fn unflagged_publisher_clear() {
        let s = StubSanctionsScreener::clear();
        let v = s.check(&identity("ACME")).await.unwrap();
        assert!(v.is_clear());
    }

    #[tokio::test]
    async fn flagged_publisher_returns_flagged() {
        let s = StubSanctionsScreener::clear();
        s.flag("ACME Corp");
        let v = s.check(&identity("ACME Corp")).await.unwrap();
        assert!(matches!(v, SanctionsVerdict::Flagged { .. }));
    }

    #[tokio::test]
    async fn unflagged_via_unflag_returns_clear() {
        let s = StubSanctionsScreener::clear();
        s.flag("X");
        s.unflag("X");
        let v = s.check(&identity("X")).await.unwrap();
        assert!(v.is_clear());
    }

    #[tokio::test]
    async fn force_inconclusive_simulates_outage() {
        let s = StubSanctionsScreener::clear();
        s.set_force_inconclusive(true);
        let v = s.check(&identity("ACME")).await.unwrap();
        assert!(matches!(v, SanctionsVerdict::Inconclusive));
    }

    #[test]
    fn sanctions_list_label_stable() {
        assert_eq!(SanctionsList::OfacSdn.label(), "ofac_sdn");
        assert_eq!(SanctionsList::Other("custom".into()).label(), "custom");
    }

    #[test]
    fn verdict_serde_round_trip() {
        let cases = vec![
            SanctionsVerdict::Clear,
            SanctionsVerdict::Flagged {
                source: SanctionsList::OfacSdn,
                matched_at: SystemTime::now(),
            },
            SanctionsVerdict::Inconclusive,
        ];
        for v in cases {
            let json = serde_json::to_string(&v).unwrap();
            let v2: SanctionsVerdict = serde_json::from_str(&json).unwrap();
            assert_eq!(v.label(), v2.label());
        }
    }
}
