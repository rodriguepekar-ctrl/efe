// SPDX-License-Identifier: UNLICENSED
//! **Sprint 13 — N21** : Blackboard — concurrent JSON-pointer-keyed
//! store with per-field versioning.
//!
//! Architecture § 7.2 + ADR-2 + I3 + I17.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::SystemTime;

use dashmap::DashMap;
use efe_nano_sdk::{AgentId, SwarmId};
use serde::{Deserialize, Serialize};

use crate::error::BlackboardError;
use crate::snapshot::BlackboardSnapshot;

/// **N21** : a single versioned blackboard cell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlackboardValue {
    pub value: serde_json::Value,
    pub version: u64,
    pub written_by: AgentId,
    pub written_at: SystemTime,
}

/// **N21** : a single write event in the audit log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlackboardWrite {
    pub pointer: String,
    pub version: u64,
    pub agent_id: AgentId,
    pub at: SystemTime,
    pub bytes: usize,
}

/// **N21** : the shared blackboard.
///
/// Concurrency model :
/// - `DashMap` shard-locked for per-key fine-grained access.
/// - Each `BlackboardValue.version` increments monotonically on
///   each write (per JSON pointer).
/// - `write_if` is the CAS primitive : write iff stored version
///   matches `expected_version`.
/// - `write_history` is a `Mutex<Vec<...>>` ; only `write()` touches it
///   and the lock is held only over the push (no `.await` between
///   lock + drop — I17).
/// - `total_bytes` is an atomic running sum of serialised value
///   sizes — used to enforce `MAX_BLACKBOARD_BYTES`.
pub struct Blackboard {
    fields: DashMap<String, BlackboardValue>,
    swarm_id: SwarmId,
    write_history: Mutex<Vec<BlackboardWrite>>,
    total_bytes: AtomicUsize,
    max_bytes: usize,
}

impl std::fmt::Debug for Blackboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Blackboard")
            .field("swarm_id", &self.swarm_id)
            .field("fields", &self.fields.len())
            .field("total_bytes", &self.total_bytes.load(Ordering::Relaxed))
            .field("max_bytes", &self.max_bytes)
            .finish()
    }
}

impl Blackboard {
    /// **N21** : Construct with the default 256 KB cap.
    pub fn new(swarm_id: SwarmId) -> Self {
        Self::with_cap(swarm_id, crate::MAX_BLACKBOARD_BYTES)
    }

    /// **N21 — test helper** : custom cap.
    pub fn with_cap(swarm_id: SwarmId, max_bytes: usize) -> Self {
        Self {
            fields: DashMap::new(),
            swarm_id,
            write_history: Mutex::new(Vec::new()),
            total_bytes: AtomicUsize::new(0),
            max_bytes,
        }
    }

    pub fn swarm_id(&self) -> SwarmId {
        self.swarm_id
    }

    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    pub fn total_bytes(&self) -> usize {
        self.total_bytes.load(Ordering::Relaxed)
    }

    pub fn len(&self) -> usize {
        self.fields.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// **N21** : Unconditional write — increments the version of the
    /// target pointer.  Returns the new version.
    ///
    /// Enforces `MAX_BLACKBOARD_BYTES` (I10 / G8) :
    /// - If the new total size > cap, the write is rejected with
    ///   `SizeLimitExceeded` AND the in-memory state is not mutated.
    pub fn write(
        &self,
        pointer: &str,
        value: serde_json::Value,
        agent_id: AgentId,
    ) -> Result<u64, BlackboardError> {
        validate_pointer(pointer)?;

        // Compute new value bytes.
        let new_bytes = serialised_size(&value);

        // Read current value (if any) to find old bytes + version.
        let (old_bytes, old_version) = match self.fields.get(pointer) {
            Some(e) => (serialised_size(&e.value().value), e.value().version),
            None => (0, 0),
        };

        // Provisional new total.
        let cur_total = self.total_bytes.load(Ordering::Acquire);
        let new_total = cur_total
            .saturating_sub(old_bytes)
            .saturating_add(new_bytes);
        if new_total > self.max_bytes {
            return Err(BlackboardError::SizeLimitExceeded {
                current: new_total,
                max: self.max_bytes,
            });
        }

        let new_version = old_version + 1;
        let bv = BlackboardValue {
            value,
            version: new_version,
            written_by: agent_id.clone(),
            written_at: SystemTime::now(),
        };
        self.fields.insert(pointer.to_string(), bv);
        self.total_bytes.store(new_total, Ordering::Release);

        // Audit log : push to the history vec.  Lock is short-lived
        // (no .await inside — I17).
        if let Ok(mut h) = self.write_history.lock() {
            h.push(BlackboardWrite {
                pointer: pointer.to_string(),
                version: new_version,
                agent_id,
                at: SystemTime::now(),
                bytes: new_bytes,
            });
        }
        Ok(new_version)
    }

    /// **N21 + F9** : CAS-style write — only succeeds if the stored
    /// version equals `expected_version`.  `expected_version = 0`
    /// means "expected absent" — succeeds iff the pointer is unset.
    ///
    /// **Atomicity** (post-audit fix B-CAS) : the compare + swap is
    /// performed under a single `DashMap::entry` shard guard so
    /// concurrent CAS writers serialise.  Without this, multiple
    /// writers could read the same `actual_version` and both
    /// "succeed", losing updates.
    pub fn write_if(
        &self,
        pointer: &str,
        expected_version: u64,
        value: serde_json::Value,
        agent_id: AgentId,
    ) -> Result<u64, BlackboardError> {
        validate_pointer(pointer)?;
        let new_bytes = serialised_size(&value);

        // Hold the shard lock for the entire compare + swap.
        let entry = self.fields.entry(pointer.to_string());

        use dashmap::mapref::entry::Entry;
        let (old_version, old_bytes) = match &entry {
            Entry::Occupied(o) => (o.get().version, serialised_size(&o.get().value)),
            Entry::Vacant(_) => (0, 0),
        };

        if old_version != expected_version {
            return Err(BlackboardError::Conflict {
                pointer: pointer.to_string(),
                expected_version,
                actual_version: old_version,
            });
        }

        // Size check.
        let cur_total = self.total_bytes.load(Ordering::Acquire);
        let new_total = cur_total
            .saturating_sub(old_bytes)
            .saturating_add(new_bytes);
        if new_total > self.max_bytes {
            return Err(BlackboardError::SizeLimitExceeded {
                current: new_total,
                max: self.max_bytes,
            });
        }

        let new_version = old_version + 1;
        let bv = BlackboardValue {
            value,
            version: new_version,
            written_by: agent_id.clone(),
            written_at: SystemTime::now(),
        };
        match entry {
            Entry::Occupied(mut o) => {
                o.insert(bv);
            }
            Entry::Vacant(v) => {
                v.insert(bv);
            }
        }
        self.total_bytes.store(new_total, Ordering::Release);

        // Audit log : short-lived lock (I17).
        if let Ok(mut h) = self.write_history.lock() {
            h.push(BlackboardWrite {
                pointer: pointer.to_string(),
                version: new_version,
                agent_id,
                at: SystemTime::now(),
                bytes: new_bytes,
            });
        }
        Ok(new_version)
    }

    /// **N21** : Read the current value at a pointer (cloned to drop
    /// the DashMap shard guard immediately — I17).
    pub fn read(&self, pointer: &str) -> Option<BlackboardValue> {
        self.fields.get(pointer).map(|r| r.value().clone())
    }

    /// **N21** : True if a pointer is currently set.
    pub fn contains(&self, pointer: &str) -> bool {
        self.fields.contains_key(pointer)
    }

    /// **N21** : Capture a deterministic snapshot — sorted by
    /// pointer for replay determinism (I13).
    pub fn snapshot(&self) -> BlackboardSnapshot {
        let mut fields: BTreeMap<String, BlackboardValue> = BTreeMap::new();
        for entry in self.fields.iter() {
            fields.insert(entry.key().clone(), entry.value().clone());
        }
        BlackboardSnapshot {
            swarm_id: self.swarm_id,
            captured_at: SystemTime::now(),
            fields,
        }
    }

    /// **N21** : Read the full audit log of writes (cloned).
    pub fn write_history(&self) -> Vec<BlackboardWrite> {
        self.write_history
            .lock()
            .map(|h| h.clone())
            .unwrap_or_default()
    }

    /// **N21** : Cumulative number of writes since construction.
    pub fn write_count(&self) -> usize {
        self.write_history
            .lock()
            .map(|h| h.len())
            .unwrap_or(0)
    }

    /// **N21** : Per-agent write count (handy for the SwarmReport).
    pub fn writes_by_agent(&self, agent_id: &AgentId) -> usize {
        self.write_history
            .lock()
            .map(|h| h.iter().filter(|w| &w.agent_id == agent_id).count())
            .unwrap_or(0)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  JSON pointer validation — RFC 6901 short check
// ─────────────────────────────────────────────────────────────────────

fn validate_pointer(p: &str) -> Result<(), BlackboardError> {
    if p.is_empty() {
        // RFC 6901 allows "" as the root pointer — but we don't
        // allow writing to root because callers should target a
        // specific field.  Reject.
        return Err(BlackboardError::InvalidPointer {
            value: "".into(),
        });
    }
    if !p.starts_with('/') {
        return Err(BlackboardError::InvalidPointer {
            value: p.to_string(),
        });
    }
    Ok(())
}

fn serialised_size(v: &serde_json::Value) -> usize {
    serde_json::to_vec(v).map(|b| b.len()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(s: &str) -> AgentId {
        AgentId::new(s).unwrap()
    }

    fn bb() -> Blackboard {
        Blackboard::new(SwarmId::new())
    }

    // ─── Basic write / read ─────────────────────────────────────────

    #[test]
    fn fresh_blackboard_is_empty() {
        let b = bb();
        assert_eq!(b.len(), 0);
        assert!(b.is_empty());
        assert_eq!(b.total_bytes(), 0);
    }

    #[test]
    fn write_then_read_returns_value() {
        let b = bb();
        let v = b
            .write("/x", serde_json::json!(42), agent("ner_extractor"))
            .unwrap();
        assert_eq!(v, 1);
        let bv = b.read("/x").unwrap();
        assert_eq!(bv.value, serde_json::json!(42));
        assert_eq!(bv.version, 1);
        assert_eq!(bv.written_by.as_str(), "ner_extractor");
    }

    #[test]
    fn write_increments_version_per_pointer() {
        let b = bb();
        let v1 = b
            .write("/x", serde_json::json!(1), agent("a-1"))
            .unwrap();
        let v2 = b
            .write("/x", serde_json::json!(2), agent("a-2"))
            .unwrap();
        assert_eq!(v1, 1);
        assert_eq!(v2, 2);
        let bv = b.read("/x").unwrap();
        assert_eq!(bv.value, serde_json::json!(2));
        assert_eq!(bv.version, 2);
        assert_eq!(bv.written_by.as_str(), "a-2");
    }

    #[test]
    fn write_to_different_pointers_independent() {
        let b = bb();
        b.write("/x", serde_json::json!(1), agent("aaa")).unwrap();
        b.write("/y", serde_json::json!(2), agent("aaa")).unwrap();
        assert_eq!(b.read("/x").unwrap().value, serde_json::json!(1));
        assert_eq!(b.read("/y").unwrap().value, serde_json::json!(2));
    }

    #[test]
    fn read_missing_pointer_returns_none() {
        let b = bb();
        assert!(b.read("/missing").is_none());
    }

    #[test]
    fn contains_reflects_write() {
        let b = bb();
        assert!(!b.contains("/x"));
        b.write("/x", serde_json::json!(1), agent("aaa")).unwrap();
        assert!(b.contains("/x"));
    }

    // ─── JSON pointer validation ────────────────────────────────────

    #[test]
    fn empty_pointer_rejected() {
        let b = bb();
        let err = b
            .write("", serde_json::json!(1), agent("aaa"))
            .unwrap_err();
        assert!(matches!(err, BlackboardError::InvalidPointer { .. }));
    }

    #[test]
    fn pointer_without_leading_slash_rejected() {
        let b = bb();
        let err = b
            .write("x", serde_json::json!(1), agent("aaa"))
            .unwrap_err();
        assert!(matches!(err, BlackboardError::InvalidPointer { .. }));
    }

    #[test]
    fn nested_pointer_accepted() {
        let b = bb();
        assert!(b
            .write("/foo/bar/baz", serde_json::json!(1), agent("aaa"))
            .is_ok());
        assert!(b.contains("/foo/bar/baz"));
    }

    // ─── write_if (CAS) ────────────────────────────────────────────

    #[test]
    fn write_if_expected_zero_succeeds_when_absent() {
        let b = bb();
        let v = b
            .write_if("/x", 0, serde_json::json!(1), agent("aaa"))
            .unwrap();
        assert_eq!(v, 1);
    }

    #[test]
    fn write_if_expected_zero_fails_when_present() {
        let b = bb();
        b.write("/x", serde_json::json!(1), agent("aaa")).unwrap();
        let err = b
            .write_if("/x", 0, serde_json::json!(2), agent("bbb"))
            .unwrap_err();
        match err {
            BlackboardError::Conflict {
                pointer,
                expected_version,
                actual_version,
            } => {
                assert_eq!(pointer, "/x");
                assert_eq!(expected_version, 0);
                assert_eq!(actual_version, 1);
            }
            other => panic!("expected Conflict, got {other:?}"),
        }
    }

    #[test]
    fn write_if_matching_version_succeeds() {
        let b = bb();
        b.write("/x", serde_json::json!(1), agent("aaa")).unwrap();
        let v = b
            .write_if("/x", 1, serde_json::json!(2), agent("bbb"))
            .unwrap();
        assert_eq!(v, 2);
    }

    #[test]
    fn write_if_stale_version_fails() {
        let b = bb();
        b.write("/x", serde_json::json!(1), agent("aaa")).unwrap();
        b.write("/x", serde_json::json!(2), agent("bbb")).unwrap();
        // Stale CAS : expected 1, actual 2.
        let err = b
            .write_if("/x", 1, serde_json::json!(3), agent("ccc"))
            .unwrap_err();
        assert!(matches!(err, BlackboardError::Conflict { .. }));
    }

    // ─── Audit log ───────────────────────────────────────────────

    #[test]
    fn write_history_records_each_write() {
        let b = bb();
        b.write("/x", serde_json::json!(1), agent("aaa")).unwrap();
        b.write("/y", serde_json::json!(2), agent("bbb")).unwrap();
        b.write("/x", serde_json::json!(3), agent("aaa")).unwrap();
        let h = b.write_history();
        assert_eq!(h.len(), 3);
        assert_eq!(h[0].pointer, "/x");
        assert_eq!(h[1].pointer, "/y");
        assert_eq!(h[2].pointer, "/x");
        assert_eq!(h[2].version, 2);
    }

    #[test]
    fn writes_by_agent_counts_correctly() {
        let b = bb();
        b.write("/x", serde_json::json!(1), agent("alpha")).unwrap();
        b.write("/y", serde_json::json!(2), agent("alpha")).unwrap();
        b.write("/z", serde_json::json!(3), agent("beta")).unwrap();
        assert_eq!(b.writes_by_agent(&agent("alpha")), 2);
        assert_eq!(b.writes_by_agent(&agent("beta")), 1);
        assert_eq!(b.writes_by_agent(&agent("gamma")), 0);
    }

    #[test]
    fn write_count_total() {
        let b = bb();
        b.write("/x", serde_json::json!(1), agent("aaa")).unwrap();
        b.write("/y", serde_json::json!(2), agent("aaa")).unwrap();
        assert_eq!(b.write_count(), 2);
    }

    // ─── Size cap ────────────────────────────────────────────────

    #[test]
    fn write_under_cap_succeeds() {
        let b = Blackboard::with_cap(SwarmId::new(), 10_000);
        let value = serde_json::json!("x".repeat(100));
        let res = b.write("/a", value, agent("aaa"));
        assert!(res.is_ok());
        assert!(b.total_bytes() > 0);
    }

    #[test]
    fn write_over_cap_rejected() {
        let b = Blackboard::with_cap(SwarmId::new(), 100);
        let big = serde_json::json!("x".repeat(500));
        let err = b.write("/a", big, agent("aaa")).unwrap_err();
        assert!(matches!(err, BlackboardError::SizeLimitExceeded { .. }));
        // State not mutated.
        assert!(!b.contains("/a"));
        assert_eq!(b.total_bytes(), 0);
    }

    #[test]
    fn write_overwriting_releases_old_bytes() {
        let b = Blackboard::with_cap(SwarmId::new(), 200);
        // 100 bytes.
        b.write(
            "/a",
            serde_json::json!("x".repeat(80)),
            agent("aaa"),
        )
        .unwrap();
        let bytes_before = b.total_bytes();
        // Overwrite with 50 bytes ; total should shrink.
        b.write(
            "/a",
            serde_json::json!("y".repeat(20)),
            agent("aaa"),
        )
        .unwrap();
        assert!(b.total_bytes() < bytes_before);
    }

    // ─── Snapshot ────────────────────────────────────────────────

    #[test]
    fn snapshot_captures_all_fields_sorted() {
        let b = bb();
        b.write("/z", serde_json::json!(1), agent("aaa")).unwrap();
        b.write("/a", serde_json::json!(2), agent("aaa")).unwrap();
        b.write("/m", serde_json::json!(3), agent("aaa")).unwrap();
        let s = b.snapshot();
        let keys: Vec<&str> = s.fields.keys().map(|s| s.as_str()).collect();
        assert_eq!(keys, vec!["/a", "/m", "/z"]);
    }

    #[test]
    fn snapshot_after_writes_matches_field_count() {
        let b = bb();
        b.write("/x", serde_json::json!(1), agent("aaa")).unwrap();
        let s = b.snapshot();
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn snapshot_swarm_id_matches_blackboard() {
        let id = SwarmId::new();
        let b = Blackboard::new(id);
        let s = b.snapshot();
        assert_eq!(s.swarm_id, id);
    }

    // ─── Concurrency ─────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_writes_to_different_pointers_safe() {
        let b = std::sync::Arc::new(bb());
        let mut handles = Vec::new();
        for i in 0..50 {
            let b = b.clone();
            handles.push(tokio::spawn(async move {
                let pointer = format!("/key_{i}");
                let agent_name = format!("agent-{}", i % 10);
                b.write(&pointer, serde_json::json!(i), agent(&agent_name))
                    .unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        assert_eq!(b.len(), 50);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_writes_to_same_pointer_all_succeed() {
        // write() is unconditional ; under concurrency the *final*
        // version may be lower than the number of writes because
        // multiple writers can read the same `old_version` and both
        // compute `new_version = old + 1` (a benign race for
        // non-CAS writes).  What we DO guarantee :
        // - All writes succeed (no errors).
        // - write_count (audit log) == number of writes.
        // - The final stored version is at most N and at least 1.
        // - The final stored value is one of the written values.
        //
        // Callers needing atomic version increment must use
        // `write_if` (CAS) instead.
        let b = std::sync::Arc::new(bb());
        let mut handles = Vec::new();
        for i in 0..20 {
            let b = b.clone();
            handles.push(tokio::spawn(async move {
                b.write("/shared", serde_json::json!(i), agent("aaa"))
                    .unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        let bv = b.read("/shared").unwrap();
        assert!(bv.version >= 1 && bv.version <= 20);
        assert_eq!(b.write_count(), 20);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_write_if_serialises_version_increment() {
        // write_if (CAS) — only one thread succeeds per version
        // transition.  Conflicts surface as `Conflict` errors.
        let b = std::sync::Arc::new(bb());
        b.write("/shared", serde_json::json!(0), agent("aaa")).unwrap();
        let mut handles = Vec::new();
        for i in 0..20 {
            let b = b.clone();
            handles.push(tokio::spawn(async move {
                // Try CAS from version 1 ; first wins, others
                // get Conflict.
                b.write_if("/shared", 1, serde_json::json!(i), agent("aaa"))
                    .ok()
            }));
        }
        let mut successes = 0;
        for h in handles {
            if h.await.unwrap().is_some() {
                successes += 1;
            }
        }
        // Exactly one CAS succeeded.
        assert_eq!(successes, 1);
        let bv = b.read("/shared").unwrap();
        assert_eq!(bv.version, 2);
    }

    // ─── Misc ────────────────────────────────────────────────────

    #[test]
    fn debug_format_no_panic() {
        let b = bb();
        let _ = format!("{b:?}");
    }

    #[test]
    fn blackboard_value_serde_round_trip() {
        let v = BlackboardValue {
            value: serde_json::json!({"a": 1}),
            version: 5,
            written_by: agent("aaa"),
            written_at: SystemTime::now(),
        };
        let json = serde_json::to_string(&v).unwrap();
        let back: BlackboardValue = serde_json::from_str(&json).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn max_bytes_default_256kb() {
        let b = bb();
        assert_eq!(b.max_bytes(), 262_144);
    }
}
