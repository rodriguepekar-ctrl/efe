// SPDX-License-Identifier: UNLICENSED
//! **Sprint 11 — N19** : GoldSet registry — § 7.3.
//!
//! Lifecycle :
//!
//! ```text
//! register (Draft) → approve (Approved) → archive (Archived)
//! ```
//!
//! Architecture G2 + I1 + I3 :
//! - **Approval requires ≥ 2 OperatorSignatures** (no single-eng
//!   bypass).
//! - **Approved sets are immutable** — edits create a NEW set (new
//!   `GoldSetId`).
//! - **Archived sets are forever queryable** for forensic audit
//!   (I13 N18-style).
//!
//! The in-memory impl uses `DashMap`, mirrors the N18 publisher
//! registry pattern.  Sprint 12 swaps in `PostgresGoldSetRegistry`.

use std::sync::Mutex;
use std::time::SystemTime;

use async_trait::async_trait;
use dashmap::DashMap;
use efe_nano_sdk::{
    GoldSet, GoldSetError, GoldSetId, GoldSetStatus, NanoAgentId, OperatorSignature,
    GOLD_SET_MIN_APPROVERS,
};
use thiserror::Error;

/// **N19** : closed taxonomy of registry-level errors.  Mirrors
/// `GoldSetError` but carries some additional storage-layer
/// variants ; we re-export `GoldSetError` from the SDK as the
/// canonical type, and this enum is what the trait surfaces.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum GoldSetRegistryError {
    #[error("gold set {gold_set_id} not found")]
    NotFound { gold_set_id: GoldSetId },

    #[error("gold set {gold_set_id} is already in status {status:?}")]
    InvalidTransition {
        gold_set_id: GoldSetId,
        status: GoldSetStatus,
    },

    #[error("insufficient approvers : got {got}, need {need}")]
    InsufficientApprovers { got: u32, need: u32 },

    #[error("operator id {operator_id} appears twice in approver list")]
    DuplicateApprover { operator_id: uuid::Uuid },

    #[error("validation : {0}")]
    Validation(#[from] GoldSetError),

    #[error("backend : {message}")]
    Backend { message: String },
}

/// **N19** : the registry abstraction (§ 7.3).
#[async_trait]
pub trait GoldSetRegistry: Send + Sync + 'static {
    /// **N19 + G1** : Register a fresh `Draft` gold set.
    ///
    /// The caller passes `pii_required_by_compliance` resolved from
    /// the agent's compliance tags (the registry can't introspect
    /// the catalog).
    async fn register(
        &self,
        draft: GoldSet,
        pii_required_by_compliance: bool,
    ) -> Result<GoldSetId, GoldSetRegistryError>;

    /// **N19 + G2** : Approve a draft.  Requires ≥ 2 distinct
    /// `OperatorSignature`s.
    async fn approve(
        &self,
        gold_set_id: GoldSetId,
        approvers: Vec<OperatorSignature>,
    ) -> Result<(), GoldSetRegistryError>;

    /// **N19 + I11** : Archive a set.  Preserved forever for
    /// forensic queries ; cannot be used as a gate.
    async fn archive(
        &self,
        gold_set_id: GoldSetId,
        rationale: String,
    ) -> Result<(), GoldSetRegistryError>;

    /// **N19** : Look up by id.
    async fn lookup(
        &self,
        gold_set_id: GoldSetId,
    ) -> Result<GoldSet, GoldSetRegistryError>;

    /// **N19** : Latest `Approved` set for a given `(agent_id,
    /// version)`.
    async fn latest_approved_for(
        &self,
        agent_id: &NanoAgentId,
        version: &efe_nano_sdk::NanoAgentVersion,
    ) -> Result<Option<GoldSet>, GoldSetRegistryError>;

    /// **N19** : List all sets for an agent (any status).
    async fn list_for_agent(
        &self,
        agent_id: &NanoAgentId,
    ) -> Result<Vec<GoldSet>, GoldSetRegistryError>;
}

/// **N19** : in-memory implementation.
pub struct InMemoryGoldSetRegistry {
    sets: DashMap<GoldSetId, GoldSet>,
    /// Serialises lifecycle transitions per id (defense against
    /// concurrent approve / archive races).
    txn_lock: Mutex<()>,
}

impl std::fmt::Debug for InMemoryGoldSetRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryGoldSetRegistry")
            .field("sets", &self.sets.len())
            .finish()
    }
}

impl Default for InMemoryGoldSetRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryGoldSetRegistry {
    pub fn new() -> Self {
        Self {
            sets: DashMap::new(),
            txn_lock: Mutex::new(()),
        }
    }

    pub fn len(&self) -> usize {
        self.sets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }
}

#[async_trait]
impl GoldSetRegistry for InMemoryGoldSetRegistry {
    async fn register(
        &self,
        mut draft: GoldSet,
        pii_required_by_compliance: bool,
    ) -> Result<GoldSetId, GoldSetRegistryError> {
        draft.validate(pii_required_by_compliance)?;
        if !matches!(draft.status, GoldSetStatus::Draft) {
            return Err(GoldSetRegistryError::InvalidTransition {
                gold_set_id: draft.gold_set_id,
                status: draft.status,
            });
        }
        // Force created_at to "now" to prevent backdating.
        draft.created_at = SystemTime::now();
        draft.approved_at = None;
        draft.approvers.clear();
        let id = draft.gold_set_id;
        self.sets.insert(id, draft);
        Ok(id)
    }

    async fn approve(
        &self,
        gold_set_id: GoldSetId,
        approvers: Vec<OperatorSignature>,
    ) -> Result<(), GoldSetRegistryError> {
        if approvers.len() < GOLD_SET_MIN_APPROVERS {
            return Err(GoldSetRegistryError::InsufficientApprovers {
                got: approvers.len() as u32,
                need: GOLD_SET_MIN_APPROVERS as u32,
            });
        }
        // Approvers must be distinct (two-person rule).
        let mut ids: Vec<uuid::Uuid> = approvers.iter().map(|a| a.operator_id).collect();
        ids.sort();
        let pre = ids.len();
        ids.dedup();
        if ids.len() != pre {
            // Find the duplicated id (first repeat).
            let dup = approvers
                .iter()
                .enumerate()
                .find_map(|(i, a)| {
                    if approvers[..i].iter().any(|b| b.operator_id == a.operator_id) {
                        Some(a.operator_id)
                    } else {
                        None
                    }
                })
                .unwrap_or(uuid::Uuid::nil());
            return Err(GoldSetRegistryError::DuplicateApprover { operator_id: dup });
        }

        let _guard = self.txn_lock.lock().map_err(|e| GoldSetRegistryError::Backend {
            message: e.to_string(),
        })?;
        let mut entry = self
            .sets
            .get_mut(&gold_set_id)
            .ok_or(GoldSetRegistryError::NotFound { gold_set_id })?;
        match entry.status {
            GoldSetStatus::Draft => {
                entry.status = GoldSetStatus::Approved;
                entry.approvers = approvers;
                entry.approved_at = Some(SystemTime::now());
                Ok(())
            }
            other => Err(GoldSetRegistryError::InvalidTransition {
                gold_set_id,
                status: other,
            }),
        }
    }

    async fn archive(
        &self,
        gold_set_id: GoldSetId,
        rationale: String,
    ) -> Result<(), GoldSetRegistryError> {
        let _ = rationale; // Sprint 12 stores in Postgres ; Sprint 11 in-memory drops.
        let _guard = self.txn_lock.lock().map_err(|e| GoldSetRegistryError::Backend {
            message: e.to_string(),
        })?;
        let mut entry = self
            .sets
            .get_mut(&gold_set_id)
            .ok_or(GoldSetRegistryError::NotFound { gold_set_id })?;
        if matches!(entry.status, GoldSetStatus::Archived) {
            return Err(GoldSetRegistryError::InvalidTransition {
                gold_set_id,
                status: entry.status,
            });
        }
        entry.status = GoldSetStatus::Archived;
        Ok(())
    }

    async fn lookup(
        &self,
        gold_set_id: GoldSetId,
    ) -> Result<GoldSet, GoldSetRegistryError> {
        self.sets
            .get(&gold_set_id)
            .map(|r| r.value().clone())
            .ok_or(GoldSetRegistryError::NotFound { gold_set_id })
    }

    async fn latest_approved_for(
        &self,
        agent_id: &NanoAgentId,
        version: &efe_nano_sdk::NanoAgentVersion,
    ) -> Result<Option<GoldSet>, GoldSetRegistryError> {
        let mut candidates: Vec<GoldSet> = self
            .sets
            .iter()
            .filter(|r| {
                matches!(r.value().status, GoldSetStatus::Approved)
                    && r.value().agent_id == *agent_id
                    && r.value().agent_version == *version
            })
            .map(|r| r.value().clone())
            .collect();
        candidates.sort_by_key(|s| std::cmp::Reverse(s.approved_at.unwrap_or(s.created_at)));
        Ok(candidates.into_iter().next())
    }

    async fn list_for_agent(
        &self,
        agent_id: &NanoAgentId,
    ) -> Result<Vec<GoldSet>, GoldSetRegistryError> {
        let mut out: Vec<GoldSet> = self
            .sets
            .iter()
            .filter(|r| r.value().agent_id == *agent_id)
            .map(|r| r.value().clone())
            .collect();
        // Deterministic order : status first (Approved first), then
        // creation desc.
        out.sort_by(|a, b| {
            status_rank(a.status)
                .cmp(&status_rank(b.status))
                .then_with(|| b.created_at.cmp(&a.created_at))
        });
        Ok(out)
    }
}

fn status_rank(s: GoldSetStatus) -> u8 {
    match s {
        GoldSetStatus::Approved => 0,
        GoldSetStatus::Draft => 1,
        GoldSetStatus::Archived => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::{
        GoldAssertionStrategy, GoldCase, GoldCaseId, GoldExpectedOutput, NanoAgentVersion,
        Provenance, Sha256Hash,
    };
    use uuid::Uuid;

    fn op_sig(id: Uuid) -> OperatorSignature {
        OperatorSignature {
            operator_id: id,
            signed_at: SystemTime::now(),
            ticket_id: "GOLD-1".into(),
            signature: vec![0u8; 64],
        }
    }

    fn draft(name: &str) -> GoldSet {
        let cases: Vec<GoldCase> = (0..5)
            .map(|i| GoldCase {
                gold_case_id: GoldCaseId::new(),
                name: format!("c{i}"),
                input: serde_json::json!({"i": i}),
                expected: GoldExpectedOutput::Inline {
                    value: serde_json::json!({"o": i}),
                },
                assertion: GoldAssertionStrategy::StrictEquality,
                timeout_seconds: 30,
                tags: vec![],
            })
            .collect();
        GoldSet {
            gold_set_id: GoldSetId::new(),
            agent_id: NanoAgentId::new(name).unwrap(),
            agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
            agent_input_schema_hash: Sha256Hash::from_bytes([1u8; 32]),
            agent_output_schema_hash: Sha256Hash::from_bytes([2u8; 32]),
            cases,
            cost_cap_usd: 100.0,
            provenance: Provenance::Manual,
            status: GoldSetStatus::Draft,
            approvers: vec![],
            created_at: SystemTime::now(),
            approved_at: None,
            gold_set_hash: Sha256Hash::from_bytes([0u8; 32]),
        }
        .with_computed_hash()
    }

    #[tokio::test]
    async fn register_then_lookup() {
        let r = InMemoryGoldSetRegistry::new();
        let id = r.register(draft("a"), false).await.unwrap();
        let got = r.lookup(id).await.unwrap();
        assert_eq!(got.gold_set_id, id);
    }

    #[tokio::test]
    async fn register_rejects_too_few_cases() {
        let r = InMemoryGoldSetRegistry::new();
        let mut d = draft("a");
        d.cases.truncate(3);
        let err = r.register(d, false).await.unwrap_err();
        assert!(matches!(
            err,
            GoldSetRegistryError::Validation(GoldSetError::TooFewCases { .. })
        ));
    }

    #[tokio::test]
    async fn approve_with_two_signatures_passes() {
        let r = InMemoryGoldSetRegistry::new();
        let id = r.register(draft("a"), false).await.unwrap();
        r.approve(
            id,
            vec![op_sig(Uuid::new_v4()), op_sig(Uuid::new_v4())],
        )
        .await
        .unwrap();
        let got = r.lookup(id).await.unwrap();
        assert!(matches!(got.status, GoldSetStatus::Approved));
        assert_eq!(got.approvers.len(), 2);
        assert!(got.approved_at.is_some());
    }

    #[tokio::test]
    async fn approve_rejects_single_signature() {
        let r = InMemoryGoldSetRegistry::new();
        let id = r.register(draft("a"), false).await.unwrap();
        let err = r.approve(id, vec![op_sig(Uuid::new_v4())]).await.unwrap_err();
        assert!(matches!(
            err,
            GoldSetRegistryError::InsufficientApprovers { .. }
        ));
    }

    #[tokio::test]
    async fn approve_rejects_duplicate_approver() {
        let r = InMemoryGoldSetRegistry::new();
        let id = r.register(draft("a"), false).await.unwrap();
        let dup = Uuid::new_v4();
        let err = r.approve(id, vec![op_sig(dup), op_sig(dup)]).await.unwrap_err();
        assert!(matches!(
            err,
            GoldSetRegistryError::DuplicateApprover { .. }
        ));
    }

    #[tokio::test]
    async fn approve_unknown_set_fails() {
        let r = InMemoryGoldSetRegistry::new();
        let err = r
            .approve(
                GoldSetId::new(),
                vec![op_sig(Uuid::new_v4()), op_sig(Uuid::new_v4())],
            )
            .await
            .unwrap_err();
        assert!(matches!(err, GoldSetRegistryError::NotFound { .. }));
    }

    #[tokio::test]
    async fn double_approve_fails() {
        let r = InMemoryGoldSetRegistry::new();
        let id = r.register(draft("a"), false).await.unwrap();
        let two = vec![op_sig(Uuid::new_v4()), op_sig(Uuid::new_v4())];
        r.approve(id, two.clone()).await.unwrap();
        let err = r.approve(id, two).await.unwrap_err();
        assert!(matches!(
            err,
            GoldSetRegistryError::InvalidTransition { .. }
        ));
    }

    #[tokio::test]
    async fn archive_changes_status() {
        let r = InMemoryGoldSetRegistry::new();
        let id = r.register(draft("a"), false).await.unwrap();
        r.archive(id, "test".into()).await.unwrap();
        let got = r.lookup(id).await.unwrap();
        assert!(matches!(got.status, GoldSetStatus::Archived));
    }

    #[tokio::test]
    async fn archive_twice_fails() {
        let r = InMemoryGoldSetRegistry::new();
        let id = r.register(draft("a"), false).await.unwrap();
        r.archive(id, "test".into()).await.unwrap();
        let err = r.archive(id, "again".into()).await.unwrap_err();
        assert!(matches!(
            err,
            GoldSetRegistryError::InvalidTransition { .. }
        ));
    }

    #[tokio::test]
    async fn latest_approved_for_returns_most_recent() {
        let r = InMemoryGoldSetRegistry::new();
        let id1 = r.register(draft("a"), false).await.unwrap();
        let id2 = r.register(draft("a"), false).await.unwrap();
        r.approve(
            id1,
            vec![op_sig(Uuid::new_v4()), op_sig(Uuid::new_v4())],
        )
        .await
        .unwrap();
        // Sleep-equivalent : approve id2 second (later approved_at).
        r.approve(
            id2,
            vec![op_sig(Uuid::new_v4()), op_sig(Uuid::new_v4())],
        )
        .await
        .unwrap();
        let latest = r
            .latest_approved_for(
                &NanoAgentId::new("a").unwrap(),
                &NanoAgentVersion::new("1.0.0").unwrap(),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latest.gold_set_id, id2);
    }

    #[tokio::test]
    async fn latest_approved_for_excludes_drafts_and_archived() {
        let r = InMemoryGoldSetRegistry::new();
        let id_draft = r.register(draft("a"), false).await.unwrap();
        let _ = id_draft;
        let id_archived = r.register(draft("a"), false).await.unwrap();
        r.archive(id_archived, "x".into()).await.unwrap();
        let latest = r
            .latest_approved_for(
                &NanoAgentId::new("a").unwrap(),
                &NanoAgentVersion::new("1.0.0").unwrap(),
            )
            .await
            .unwrap();
        assert!(latest.is_none());
    }

    #[tokio::test]
    async fn list_for_agent_includes_all_statuses_sorted() {
        let r = InMemoryGoldSetRegistry::new();
        let _ = r.register(draft("a"), false).await.unwrap();
        let id_approved = r.register(draft("a"), false).await.unwrap();
        r.approve(
            id_approved,
            vec![op_sig(Uuid::new_v4()), op_sig(Uuid::new_v4())],
        )
        .await
        .unwrap();
        let id_archived = r.register(draft("a"), false).await.unwrap();
        r.archive(id_archived, "x".into()).await.unwrap();
        let all = r
            .list_for_agent(&NanoAgentId::new("a").unwrap())
            .await
            .unwrap();
        assert_eq!(all.len(), 3);
        assert!(matches!(all[0].status, GoldSetStatus::Approved));
    }

    #[tokio::test]
    async fn lookup_unknown_returns_not_found() {
        let r = InMemoryGoldSetRegistry::new();
        let err = r.lookup(GoldSetId::new()).await.unwrap_err();
        assert!(matches!(err, GoldSetRegistryError::NotFound { .. }));
    }

    #[tokio::test]
    async fn debug_format_no_panic() {
        let r = InMemoryGoldSetRegistry::new();
        let _ = format!("{r:?}");
    }
}
