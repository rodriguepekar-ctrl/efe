// SPDX-License-Identifier: UNLICENSED
//! **Sprint 13 — N21** : Blackboard snapshot.
//!
//! Frozen read-only view of the blackboard taken at a specific
//! point in time.  Used by the aggregator at end-of-swarm to
//! assemble the `ContextBundle`.

use std::collections::BTreeMap;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::blackboard::BlackboardValue;

/// **N21** : a frozen snapshot of the blackboard at a moment in time.
///
/// Stored as `BTreeMap<String, BlackboardValue>` (sorted by JSON
/// pointer for deterministic iteration — important for replay
/// determinism I13).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlackboardSnapshot {
    pub swarm_id: efe_nano_sdk::SwarmId,
    pub captured_at: SystemTime,
    /// Sorted by JSON pointer for replay determinism.
    pub fields: BTreeMap<String, BlackboardValue>,
}

impl BlackboardSnapshot {
    /// **N21** : Look up a field by pointer.
    pub fn get(&self, pointer: &str) -> Option<&BlackboardValue> {
        self.fields.get(pointer)
    }

    /// **N21** : Number of fields in the snapshot.
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// **N21** : Estimated bytes if serialised to JSON.
    pub fn estimated_size_bytes(&self) -> usize {
        serde_json::to_vec(self).map(|v| v.len()).unwrap_or(0)
    }

    /// **N21** : Iterate fields in deterministic (sorted) order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &BlackboardValue)> {
        self.fields.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::{AgentId, SwarmId};

    fn snap_with(values: Vec<(&str, serde_json::Value)>) -> BlackboardSnapshot {
        let mut fields = BTreeMap::new();
        for (pointer, v) in values {
            fields.insert(
                pointer.to_string(),
                BlackboardValue {
                    value: v,
                    version: 1,
                    written_by: AgentId::new("ner_extractor").unwrap(),
                    written_at: SystemTime::now(),
                },
            );
        }
        BlackboardSnapshot {
            swarm_id: SwarmId::new(),
            captured_at: SystemTime::now(),
            fields,
        }
    }

    #[test]
    fn snapshot_get_returns_value() {
        let s = snap_with(vec![("/a", serde_json::json!(1))]);
        let v = s.get("/a").unwrap();
        assert_eq!(v.value, serde_json::json!(1));
    }

    #[test]
    fn snapshot_get_missing_returns_none() {
        let s = snap_with(vec![]);
        assert!(s.get("/missing").is_none());
    }

    #[test]
    fn snapshot_len_and_empty() {
        let s = snap_with(vec![]);
        assert_eq!(s.len(), 0);
        assert!(s.is_empty());
        let s2 = snap_with(vec![("/x", serde_json::json!(1))]);
        assert_eq!(s2.len(), 1);
        assert!(!s2.is_empty());
    }

    #[test]
    fn snapshot_iter_sorted_by_pointer() {
        let s = snap_with(vec![
            ("/z", serde_json::json!(0)),
            ("/a", serde_json::json!(0)),
            ("/m", serde_json::json!(0)),
        ]);
        let keys: Vec<&str> = s.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, vec!["/a", "/m", "/z"]);
    }

    #[test]
    fn snapshot_serde_round_trip() {
        let s = snap_with(vec![("/a", serde_json::json!(42))]);
        let json = serde_json::to_string(&s).unwrap();
        let back: BlackboardSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn snapshot_estimated_size_grows_with_fields() {
        let s1 = snap_with(vec![]);
        let s2 = snap_with(vec![("/a", serde_json::json!("x".repeat(100)))]);
        assert!(s2.estimated_size_bytes() > s1.estimated_size_bytes());
    }
}
