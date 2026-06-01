//! Per-op determinism checker.
//!
//! Compares an op's original `output_hash` against the replay-produced
//! hash, classifying the result based on the agent's `DeterminismLevel`.
//!
//! **Sprint 7 — N5 rewrite** : exhaustive 5-variant match on
//! [`DeterminismLevel`].  Pre-Sprint-7 the checker dispatched on the
//! `execution_mode: &str` field to distinguish Mode B (seeded, replayable)
//! from Mode C (LLM, non-replayable) — both classed as `Probabilistic`.
//! That string-typed dispatch was brittle ; Sprint 7's enum split
//! makes the contract explicit and impossible to drift.

use efe_nano_sdk::DeterminismLevel;

use crate::audit::hash_json;
use crate::replay::types::ReplayOpStatus;

// ─────────────────────────────────────────────────────────────────────
//  DeterminismChecker
// ─────────────────────────────────────────────────────────────────────

pub struct DeterminismChecker;

impl DeterminismChecker {
    /// Compare original vs replay hash, with determinism-aware semantics.
    ///
    /// **Sprint 7 — N5 contract** :
    ///
    /// | Variant         | (orig, replay)         | Status                       |
    /// |-----------------|------------------------|------------------------------|
    /// | FullDeterministic | (Some, Some) eq      | Match                        |
    /// | FullDeterministic | (Some, Some) neq     | Mismatch                     |
    /// | FullDeterministic | (_, None) / (None,_) | ReplayFailed                 |
    /// | Seeded            | (Some, Some) eq      | Match                        |
    /// | Seeded            | (Some, Some) neq     | Mismatch (seed drift!)       |
    /// | Seeded            | (_, None)            | ReplayFailed                 |
    /// | LlmReplayable     | (Some, Some) eq      | Match (cache hit)            |
    /// | LlmReplayable     | (Some, Some) neq     | Mismatch (provider drift)    |
    /// | LlmReplayable     | (Some, None)         | ReplayWindowExpired (N3)     |
    /// | LlmEphemeral      | *                    | SkippedNonDeterministic      |
    /// | SideEffect        | *                    | SkippedNonDeterministic      |
    ///
    /// The `_execution_mode: &str` parameter is preserved for backward
    /// compatibility with the Sprint 6 signature but is no longer
    /// consulted — the contract is fully encoded in `determinism`.
    pub fn classify(
        determinism: DeterminismLevel,
        _execution_mode: &str,
        original: Option<[u8; 32]>,
        replay: Option<[u8; 32]>,
    ) -> ReplayOpStatus {
        match determinism {
            DeterminismLevel::FullDeterministic | DeterminismLevel::Seeded => {
                match (original, replay) {
                    (Some(o), Some(r)) if o == r => ReplayOpStatus::Match,
                    (Some(_), Some(_)) => ReplayOpStatus::Mismatch,
                    _ => ReplayOpStatus::ReplayFailed,
                }
            }
            DeterminismLevel::LlmReplayable => {
                match (original, replay) {
                    // Cache hit + same hash ⇒ Match.
                    (Some(o), Some(r)) if o == r => ReplayOpStatus::Match,
                    // Cache hit + different hash ⇒ provider drift (alarm).
                    (Some(_), Some(_)) => ReplayOpStatus::Mismatch,
                    // Original existed but replay produced nothing ⇒
                    // window expired (cache evicted).
                    (Some(_), None) => ReplayOpStatus::ReplayWindowExpired,
                    // Original itself was None (op failed originally) :
                    // we never had a hash to compare against — fall
                    // back to the ReplayFailed signal.
                    (None, _) => ReplayOpStatus::ReplayFailed,
                }
            }
            DeterminismLevel::LlmEphemeral | DeterminismLevel::SideEffect => {
                ReplayOpStatus::SkippedNonDeterministic
            }
        }
    }

    /// Hash a JSON value for replay comparison.
    pub fn hash(value: &serde_json::Value) -> [u8; 32] {
        hash_json(value)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn full_deterministic_match() {
        let h = [42u8; 32];
        assert_eq!(
            DeterminismChecker::classify(
                DeterminismLevel::FullDeterministic,
                "pure_rust",
                Some(h),
                Some(h)
            ),
            ReplayOpStatus::Match
        );
    }

    #[test]
    fn full_deterministic_mismatch() {
        let a = [42u8; 32];
        let b = [43u8; 32];
        assert_eq!(
            DeterminismChecker::classify(
                DeterminismLevel::FullDeterministic,
                "pure_rust",
                Some(a),
                Some(b)
            ),
            ReplayOpStatus::Mismatch
        );
    }

    #[test]
    fn full_deterministic_missing_replay_is_failed() {
        let h = [42u8; 32];
        assert_eq!(
            DeterminismChecker::classify(
                DeterminismLevel::FullDeterministic,
                "pure_rust",
                Some(h),
                None,
            ),
            ReplayOpStatus::ReplayFailed
        );
    }

    // ─────────────────────────────────────────────────────────────────
    //  Sprint 7 — N5 : Seeded (was Mode B under Probabilistic)
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn seeded_strict_match() {
        let h = [10u8; 32];
        assert_eq!(
            DeterminismChecker::classify(
                DeterminismLevel::Seeded,
                "local_model",
                Some(h),
                Some(h),
            ),
            ReplayOpStatus::Match
        );
    }

    #[test]
    fn seeded_mismatch_signals_seed_drift() {
        assert_eq!(
            DeterminismChecker::classify(
                DeterminismLevel::Seeded,
                "local_model",
                Some([1u8; 32]),
                Some([2u8; 32]),
            ),
            ReplayOpStatus::Mismatch
        );
    }

    // ─────────────────────────────────────────────────────────────────
    //  Sprint 7 — N3 / N5 : LlmReplayable
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn llm_replayable_cache_hit_matches() {
        let h = [7u8; 32];
        assert_eq!(
            DeterminismChecker::classify(
                DeterminismLevel::LlmReplayable,
                "llm_augmented",
                Some(h),
                Some(h),
            ),
            ReplayOpStatus::Match
        );
    }

    #[test]
    fn llm_replayable_cache_hit_mismatch_signals_provider_drift() {
        assert_eq!(
            DeterminismChecker::classify(
                DeterminismLevel::LlmReplayable,
                "llm_augmented",
                Some([1u8; 32]),
                Some([2u8; 32]),
            ),
            ReplayOpStatus::Mismatch
        );
    }

    #[test]
    fn llm_replayable_cache_miss_signals_window_expired() {
        assert_eq!(
            DeterminismChecker::classify(
                DeterminismLevel::LlmReplayable,
                "llm_augmented",
                Some([1u8; 32]),
                None,
            ),
            ReplayOpStatus::ReplayWindowExpired
        );
    }

    // ─────────────────────────────────────────────────────────────────
    //  Sprint 7 — N5 : LlmEphemeral
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn llm_ephemeral_always_skipped() {
        assert_eq!(
            DeterminismChecker::classify(
                DeterminismLevel::LlmEphemeral,
                "llm_augmented",
                Some([1u8; 32]),
                Some([2u8; 32]),
            ),
            ReplayOpStatus::SkippedNonDeterministic
        );
    }

    #[test]
    fn side_effect_always_skipped() {
        assert_eq!(
            DeterminismChecker::classify(
                DeterminismLevel::SideEffect,
                "pure_rust",
                Some([1u8; 32]),
                Some([1u8; 32]),
            ),
            ReplayOpStatus::SkippedNonDeterministic
        );
    }

    #[test]
    fn hash_helper_works() {
        let v = json!({"x": 1});
        let h = DeterminismChecker::hash(&v);
        assert_eq!(h.len(), 32);
    }
}
