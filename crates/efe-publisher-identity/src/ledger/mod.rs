// SPDX-License-Identifier: UNLICENSED
//! **Sprint 10 — N18** : Publish ledger — HMAC-chained append-only
//! audit log of publish actions.
//!
//! Architecture § 7.4 + ADR-6.

use std::sync::Mutex;
use std::time::SystemTime;

use async_trait::async_trait;
use dashmap::DashMap;
use efe_nano_sdk::{
    KeyId, KycTier, OperatorSignature, PublisherId, PublisherIdentity, PublisherSignature,
    RevocationCertificate,
};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use thiserror::Error;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

/// **Sprint 10 — N18** : closed taxonomy of ledger errors.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum LedgerError {
    #[error("chain integrity broken at sequence {sequence_number}")]
    ChainBroken { sequence_number: u64 },

    #[error("backend : {message}")]
    Backend { message: String },
}

/// **Sprint 10 — N18** : ledger entry kinds.
///
/// `PublisherRegistered.identity` is boxed because `PublisherIdentity`
/// is ~384 bytes (driven by `Vec<HistoricalKey>` + `PublisherKey` +
/// `RevocationCertificate`) and the other variants are 8–170 bytes ;
/// boxing keeps the enum size down for the hot `AgentPublished` path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum PublishLedgerEntry {
    PublisherRegistered { identity: Box<PublisherIdentity> },
    AgentPublished {
        publisher_id: PublisherId,
        agent_id: String,
        version: String,
        signature: PublisherSignature,
        kyc_tier_snapshot: KycTier,
    },
    PublisherKeyRotated {
        publisher_id: PublisherId,
        previous_key_id: KeyId,
        new_key_id: KeyId,
    },
    PublisherSuspended { publisher_id: PublisherId, reason: String },
    PublisherRevoked {
        publisher_id: PublisherId,
        certificate: RevocationCertificate,
    },
    VersionRevoked {
        publisher_id: PublisherId,
        agent_id: String,
        version: String,
    },
    PublisherTierPromoted {
        publisher_id: PublisherId,
        from: KycTier,
        to: KycTier,
        approvers: Vec<OperatorSignature>,
    },
    /// **Sprint 12 — N20 cross-cutting** : an agent was promoted to
    /// `StabilityTier::Tier1` after passing (or bypassing) the N19
    /// gold-test gate.  This row is the audit anchor that lets
    /// compliance reconstruct the full promotion chain via
    /// `SELECT * FROM publish_ledger WHERE entry_kind = 'agent_
    /// promoted_to_tier_1'`.
    ///
    /// The `gold_run_id` joins to the N19 gold-run ledger ;
    /// `bypass: Some(...)` indicates the gate was skipped via the
    /// 2-of-3 emergency bypass workflow.  Catalog runtime's
    /// `promote_to(Tier1)` MUST append this row BEFORE flipping the
    /// `catalog.entries.stability_tier` column (see N20 § 8.2
    /// Postgres TRIGGER `enforce_tier_1_audit`).
    AgentPromotedToTier1 {
        publisher_id: PublisherId,
        agent_id: String,
        version: String,
        /// References the N19 `GoldRunCompleted` or `GoldRunBypassed`
        /// row's `gold_run_id`.  The publisher ledger does NOT join
        /// to the gold-run ledger by FK (different schemas, different
        /// regions in N16 setup) — the join is logical, not
        /// physical.
        gold_run_id: uuid::Uuid,
        /// Set when the promote used a `BypassCertificate`.  The
        /// catalog UI flags bypassed agents until the next clean
        /// gold run.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bypass: Option<efe_nano_sdk::BypassCertificate>,
    },
}

impl PublishLedgerEntry {
    /// **N18** : Get the subject publisher id of this entry.
    pub fn publisher_id(&self) -> PublisherId {
        match self {
            Self::PublisherRegistered { identity } => identity.publisher_id,
            Self::AgentPublished { publisher_id, .. } => *publisher_id,
            Self::PublisherKeyRotated { publisher_id, .. } => *publisher_id,
            Self::PublisherSuspended { publisher_id, .. } => *publisher_id,
            Self::PublisherRevoked { publisher_id, .. } => *publisher_id,
            Self::VersionRevoked { publisher_id, .. } => *publisher_id,
            Self::PublisherTierPromoted { publisher_id, .. } => *publisher_id,
            Self::AgentPromotedToTier1 { publisher_id, .. } => *publisher_id,
        }
    }

    /// **N18** : Stable label for metrics.
    pub fn kind_label(&self) -> &'static str {
        match self {
            Self::PublisherRegistered { .. } => "publisher_registered",
            Self::AgentPublished { .. } => "agent_published",
            Self::PublisherKeyRotated { .. } => "publisher_key_rotated",
            Self::PublisherSuspended { .. } => "publisher_suspended",
            Self::PublisherRevoked { .. } => "publisher_revoked",
            Self::VersionRevoked { .. } => "version_revoked",
            Self::PublisherTierPromoted { .. } => "publisher_tier_promoted",
            Self::AgentPromotedToTier1 { .. } => "agent_promoted_to_tier_1",
        }
    }
}

/// **Sprint 10 — N18** : persisted ledger row (canonical body + HMAC).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerRow {
    pub sequence_number: u64,
    pub trace_id: Uuid,
    pub publisher_id: PublisherId,
    pub entry: PublishLedgerEntry,
    pub canonical_body: Vec<u8>,
    pub hmac: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_trace_id: Option<Uuid>,
    pub created_at: SystemTime,
}

/// **Sprint 10 — N18** : ledger trait.
#[async_trait]
pub trait PublishLedger: Send + Sync + 'static {
    async fn append(
        &self,
        entry: PublishLedgerEntry,
    ) -> Result<LedgerRow, LedgerError>;

    async fn history(
        &self,
        publisher_id: PublisherId,
        limit: usize,
    ) -> Result<Vec<LedgerRow>, LedgerError>;

    async fn verify_chain(
        &self,
        publisher_id: PublisherId,
    ) -> Result<ChainVerifyReport, LedgerError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainVerifyReport {
    pub publisher_id: PublisherId,
    pub entries_verified: u64,
    pub head_trace_id: Option<Uuid>,
    pub ok: bool,
}

/// **Sprint 10 — N18** : in-memory ledger.
pub struct InMemoryPublishLedger {
    hmac_key: Vec<u8>,
    rows: DashMap<PublisherId, Mutex<Vec<LedgerRow>>>,
    next_seq: Mutex<u64>,
}

impl std::fmt::Debug for InMemoryPublishLedger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryPublishLedger")
            .field("hmac_key_len", &self.hmac_key.len())
            .finish()
    }
}

impl InMemoryPublishLedger {
    /// **N18** : Construct with a shared HMAC key.  Must be ≥ 32 bytes.
    pub fn new(hmac_key: impl Into<Vec<u8>>) -> Self {
        let hmac_key = hmac_key.into();
        assert!(
            hmac_key.len() >= 32,
            "ledger HMAC key must be ≥ 32 bytes (got {})",
            hmac_key.len()
        );
        Self {
            hmac_key,
            rows: DashMap::new(),
            next_seq: Mutex::new(1),
        }
    }

    /// **N18 + A4.2** : Construct from a pre-existing set of ledger
    /// rows (typically loaded from a JSON forensic dump).  The rows
    /// are grouped by `publisher_id` and inserted in their existing
    /// order ; `next_seq` is advanced past the highest seen sequence
    /// number so subsequent appends extend the chain rather than
    /// collide.
    ///
    /// Returns `Err` if `hmac_key.len() < 32`.
    pub fn with_seed_rows(
        hmac_key: impl Into<Vec<u8>>,
        rows: Vec<LedgerRow>,
    ) -> Result<Self, String> {
        let hmac_key = hmac_key.into();
        if hmac_key.len() < 32 {
            return Err(format!(
                "ledger HMAC key must be ≥ 32 bytes (got {})",
                hmac_key.len()
            ));
        }
        let store: DashMap<PublisherId, Mutex<Vec<LedgerRow>>> = DashMap::new();
        let mut max_seq: u64 = 0;
        for row in rows {
            max_seq = max_seq.max(row.sequence_number);
            store
                .entry(row.publisher_id)
                .or_insert_with(|| Mutex::new(Vec::new()))
                .value()
                .lock()
                .map_err(|e| format!("seed lock : {e}"))?
                .push(row);
        }
        // **N18 + I15** : Forensic dumps may arrive un-ordered (e.g.
        // when reconstructed from a Postgres scan or merged across
        // shards).  The HMAC chain depends on insertion order, so we
        // sort each publisher's rows by `sequence_number` before
        // verification can run.
        for entry in store.iter() {
            if let Ok(mut rows) = entry.value().lock() {
                rows.sort_by_key(|r| r.sequence_number);
            }
        }
        Ok(Self {
            hmac_key,
            rows: store,
            next_seq: Mutex::new(max_seq.saturating_add(1)),
        })
    }

    fn compute_hmac(&self, canonical_body: &[u8], prev: Option<&[u8]>) -> Vec<u8> {
        let mut mac = HmacSha256::new_from_slice(&self.hmac_key)
            .expect("HMAC key length already validated");
        if let Some(p) = prev {
            mac.update(p);
        }
        mac.update(canonical_body);
        mac.finalize().into_bytes().to_vec()
    }
}

#[async_trait]
impl PublishLedger for InMemoryPublishLedger {
    async fn append(
        &self,
        entry: PublishLedgerEntry,
    ) -> Result<LedgerRow, LedgerError> {
        let publisher_id = entry.publisher_id();
        let canonical_body = serde_json::to_vec(&entry).map_err(|e| {
            LedgerError::Backend {
                message: format!("serialize : {e}"),
            }
        })?;
        let seq = {
            let mut s = self.next_seq.lock().map_err(|e| LedgerError::Backend {
                message: e.to_string(),
            })?;
            let now = *s;
            *s += 1;
            now
        };
        // Ensure the publisher row vector exists, then drop the
        // DashMap entry guard before locking the inner Mutex.  Holding
        // both at once would block other shards needlessly.
        let _ = self
            .rows
            .entry(publisher_id)
            .or_insert_with(|| Mutex::new(Vec::new()));
        let guard = self.rows.get(&publisher_id).ok_or(LedgerError::Backend {
            message: "publisher row lock vanished".into(),
        })?;
        let mut rows = guard.value().lock().map_err(|e| LedgerError::Backend {
            message: e.to_string(),
        })?;
        let prev_hmac = rows.last().map(|r| r.hmac.clone());
        let prev_trace_id = rows.last().map(|r| r.trace_id);
        let hmac = self.compute_hmac(&canonical_body, prev_hmac.as_deref());
        let row = LedgerRow {
            sequence_number: seq,
            trace_id: Uuid::new_v4(),
            publisher_id,
            entry,
            canonical_body,
            hmac,
            prev_trace_id,
            created_at: SystemTime::now(),
        };
        rows.push(row.clone());
        Ok(row)
    }

    async fn history(
        &self,
        publisher_id: PublisherId,
        limit: usize,
    ) -> Result<Vec<LedgerRow>, LedgerError> {
        let r = self.rows.get(&publisher_id);
        match r {
            None => Ok(vec![]),
            Some(g) => {
                let rows = g.value().lock().map_err(|e| LedgerError::Backend {
                    message: e.to_string(),
                })?;
                Ok(rows.iter().take(limit).cloned().collect())
            }
        }
    }

    async fn verify_chain(
        &self,
        publisher_id: PublisherId,
    ) -> Result<ChainVerifyReport, LedgerError> {
        let r = self.rows.get(&publisher_id);
        let rows = match r {
            None => {
                return Ok(ChainVerifyReport {
                    publisher_id,
                    entries_verified: 0,
                    head_trace_id: None,
                    ok: true,
                });
            }
            Some(g) => g
                .value()
                .lock()
                .map_err(|e| LedgerError::Backend {
                    message: e.to_string(),
                })?
                .clone(),
        };
        let mut prev_hmac: Option<Vec<u8>> = None;
        for (i, row) in rows.iter().enumerate() {
            let expected =
                self.compute_hmac(&row.canonical_body, prev_hmac.as_deref());
            if expected != row.hmac {
                return Err(LedgerError::ChainBroken {
                    sequence_number: i as u64 + 1,
                });
            }
            prev_hmac = Some(row.hmac.clone());
        }
        Ok(ChainVerifyReport {
            publisher_id,
            entries_verified: rows.len() as u64,
            head_trace_id: rows.last().map(|r| r.trace_id),
            ok: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::{Iso3166, PublisherKey, PublisherStatus, SignatureAlgorithm};

    fn sample_identity(id: PublisherId, name: &str) -> PublisherIdentity {
        PublisherIdentity {
            publisher_id: id,
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

    #[test]
    #[should_panic]
    fn new_panics_on_short_key() {
        let _ = InMemoryPublishLedger::new(b"too-short".to_vec());
    }

    #[tokio::test]
    async fn append_then_verify_chain() {
        let ledger = InMemoryPublishLedger::new([7u8; 32]);
        let id = PublisherId::new();
        let entry = PublishLedgerEntry::PublisherRegistered {
            identity: Box::new(sample_identity(id, "ACME")),
        };
        ledger.append(entry).await.unwrap();
        let r = ledger.verify_chain(id).await.unwrap();
        assert!(r.ok);
        assert_eq!(r.entries_verified, 1);
    }

    #[tokio::test]
    async fn empty_chain_verifies_clean() {
        let ledger = InMemoryPublishLedger::new([7u8; 32]);
        let r = ledger.verify_chain(PublisherId::new()).await.unwrap();
        assert!(r.ok);
        assert_eq!(r.entries_verified, 0);
        assert!(r.head_trace_id.is_none());
    }

    #[tokio::test]
    async fn three_entry_chain_verifies() {
        let ledger = InMemoryPublishLedger::new([7u8; 32]);
        let id = PublisherId::new();
        ledger
            .append(PublishLedgerEntry::PublisherRegistered {
                identity: Box::new(sample_identity(id, "ACME")),
            })
            .await
            .unwrap();
        ledger
            .append(PublishLedgerEntry::PublisherSuspended {
                publisher_id: id,
                reason: "test".into(),
            })
            .await
            .unwrap();
        ledger
            .append(PublishLedgerEntry::PublisherSuspended {
                publisher_id: id,
                reason: "test2".into(),
            })
            .await
            .unwrap();
        let r = ledger.verify_chain(id).await.unwrap();
        assert!(r.ok);
        assert_eq!(r.entries_verified, 3);
    }

    #[tokio::test]
    async fn history_returns_entries_in_order() {
        let ledger = InMemoryPublishLedger::new([7u8; 32]);
        let id = PublisherId::new();
        ledger
            .append(PublishLedgerEntry::PublisherRegistered {
                identity: Box::new(sample_identity(id, "ACME")),
            })
            .await
            .unwrap();
        ledger
            .append(PublishLedgerEntry::PublisherSuspended {
                publisher_id: id,
                reason: "x".into(),
            })
            .await
            .unwrap();
        let h = ledger.history(id, 10).await.unwrap();
        assert_eq!(h.len(), 2);
        assert!(matches!(h[0].entry, PublishLedgerEntry::PublisherRegistered { .. }));
        assert!(matches!(h[1].entry, PublishLedgerEntry::PublisherSuspended { .. }));
    }

    #[tokio::test]
    async fn history_respects_limit() {
        let ledger = InMemoryPublishLedger::new([7u8; 32]);
        let id = PublisherId::new();
        for _ in 0..5 {
            ledger
                .append(PublishLedgerEntry::PublisherSuspended {
                    publisher_id: id,
                    reason: "x".into(),
                })
                .await
                .unwrap();
        }
        let h = ledger.history(id, 3).await.unwrap();
        assert_eq!(h.len(), 3);
    }

    #[tokio::test]
    async fn chain_breaks_on_tampered_hmac() {
        let ledger = InMemoryPublishLedger::new([7u8; 32]);
        let id = PublisherId::new();
        ledger
            .append(PublishLedgerEntry::PublisherRegistered {
                identity: Box::new(sample_identity(id, "ACME")),
            })
            .await
            .unwrap();
        // Tamper.
        {
            let g = ledger.rows.get(&id).unwrap();
            let mut rows = g.value().lock().unwrap();
            rows[0].hmac[0] ^= 0x01;
        }
        let err = ledger.verify_chain(id).await.unwrap_err();
        assert!(matches!(err, LedgerError::ChainBroken { .. }));
    }

    #[test]
    fn entry_publisher_id_extraction() {
        let id = PublisherId::new();
        let cases: Vec<PublishLedgerEntry> = vec![
            PublishLedgerEntry::PublisherRegistered {
                identity: Box::new(sample_identity(id, "X")),
            },
            PublishLedgerEntry::PublisherSuspended {
                publisher_id: id,
                reason: "x".into(),
            },
            PublishLedgerEntry::VersionRevoked {
                publisher_id: id,
                agent_id: "x".into(),
                version: "1".into(),
            },
        ];
        for c in cases {
            assert_eq!(c.publisher_id(), id);
        }
    }

    #[test]
    fn entry_kind_labels() {
        let id = PublisherId::new();
        let e = PublishLedgerEntry::PublisherSuspended {
            publisher_id: id,
            reason: "x".into(),
        };
        assert_eq!(e.kind_label(), "publisher_suspended");
    }
}
