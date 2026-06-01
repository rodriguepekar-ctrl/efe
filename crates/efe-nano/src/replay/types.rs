//! Replay types : `ReplayReport`, `PerOpReplay`, `ReplayOpStatus`.

use chrono::{DateTime, Utc};
use efe_nano_sdk::{SwarmResponse, TenantId};
use std::collections::HashMap;
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────
//  ReplayReport
// ─────────────────────────────────────────────────────────────────────

/// Result of a swarm replay attempt.
#[derive(Debug, Clone)]
pub struct ReplayReport {
    pub request_id: Uuid,
    pub tenant: TenantId,
    /// HMAC chain integrity result : true if the chain verified end-to-end.
    pub chain_verified: bool,
    pub total_ops: u32,
    pub matched_ops: u32,
    pub mismatched_ops: u32,
    pub skipped_non_deterministic_ops: u32,
    /// **Sprint 7 — N3** : count of `LlmReplayable` ops whose cache
    /// entry expired between original call and replay attempt.
    pub replay_window_expired_ops: u32,
    pub failed_ops: u32,
    /// Per-op detail.
    pub per_op: Vec<PerOpReplay>,
    /// The fresh response produced by re-dispatching the swarm.  None
    /// if the dispatch failed (failure detail in `dispatch_error`).
    pub new_response: Option<SwarmResponse>,
    pub dispatch_error: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub duration_ms: u32,
}

impl ReplayReport {
    /// All deterministic ops re-matched their original output_hash.
    ///
    /// **Sprint 7 — N3** : `ReplayWindowExpired` counts as
    /// non-consistent — the chain integrity holds but the regulatory
    /// replay guarantee is no longer provable for the expired op.
    pub fn is_fully_consistent(&self) -> bool {
        self.chain_verified
            && self.mismatched_ops == 0
            && self.failed_ops == 0
            && self.replay_window_expired_ops == 0
            && (self.matched_ops + self.skipped_non_deterministic_ops) == self.total_ops
    }

    /// Build per-op map for quick lookup.
    pub fn per_op_map(&self) -> HashMap<String, &PerOpReplay> {
        self.per_op.iter().map(|p| (p.op_id.clone(), p)).collect()
    }
}

// ─────────────────────────────────────────────────────────────────────
//  PerOpReplay
// ─────────────────────────────────────────────────────────────────────

/// Per-op replay result.
#[derive(Debug, Clone)]
pub struct PerOpReplay {
    pub op_id: String,
    pub nano_agent_id: String,
    pub execution_mode: String,
    pub original_output_hash: Option<[u8; 32]>,
    pub replay_output_hash: Option<[u8; 32]>,
    pub status: ReplayOpStatus,
    pub explanation: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────
//  ReplayOpStatus
// ─────────────────────────────────────────────────────────────────────

/// Outcome of a single op's replay.
///
/// **Sprint 7 — N3** : adds `ReplayWindowExpired` for the LlmReplayable
/// case where the original LLM response was cached but the cache entry
/// has been evicted (TTL passed).  Distinct from `SkippedNonDeterministic`
/// which means "intentionally not replayable" and from `ReplayFailed`
/// which means "infrastructure error during replay".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayOpStatus {
    /// Replay output_hash matches original (FullDeterministic, Seeded,
    /// or LlmReplayable with cache hit).
    Match,
    /// Hashes differ — possible determinism drift.
    Mismatch,
    /// Agent is `DeterminismLevel::LlmEphemeral` or `SideEffect` ;
    /// we skipped comparison by design.
    SkippedNonDeterministic,
    /// **Sprint 7 — N3** : Agent is `DeterminismLevel::LlmReplayable`
    /// but the cached LLM response could not be retrieved (window
    /// expired, cache evicted, or cache backend unavailable).
    /// Auditor-visible signal that the trace is **no longer replayable**
    /// — distinct from `SkippedNonDeterministic` (intentional opt-out).
    ReplayWindowExpired,
    /// Replay dispatch itself failed for this op.
    ReplayFailed,
}

impl ReplayOpStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Match => "match",
            Self::Mismatch => "mismatch",
            Self::SkippedNonDeterministic => "skipped_non_deterministic",
            Self::ReplayWindowExpired => "replay_window_expired",
            Self::ReplayFailed => "replay_failed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_report() -> ReplayReport {
        ReplayReport {
            request_id: Uuid::nil(),
            tenant: TenantId::system(),
            chain_verified: true,
            total_ops: 0,
            matched_ops: 0,
            mismatched_ops: 0,
            skipped_non_deterministic_ops: 0,
            replay_window_expired_ops: 0,
            failed_ops: 0,
            per_op: vec![],
            new_response: None,
            dispatch_error: None,
            started_at: Utc::now(),
            completed_at: Utc::now(),
            duration_ms: 0,
        }
    }

    #[test]
    fn fully_consistent_empty_report() {
        let r = empty_report();
        assert!(r.is_fully_consistent());
    }

    #[test]
    fn not_consistent_when_mismatched() {
        let mut r = empty_report();
        r.total_ops = 1;
        r.mismatched_ops = 1;
        assert!(!r.is_fully_consistent());
    }

    #[test]
    fn not_consistent_when_chain_unverified() {
        let mut r = empty_report();
        r.chain_verified = false;
        assert!(!r.is_fully_consistent());
    }

    #[test]
    fn consistent_when_all_matched_or_skipped() {
        let mut r = empty_report();
        r.total_ops = 5;
        r.matched_ops = 3;
        r.skipped_non_deterministic_ops = 2;
        assert!(r.is_fully_consistent());
    }

    #[test]
    fn op_status_str() {
        assert_eq!(ReplayOpStatus::Match.as_str(), "match");
        assert_eq!(ReplayOpStatus::Mismatch.as_str(), "mismatch");
        assert_eq!(
            ReplayOpStatus::SkippedNonDeterministic.as_str(),
            "skipped_non_deterministic"
        );
        assert_eq!(
            ReplayOpStatus::ReplayWindowExpired.as_str(),
            "replay_window_expired"
        );
        assert_eq!(ReplayOpStatus::ReplayFailed.as_str(), "replay_failed");
    }

    #[test]
    fn replay_window_expired_counts_as_inconsistent() {
        let mut r = empty_report();
        r.total_ops = 1;
        r.replay_window_expired_ops = 1;
        assert!(!r.is_fully_consistent());
    }

    #[test]
    fn per_op_map_lookup() {
        let mut r = empty_report();
        r.per_op.push(PerOpReplay {
            op_id: "a".into(),
            nano_agent_id: "x".into(),
            execution_mode: "pure_rust".into(),
            original_output_hash: None,
            replay_output_hash: None,
            status: ReplayOpStatus::Match,
            explanation: None,
        });
        let m = r.per_op_map();
        assert!(m.contains_key("a"));
        assert_eq!(m["a"].status, ReplayOpStatus::Match);
    }
}
