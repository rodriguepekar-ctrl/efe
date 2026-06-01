// SPDX-License-Identifier: UNLICENSED
//! Implémentation `BudgetTracker`.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use dashmap::DashMap;
use serde::{Deserialize, Serialize};

use crate::error::{MultiLlmError, MultiLlmResult};

/// Tracker atomique du coût cumulé d'une mission multi-LLM.
///
/// ## Invariant I13
///
/// `try_charge` est atomique : si l'opération ferait dépasser
/// `max_cost_cents`, le compteur est revert et `MultiLlmError::BudgetExceeded`
/// est retournée.  Garantit qu'on ne **dépasse jamais** le hard-cap, même
/// en concurrence forte.
///
/// ## Granularité
///
/// - Total : compteur monotone unique.
/// - Per-LLM : `DashMap<LlmRef, AtomicU32>` pour breakdown observabilité.
#[derive(Clone)]
pub struct BudgetTracker {
    max_cost_cents: u32,
    spent_cents: Arc<AtomicU32>,
    per_llm: Arc<DashMap<String, AtomicU32>>,
}

impl BudgetTracker {
    /// Construit un tracker avec hard-cap.
    #[must_use]
    pub fn new(max_cost_cents: u32) -> Self {
        Self {
            max_cost_cents,
            spent_cents: Arc::new(AtomicU32::new(0)),
            per_llm: Arc::new(DashMap::new()),
        }
    }

    /// Charge `cents` au compte du LLM `llm`.  Retourne le total cumulé
    /// si OK, ou une erreur si dépassement (avec revert atomique).
    pub fn try_charge(&self, llm: &str, cents: u32) -> MultiLlmResult<u32> {
        // CAS atomique : on tente d'incrémenter ; si le résultat dépasse, on revert.
        let new_total = self.spent_cents.fetch_add(cents, Ordering::SeqCst) + cents;
        if new_total > self.max_cost_cents {
            // Revert atomique (jamais OOM car cents ≤ déjà ajouté).
            self.spent_cents.fetch_sub(cents, Ordering::SeqCst);
            return Err(MultiLlmError::BudgetExceeded {
                spent_cents: self.spent_cents.load(Ordering::SeqCst),
                max_cents: self.max_cost_cents,
            });
        }
        // Update per-LLM.
        self.per_llm
            .entry(llm.to_string())
            .or_insert_with(|| AtomicU32::new(0))
            .fetch_add(cents, Ordering::SeqCst);
        Ok(new_total)
    }

    /// Total déjà engagé (cents).
    #[must_use]
    pub fn spent_cents(&self) -> u32 {
        self.spent_cents.load(Ordering::SeqCst)
    }

    /// Plafond.
    #[must_use]
    pub fn max_cost_cents(&self) -> u32 {
        self.max_cost_cents
    }

    /// Budget restant.
    #[must_use]
    pub fn remaining(&self) -> u32 {
        self.max_cost_cents.saturating_sub(self.spent_cents())
    }

    /// Breakdown per-LLM.
    #[must_use]
    pub fn per_llm_breakdown(&self) -> HashMap<String, u32> {
        self.per_llm
            .iter()
            .map(|e| (e.key().clone(), e.value().load(Ordering::SeqCst)))
            .collect()
    }

    /// Snapshot sérialisable.
    #[must_use]
    pub fn snapshot(&self) -> BudgetSnapshot {
        BudgetSnapshot {
            spent_cents: self.spent_cents(),
            max_cost_cents: self.max_cost_cents,
            remaining: self.remaining(),
            per_llm: self.per_llm_breakdown(),
        }
    }
}

/// Snapshot du tracker pour audit + observabilité.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BudgetSnapshot {
    /// Engagé.
    pub spent_cents: u32,
    /// Hard-cap.
    pub max_cost_cents: u32,
    /// Restant.
    pub remaining: u32,
    /// Per-LLM.
    pub per_llm: HashMap<String, u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_tracker_starts_at_zero() {
        let t = BudgetTracker::new(1000);
        assert_eq!(t.spent_cents(), 0);
        assert_eq!(t.remaining(), 1000);
    }

    #[test]
    fn charge_under_cap_succeeds() {
        let t = BudgetTracker::new(1000);
        assert_eq!(t.try_charge("claude", 200).unwrap(), 200);
        assert_eq!(t.spent_cents(), 200);
        assert_eq!(t.remaining(), 800);
    }

    #[test]
    fn charge_over_cap_errors_and_reverts() {
        let t = BudgetTracker::new(100);
        let _ = t.try_charge("claude", 60).unwrap();
        let r = t.try_charge("gpt", 50);
        assert!(matches!(r, Err(MultiLlmError::BudgetExceeded { .. })));
        // Le compteur doit avoir reverté → 60 (pas 110).
        assert_eq!(t.spent_cents(), 60);
    }

    #[test]
    fn per_llm_breakdown_tracks_each() {
        let t = BudgetTracker::new(1000);
        t.try_charge("claude", 100).unwrap();
        t.try_charge("gpt", 50).unwrap();
        t.try_charge("claude", 30).unwrap();
        let b = t.per_llm_breakdown();
        assert_eq!(b.get("claude").copied(), Some(130));
        assert_eq!(b.get("gpt").copied(), Some(50));
    }

    #[test]
    fn concurrent_charges_are_atomic() {
        use std::thread;
        let t = BudgetTracker::new(10_000);
        let mut handles = vec![];
        for _ in 0..100 {
            let t = t.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..10 {
                    let _ = t.try_charge("x", 1);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(t.spent_cents(), 1000);
    }

    #[test]
    fn concurrent_charges_at_cap_never_overflow() {
        use std::thread;
        let t = BudgetTracker::new(50);
        let mut handles = vec![];
        for _ in 0..50 {
            let t = t.clone();
            handles.push(thread::spawn(move || {
                let _ = t.try_charge("x", 1);
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        // Borne dure respectée même avec contention.
        assert!(t.spent_cents() <= 50);
    }

    #[test]
    fn snapshot_serializable() {
        let t = BudgetTracker::new(500);
        t.try_charge("a", 100).unwrap();
        let snap = t.snapshot();
        let s = serde_json::to_string(&snap).unwrap();
        let back: BudgetSnapshot = serde_json::from_str(&s).unwrap();
        assert_eq!(back, snap);
    }

    #[test]
    fn exact_cap_is_allowed() {
        let t = BudgetTracker::new(100);
        assert!(t.try_charge("x", 100).is_ok());
        assert_eq!(t.remaining(), 0);
        // 1 cent de plus = refus.
        assert!(t.try_charge("y", 1).is_err());
    }
}
