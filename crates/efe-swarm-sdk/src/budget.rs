// SPDX-License-Identifier: UNLICENSED
//! Gouvernance de budget — un `BudgetGuard` **atomique** qui honore l'**invariant I13**
//! (`charge` ne **dépasse JAMAIS** le plafond, même en concurrence : ajout atomique puis
//! **revert** si dépassement).
//!
//! > Réplique le contrat de `efe_multi_llm::BudgetTracker` (même I13) en **~40 lignes**,
//! > pour garder le SDK **léger** (l'import d'`efe-multi-llm` tirerait tout son arbre
//! > réseau/TLS pour un simple compteur atomique). Le contrat est identique.
//!
//! Trois politiques de dépassement :
//! - **`Error`** : pré-vol — si le coût estimé dépasse le plafond, on REFUSE avant
//!   d'exécuter ; et si le budget est épuisé en cours (T3 dynamique), le run échoue.
//! - **`Stop`** : on exécute en facturant chaque appel ; à l'épuisement, les agents
//!   renvoient un **sentinel neutre** (la topologie dégrade) → résultat **partiel**.
//! - **`Degrade`** : pré-vol — on **réduit K** (et on rogne les juges) pour tenir dans
//!   le budget, puis on exécute normalement.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

use serde::Serialize;

/// Capture immuable et sérialisable de l'état du budget (audit/observabilité).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BudgetSnapshot {
    pub spent_cents: u32,
    pub max_cost_cents: u32,
    pub remaining: u32,
    /// Ventilation par agent/llm (ordre déterministe).
    pub per_llm: BTreeMap<String, u32>,
}

/// Politique en cas de dépassement de budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnExceed {
    /// Refuse avant exécution si l'estimation dépasse ; échoue si épuisé en cours.
    Error,
    /// Exécute et s'arrête proprement (résultat partiel) à l'épuisement.
    Stop,
    /// Réduit la topologie (K, juges) pour tenir dans le budget, puis exécute.
    Degrade,
}

/// Plafond + politique passés à `swarm.run`.
#[derive(Debug, Clone, Copy)]
pub struct Budget {
    pub max_cents: u32,
    pub on_exceed: OnExceed,
    /// Prix par appel d'agent (cents). `None` ⇒ le prix du `ClientMode` est utilisé.
    pub price_per_call_cents: Option<u32>,
}

impl Budget {
    /// Plafond avec arrêt propre (la politique la plus sûre par défaut).
    pub fn cents(max_cents: u32) -> Self {
        Self { max_cents, on_exceed: OnExceed::Stop, price_per_call_cents: None }
    }
    pub fn with_policy(mut self, p: OnExceed) -> Self {
        self.on_exceed = p;
        self
    }
    pub fn with_price_per_call(mut self, cents: u32) -> Self {
        self.price_per_call_cents = Some(cents);
        self
    }
    /// Budget « illimité » pratique (plafond max).
    pub fn unlimited() -> Self {
        Self { max_cents: u32::MAX, on_exceed: OnExceed::Stop, price_per_call_cents: None }
    }
}

/// Garde de budget partagée par tous les agents d'un run : facture chaque appel et
/// signale l'épuisement. **Invariant I13** : le compteur ne dépasse JAMAIS le plafond.
#[derive(Debug)]
pub struct BudgetGuard {
    max_cents: u32,
    price_cents: u32,
    spent: AtomicU32,
    per_agent: Mutex<BTreeMap<String, u32>>,
    exhausted: AtomicBool,
}

impl BudgetGuard {
    pub fn new(max_cents: u32, price_cents: u32) -> Self {
        Self {
            max_cents,
            price_cents,
            spent: AtomicU32::new(0),
            per_agent: Mutex::new(BTreeMap::new()),
            exhausted: AtomicBool::new(false),
        }
    }

    /// Facture un appel d'agent. Renvoie `true` si débité (l'agent peut s'exécuter),
    /// `false` si le plafond serait dépassé (l'agent renvoie un sentinel). **I13** :
    /// ajout atomique puis revert si dépassement ⇒ le compteur ne dépasse **jamais**.
    pub fn charge(&self, who: &str) -> bool {
        let cents = self.price_cents;
        if cents == 0 {
            self.note(who, 0); // local/gratuit : on compte l'appel, sans plafonner.
            return true;
        }
        let prev = self.spent.fetch_add(cents, Ordering::SeqCst);
        if prev.saturating_add(cents) > self.max_cents {
            self.spent.fetch_sub(cents, Ordering::SeqCst); // revert (jamais de dépassement)
            self.exhausted.store(true, Ordering::Relaxed);
            false
        } else {
            self.note(who, cents);
            true
        }
    }

    fn note(&self, who: &str, cents: u32) {
        if let Ok(mut m) = self.per_agent.lock() {
            *m.entry(who.to_string()).or_insert(0) += cents;
        }
    }

    pub fn exhausted(&self) -> bool {
        self.exhausted.load(Ordering::Relaxed)
    }
    pub fn spent_cents(&self) -> u32 {
        self.spent.load(Ordering::SeqCst)
    }
    pub fn max_cents(&self) -> u32 {
        self.max_cents
    }
    /// Prix facturé par appel (cents) — pour attribuer le coût d'un appel à un observateur.
    pub fn price_cents(&self) -> u32 {
        self.price_cents
    }
    pub fn snapshot(&self) -> BudgetSnapshot {
        let spent = self.spent_cents();
        BudgetSnapshot {
            spent_cents: spent,
            max_cost_cents: self.max_cents,
            remaining: self.max_cents.saturating_sub(spent),
            per_llm: self.per_agent.lock().map(|m| m.clone()).unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_charges_until_cap_then_marks_exhausted() {
        let g = BudgetGuard::new(10, 4); // plafond 10¢, 4¢/appel → 2 appels OK, le 3e refusé
        assert!(g.charge("a"));
        assert!(g.charge("a"));
        assert!(!g.charge("a"), "3e appel (12¢ > 10¢) refusé");
        assert!(g.exhausted());
        assert_eq!(g.spent_cents(), 8, "jamais au-delà du plafond (I13)");
        let snap = g.snapshot();
        assert_eq!(snap.spent_cents, 8);
        assert_eq!(snap.remaining, 2);
        assert_eq!(snap.per_llm.get("a"), Some(&8));
    }

    #[test]
    fn zero_price_never_exhausts() {
        let g = BudgetGuard::new(0, 0);
        for _ in 0..100 {
            assert!(g.charge("local"));
        }
        assert!(!g.exhausted());
        assert_eq!(g.spent_cents(), 0);
    }

    #[test]
    fn i13_holds_under_concurrency() {
        use std::sync::Arc;
        use std::thread;
        let g = Arc::new(BudgetGuard::new(100, 1));
        let mut hs = Vec::new();
        for _ in 0..8 {
            let g = g.clone();
            hs.push(thread::spawn(move || {
                for _ in 0..1000 {
                    g.charge("t");
                }
            }));
        }
        for h in hs {
            h.join().unwrap();
        }
        assert!(g.spent_cents() <= 100, "I13 : JAMAIS au-delà du plafond, même en concurrence");
    }

    #[test]
    fn budget_builder() {
        let b = Budget::cents(50).with_policy(OnExceed::Degrade).with_price_per_call(3);
        assert_eq!(b.max_cents, 50);
        assert_eq!(b.on_exceed, OnExceed::Degrade);
        assert_eq!(b.price_per_call_cents, Some(3));
    }
}
