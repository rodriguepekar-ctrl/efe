// SPDX-License-Identifier: UNLICENSED
//! **Sprint 11 — N19** : GoldRunLedger — § 7.6 + ADR-6 + ADR-10.
//!
//! Mirrors the N18 publish-ledger pattern exactly :
//! - HMAC-SHA256 chain (key ≥ 32 bytes),
//! - per-agent row vectors (DashMap<NanoAgentId, Mutex<Vec<...>>>),
//! - `prev_trace_id` linking each row to its predecessor,
//! - sequential `sequence_number` allocation under a shared Mutex.
//!
//! Entry kinds :
//! - `GoldRunCompleted { report }` — full gold run result.
//! - `GoldRunBypassed { ... }` — bypass usage event.
//! - `GoldDriftDetected { ... }` — drift sweep failure.
//! - `GoldSetApproved { ... }` — operator approval ceremony.

use std::sync::Mutex;
use std::time::SystemTime;

use async_trait::async_trait;
use dashmap::DashMap;
use efe_nano_sdk::{
    BypassCertificate, GoldReport, GoldRunId, GoldSetId, NanoAgentId, NanoAgentVersion,
    OperatorSignature,
};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use thiserror::Error;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

/// **N19** : closed taxonomy of ledger errors.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum GoldRunLedgerError {
    #[error("chain integrity broken at sequence {sequence_number}")]
    ChainBroken { sequence_number: u64 },

    #[error("backend : {message}")]
    Backend { message: String },
}

/// **N19** : the kinds of entries that land in the gold-run ledger.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GoldRunLedgerEntry {
    /// A gold gate run completed (passed, failed, errored, or
    /// aborted).
    GoldRunCompleted { report: Box<GoldReport> },

    /// An emergency bypass was used to promote an agent without a
    /// gold gate run.
    GoldRunBypassed {
        gold_run_id: GoldRunId,
        agent_id: NanoAgentId,
        agent_version: NanoAgentVersion,
        certificate: BypassCertificate,
    },

    /// The 30-day drift sweep demoted an agent.
    GoldDriftDetected {
        gold_run_id: GoldRunId,
        agent_id: NanoAgentId,
        agent_version: NanoAgentVersion,
        last_passing_at: SystemTime,
    },

    /// An operator approved a gold set (2-of-3 ceremony).
    GoldSetApproved {
        gold_set_id: GoldSetId,
        agent_id: NanoAgentId,
        approvers: Vec<OperatorSignature>,
        approved_at: SystemTime,
    },

    /// A gold set was archived.
    GoldSetArchived {
        gold_set_id: GoldSetId,
        agent_id: NanoAgentId,
        rationale: String,
    },
}

impl GoldRunLedgerEntry {
    /// **N19** : Subject agent id of this entry.
    pub fn agent_id(&self) -> NanoAgentId {
        match self {
            Self::GoldRunCompleted { report } => report.agent_id.clone(),
            Self::GoldRunBypassed { agent_id, .. } => agent_id.clone(),
            Self::GoldDriftDetected { agent_id, .. } => agent_id.clone(),
            Self::GoldSetApproved { agent_id, .. } => agent_id.clone(),
            Self::GoldSetArchived { agent_id, .. } => agent_id.clone(),
        }
    }

    /// **N19** : Stable label for metrics.
    pub fn kind_label(&self) -> &'static str {
        match self {
            Self::GoldRunCompleted { .. } => "gold_run_completed",
            Self::GoldRunBypassed { .. } => "gold_run_bypassed",
            Self::GoldDriftDetected { .. } => "gold_drift_detected",
            Self::GoldSetApproved { .. } => "gold_set_approved",
            Self::GoldSetArchived { .. } => "gold_set_archived",
        }
    }
}

/// **N19** : persisted ledger row (canonical body + HMAC).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldRunLedgerRow {
    pub sequence_number: u64,
    pub trace_id: Uuid,
    pub agent_id: NanoAgentId,
    pub entry: GoldRunLedgerEntry,
    pub canonical_body: Vec<u8>,
    pub hmac: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_trace_id: Option<Uuid>,
    pub created_at: SystemTime,
}

/// **N19** : chain-verification report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChainVerifyReport {
    pub agent_id: NanoAgentId,
    pub entries_verified: u64,
    pub head_trace_id: Option<Uuid>,
    pub ok: bool,
}

/// **N19** : the ledger trait.
#[async_trait]
pub trait GoldRunLedger: Send + Sync + 'static {
    async fn append(
        &self,
        entry: GoldRunLedgerEntry,
    ) -> Result<GoldRunLedgerRow, GoldRunLedgerError>;

    async fn history(
        &self,
        agent_id: &NanoAgentId,
        limit: usize,
    ) -> Result<Vec<GoldRunLedgerRow>, GoldRunLedgerError>;

    async fn verify_chain(
        &self,
        agent_id: &NanoAgentId,
    ) -> Result<ChainVerifyReport, GoldRunLedgerError>;

    /// **N19 + G14** : Look up a single ledger row by `gold_run_id`.
    /// The in-memory impl scans ; Sprint 12 Postgres adds an
    /// `(gold_run_id) UNIQUE` index for O(1) lookup.
    async fn find_by_gold_run_id(
        &self,
        gold_run_id: GoldRunId,
    ) -> Result<Option<GoldRunLedgerRow>, GoldRunLedgerError>;
}

// ─────────────────────────────────────────────────────────────────────
//  InMemoryGoldRunLedger
// ─────────────────────────────────────────────────────────────────────

/// **N19** : in-memory ledger.  Mirrors N18 InMemoryPublishLedger.
pub struct InMemoryGoldRunLedger {
    hmac_key: Vec<u8>,
    rows: DashMap<NanoAgentId, Mutex<Vec<GoldRunLedgerRow>>>,
    next_seq: Mutex<u64>,
}

impl std::fmt::Debug for InMemoryGoldRunLedger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryGoldRunLedger")
            .field("hmac_key_len", &self.hmac_key.len())
            .field("agents", &self.rows.len())
            .finish()
    }
}

impl InMemoryGoldRunLedger {
    /// **N19** : Construct.  HMAC key MUST be ≥ 32 bytes.
    pub fn new(hmac_key: impl Into<Vec<u8>>) -> Self {
        let hmac_key = hmac_key.into();
        assert!(
            hmac_key.len() >= 32,
            "gold-run ledger HMAC key must be ≥ 32 bytes (got {})",
            hmac_key.len()
        );
        Self {
            hmac_key,
            rows: DashMap::new(),
            next_seq: Mutex::new(1),
        }
    }

    /// **N19 + I15** : Construct from a pre-existing set of rows
    /// (forensic dump).  Rows are sorted per-agent by
    /// `sequence_number` after insertion.
    pub fn with_seed_rows(
        hmac_key: impl Into<Vec<u8>>,
        rows: Vec<GoldRunLedgerRow>,
    ) -> Result<Self, String> {
        let hmac_key = hmac_key.into();
        if hmac_key.len() < 32 {
            return Err(format!(
                "gold-run ledger HMAC key must be ≥ 32 bytes (got {})",
                hmac_key.len()
            ));
        }
        let store: DashMap<NanoAgentId, Mutex<Vec<GoldRunLedgerRow>>> = DashMap::new();
        let mut max_seq: u64 = 0;
        for row in rows {
            max_seq = max_seq.max(row.sequence_number);
            store
                .entry(row.agent_id.clone())
                .or_insert_with(|| Mutex::new(Vec::new()))
                .value()
                .lock()
                .map_err(|e| format!("seed lock : {e}"))?
                .push(row);
        }
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

    /// **N19** : Test-only — count rows in the chain for an agent.
    pub fn row_count(&self, agent_id: &NanoAgentId) -> usize {
        self.rows
            .get(agent_id)
            .map(|g| g.value().lock().map(|r| r.len()).unwrap_or(0))
            .unwrap_or(0)
    }
}

#[async_trait]
impl GoldRunLedger for InMemoryGoldRunLedger {
    async fn append(
        &self,
        entry: GoldRunLedgerEntry,
    ) -> Result<GoldRunLedgerRow, GoldRunLedgerError> {
        let agent_id = entry.agent_id();
        let canonical_body = serde_json::to_vec(&entry).map_err(|e| {
            GoldRunLedgerError::Backend {
                message: format!("serialize : {e}"),
            }
        })?;
        // Ensure the vec exists ; drop the DashMap guard before
        // locking the inner Mutex (same pattern as N18 ledger).
        let _ = self
            .rows
            .entry(agent_id.clone())
            .or_insert_with(|| Mutex::new(Vec::new()));
        let guard = self
            .rows
            .get(&agent_id)
            .ok_or(GoldRunLedgerError::Backend {
                message: "row lock vanished".into(),
            })?;
        let mut rows = guard.value().lock().map_err(|e| GoldRunLedgerError::Backend {
            message: e.to_string(),
        })?;
        // Hold the rows lock when allocating seq so seq order matches
        // push order (lesson from N18 audit pass A1 race).
        let seq = {
            let mut s = self.next_seq.lock().map_err(|e| GoldRunLedgerError::Backend {
                message: e.to_string(),
            })?;
            let now = *s;
            *s += 1;
            now
        };
        let prev_hmac = rows.last().map(|r| r.hmac.clone());
        let prev_trace_id = rows.last().map(|r| r.trace_id);
        let hmac = self.compute_hmac(&canonical_body, prev_hmac.as_deref());
        let row = GoldRunLedgerRow {
            sequence_number: seq,
            trace_id: Uuid::new_v4(),
            agent_id,
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
        agent_id: &NanoAgentId,
        limit: usize,
    ) -> Result<Vec<GoldRunLedgerRow>, GoldRunLedgerError> {
        match self.rows.get(agent_id) {
            None => Ok(vec![]),
            Some(g) => {
                let rows = g.value().lock().map_err(|e| GoldRunLedgerError::Backend {
                    message: e.to_string(),
                })?;
                Ok(rows.iter().take(limit).cloned().collect())
            }
        }
    }

    async fn verify_chain(
        &self,
        agent_id: &NanoAgentId,
    ) -> Result<ChainVerifyReport, GoldRunLedgerError> {
        let r = self.rows.get(agent_id);
        let rows = match r {
            None => {
                return Ok(ChainVerifyReport {
                    agent_id: agent_id.clone(),
                    entries_verified: 0,
                    head_trace_id: None,
                    ok: true,
                });
            }
            Some(g) => g
                .value()
                .lock()
                .map_err(|e| GoldRunLedgerError::Backend {
                    message: e.to_string(),
                })?
                .clone(),
        };
        let mut prev_hmac: Option<Vec<u8>> = None;
        for (i, row) in rows.iter().enumerate() {
            let expected =
                self.compute_hmac(&row.canonical_body, prev_hmac.as_deref());
            if expected != row.hmac {
                return Err(GoldRunLedgerError::ChainBroken {
                    sequence_number: i as u64 + 1,
                });
            }
            prev_hmac = Some(row.hmac.clone());
        }
        Ok(ChainVerifyReport {
            agent_id: agent_id.clone(),
            entries_verified: rows.len() as u64,
            head_trace_id: rows.last().map(|r| r.trace_id),
            ok: true,
        })
    }

    async fn find_by_gold_run_id(
        &self,
        gold_run_id: GoldRunId,
    ) -> Result<Option<GoldRunLedgerRow>, GoldRunLedgerError> {
        // In-memory full scan ; Sprint 12 Postgres adds an index.
        for entry in self.rows.iter() {
            let rows = entry.value().lock().map_err(|e| {
                GoldRunLedgerError::Backend {
                    message: e.to_string(),
                }
            })?;
            for row in rows.iter() {
                let row_gold_run_id = match &row.entry {
                    GoldRunLedgerEntry::GoldRunCompleted { report } => report.gold_run_id,
                    GoldRunLedgerEntry::GoldRunBypassed { gold_run_id, .. } => *gold_run_id,
                    GoldRunLedgerEntry::GoldDriftDetected { gold_run_id, .. } => *gold_run_id,
                    // The remaining variants are about gold sets, not
                    // runs ; they have no `gold_run_id` of their own.
                    GoldRunLedgerEntry::GoldSetApproved { .. }
                    | GoldRunLedgerEntry::GoldSetArchived { .. } => continue,
                };
                if row_gold_run_id == gold_run_id {
                    return Ok(Some(row.clone()));
                }
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::{
        GoldOverallVerdict, GoldReport, GoldRunId, GoldSetId, NanoAgentVersion, Sha256Hash,
    };
    use std::time::Duration;

    fn agent_id() -> NanoAgentId {
        NanoAgentId::new("test").unwrap()
    }

    fn report() -> GoldReport {
        GoldReport {
            gold_run_id: GoldRunId::new(),
            gold_set_id: GoldSetId::new(),
            agent_id: agent_id(),
            agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
            gold_set_hash: Sha256Hash::from_bytes([1u8; 32]),
            started_at: SystemTime::now(),
            completed_at: SystemTime::now() + Duration::from_secs(1),
            cases: vec![],
            overall: GoldOverallVerdict::Passed {
                cases_passed: 5,
                cases_total: 5,
            },
            total_cost_usd: 0.1,
            cost_cap_usd: 100.0,
            bypass: None,
        }
    }

    #[test]
    #[should_panic]
    fn new_panics_on_short_key() {
        let _ = InMemoryGoldRunLedger::new(b"short".to_vec());
    }

    #[tokio::test]
    async fn append_then_verify_chain() {
        let l = InMemoryGoldRunLedger::new([1u8; 32]);
        l.append(GoldRunLedgerEntry::GoldRunCompleted {
            report: Box::new(report()),
        })
        .await
        .unwrap();
        let r = l.verify_chain(&agent_id()).await.unwrap();
        assert!(r.ok);
        assert_eq!(r.entries_verified, 1);
    }

    #[tokio::test]
    async fn three_entry_chain_verifies() {
        let l = InMemoryGoldRunLedger::new([1u8; 32]);
        for _ in 0..3 {
            l.append(GoldRunLedgerEntry::GoldRunCompleted {
                report: Box::new(report()),
            })
            .await
            .unwrap();
        }
        let r = l.verify_chain(&agent_id()).await.unwrap();
        assert!(r.ok);
        assert_eq!(r.entries_verified, 3);
    }

    #[tokio::test]
    async fn history_respects_limit() {
        let l = InMemoryGoldRunLedger::new([1u8; 32]);
        for _ in 0..5 {
            l.append(GoldRunLedgerEntry::GoldRunCompleted {
                report: Box::new(report()),
            })
            .await
            .unwrap();
        }
        let h = l.history(&agent_id(), 3).await.unwrap();
        assert_eq!(h.len(), 3);
    }

    #[tokio::test]
    async fn empty_chain_verifies_clean() {
        let l = InMemoryGoldRunLedger::new([1u8; 32]);
        let r = l.verify_chain(&agent_id()).await.unwrap();
        assert!(r.ok);
        assert_eq!(r.entries_verified, 0);
        assert!(r.head_trace_id.is_none());
    }

    #[tokio::test]
    async fn chain_breaks_on_tampered_hmac() {
        let l = InMemoryGoldRunLedger::new([1u8; 32]);
        l.append(GoldRunLedgerEntry::GoldRunCompleted {
            report: Box::new(report()),
        })
        .await
        .unwrap();
        {
            let g = l.rows.get(&agent_id()).unwrap();
            let mut rows = g.value().lock().unwrap();
            rows[0].hmac[0] ^= 0x01;
        }
        let err = l.verify_chain(&agent_id()).await.unwrap_err();
        assert!(matches!(err, GoldRunLedgerError::ChainBroken { .. }));
    }

    #[tokio::test]
    async fn entry_kind_labels_distinct() {
        let l = InMemoryGoldRunLedger::new([2u8; 32]);
        l.append(GoldRunLedgerEntry::GoldRunCompleted {
            report: Box::new(report()),
        })
        .await
        .unwrap();
        l.append(GoldRunLedgerEntry::GoldSetArchived {
            gold_set_id: GoldSetId::new(),
            agent_id: agent_id(),
            rationale: "x".into(),
        })
        .await
        .unwrap();
        let h = l.history(&agent_id(), 10).await.unwrap();
        assert_eq!(h[0].entry.kind_label(), "gold_run_completed");
        assert_eq!(h[1].entry.kind_label(), "gold_set_archived");
    }

    #[tokio::test]
    async fn with_seed_rows_sorts_and_verifies() {
        let key = [3u8; 32];
        let l = InMemoryGoldRunLedger::new(key);
        for _ in 0..3 {
            l.append(GoldRunLedgerEntry::GoldRunCompleted {
                report: Box::new(report()),
            })
            .await
            .unwrap();
        }
        let mut h = l.history(&agent_id(), 100).await.unwrap();
        h.reverse();
        let reseeded = InMemoryGoldRunLedger::with_seed_rows(key, h).unwrap();
        let r = reseeded.verify_chain(&agent_id()).await.unwrap();
        assert!(r.ok);
        assert_eq!(r.entries_verified, 3);
    }

    #[tokio::test]
    async fn with_seed_rows_rejects_short_key() {
        let err = InMemoryGoldRunLedger::with_seed_rows(b"short".to_vec(), vec![]).unwrap_err();
        assert!(err.contains("≥ 32 bytes"));
    }

    #[tokio::test]
    async fn approval_entry_round_trips_through_chain() {
        let l = InMemoryGoldRunLedger::new([4u8; 32]);
        let entry = GoldRunLedgerEntry::GoldSetApproved {
            gold_set_id: GoldSetId::new(),
            agent_id: agent_id(),
            approvers: vec![OperatorSignature {
                operator_id: Uuid::new_v4(),
                signed_at: SystemTime::now(),
                ticket_id: "GOLD-7".into(),
                signature: vec![0u8; 64],
            }],
            approved_at: SystemTime::now(),
        };
        let row = l.append(entry).await.unwrap();
        assert_eq!(row.entry.kind_label(), "gold_set_approved");
    }

    /// **G3** : a `BypassCertificate` carried in the ledger is
    /// preserved byte-for-byte through the chain.
    #[tokio::test]
    async fn bypass_entry_round_trips() {
        let l = InMemoryGoldRunLedger::new([5u8; 32]);
        let cert = BypassCertificate {
            approvers: vec![],
            ticket_id: "INC-9117".into(),
            rationale: "x".repeat(64),
            valid_until: SystemTime::now() + Duration::from_secs(86_400),
        };
        l.append(GoldRunLedgerEntry::GoldRunBypassed {
            gold_run_id: GoldRunId::new(),
            agent_id: agent_id(),
            agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
            certificate: cert.clone(),
        })
        .await
        .unwrap();
        let h = l.history(&agent_id(), 10).await.unwrap();
        match &h[0].entry {
            GoldRunLedgerEntry::GoldRunBypassed { certificate, .. } => {
                assert_eq!(certificate.ticket_id, cert.ticket_id);
            }
            _ => panic!("expected bypass"),
        }
    }

    #[test]
    fn debug_format_no_panic() {
        let l = InMemoryGoldRunLedger::new([6u8; 32]);
        let _ = format!("{l:?}");
    }
}
