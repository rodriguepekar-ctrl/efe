// SPDX-License-Identifier: UNLICENSED
//! **Phase 1 · WS4** — distribution du travail / contention CAS (corrige F6).
//!
//! Dans le modèle naïf (`swarm_relay`/`swarm_slm`), TOUS les R consommateurs
//! reçoivent chaque item et tentent `write_if("/claim/<i>", 0, …)` → par item :
//! 1 gagnant + (R−1) conflits. La baseline `swarm_load` a mesuré 90 %→99,9 % de
//! CAS gaspillés quand N croît : **O(N×items)**.
//!
//! [`WorkQueue`] propose deux modèles :
//! - [`ClaimModel::Broadcast`] : l'ancien (tout le monde tente). Simple, petit N.
//!   L'invariant gelé `gagnés+perdus == items×R` reste vrai.
//! - [`ClaimModel::Sharded`] : item `i` POSSÉDÉ par `shard_owner(i)%R` ; seul le
//!   propriétaire fait le CAS → **O(items)**, ~0 conflit, chaque item réclamé
//!   exactement une fois. Le **vol** (stealing) permet de récupérer les items
//!   d'un propriétaire lent/absent, avec une contention BORNÉE (≤ items volés).
//!
//! Compose avec WS1 : [`shard_topic_for_item`] route la notif d'un item vers le
//! topic de SON propriétaire → un worker ne REÇOIT que ses items (fan-out réduit)
//! ET ne fait le CAS que sur eux (contention supprimée).

use std::sync::atomic::{AtomicU64, Ordering};

use efe_nano_sdk::AgentId;
use serde_json::json;

use crate::blackboard::Blackboard;
use crate::error::BlackboardError;

/// Propriétaire shardé d'un item (déterministe, identique pour tous les workers).
/// Finaliseur splitmix64 → bonne distribution même pour des ids structurés.
pub fn shard_owner(item: u64, workers: usize) -> usize {
    if workers <= 1 {
        return 0;
    }
    let mut z = item.wrapping_add(0x9E3779B97F4A7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^= z >> 31;
    (z % workers as u64) as usize
}

/// Topic de notification d'un worker (pour s'abonner à SON shard).
pub fn shard_topic(base: &str, worker: usize) -> String {
    format!("{base}.shard.{worker}")
}

/// Topic vers lequel router la notif d'un item (le shard de son propriétaire).
pub fn shard_topic_for_item(base: &str, item: u64, workers: usize) -> String {
    shard_topic(base, shard_owner(item, workers))
}

/// Modèle de réclamation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimModel {
    /// Tous les workers tentent chaque item — O(N×items).
    Broadcast,
    /// Item possédé par `shard_owner(i)%R` ; seul le propriétaire tente — O(items).
    Sharded {
        /// Autorise le vol des items non réclamés des autres shards.
        stealing: bool,
    },
}

/// Issue d'une tentative de réclamation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimOutcome {
    /// CAS réussi — ce worker possède l'item.
    Won,
    /// CAS en conflit — un autre worker a gagné.
    Lost,
    /// Pas le propriétaire (modèle shardé) — AUCUN CAS effectué (le gain WS4).
    NotOwner,
}

/// Statistiques cumulées d'un [`WorkQueue`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorkQueueStats {
    pub attempts: u64,
    pub won: u64,
    pub conflicts: u64,
    pub stolen: u64,
    pub skipped_not_owner: u64,
}

/// File de travail d'UN worker : décide s'il doit réclamer un item et exécute le
/// CAS sur le Blackboard. Tient ses propres compteurs (atomiques → partageable).
#[derive(Debug)]
pub struct WorkQueue {
    model: ClaimModel,
    worker: usize,
    workers: usize,
    claim_ns: String,
    attempts: AtomicU64,
    won: AtomicU64,
    conflicts: AtomicU64,
    stolen: AtomicU64,
    skipped_not_owner: AtomicU64,
}

impl WorkQueue {
    /// `worker` = index de CE worker (0..`workers`) ; `claim_ns` = préfixe des
    /// clés de claim (ex. `"/claim"`).
    pub fn new(model: ClaimModel, worker: usize, workers: usize, claim_ns: impl Into<String>) -> Self {
        Self {
            model,
            worker,
            workers: workers.max(1),
            claim_ns: claim_ns.into(),
            attempts: AtomicU64::new(0),
            won: AtomicU64::new(0),
            conflicts: AtomicU64::new(0),
            stolen: AtomicU64::new(0),
            skipped_not_owner: AtomicU64::new(0),
        }
    }

    pub fn worker(&self) -> usize {
        self.worker
    }

    /// Ce worker possède-t-il cet item (selon le modèle) ?
    pub fn owns(&self, item: u64) -> bool {
        match self.model {
            ClaimModel::Broadcast => true,
            ClaimModel::Sharded { .. } => shard_owner(item, self.workers) == self.worker,
        }
    }

    fn key(&self, item: u64) -> String {
        format!("{}/{}", self.claim_ns, item)
    }

    fn do_cas(&self, bb: &Blackboard, item: u64, agent: &AgentId, stealing: bool) -> ClaimOutcome {
        self.attempts.fetch_add(1, Ordering::Relaxed);
        match bb.write_if(&self.key(item), 0, json!(agent.as_str()), agent.clone()) {
            Ok(_) => {
                self.won.fetch_add(1, Ordering::Relaxed);
                if stealing {
                    self.stolen.fetch_add(1, Ordering::Relaxed);
                }
                ClaimOutcome::Won
            }
            Err(BlackboardError::Conflict { .. }) => {
                self.conflicts.fetch_add(1, Ordering::Relaxed);
                ClaimOutcome::Lost
            }
            // Autres erreurs (cap, pointeur) : traitées comme une non-acquisition.
            Err(_) => {
                self.conflicts.fetch_add(1, Ordering::Relaxed);
                ClaimOutcome::Lost
            }
        }
    }

    /// **WS4** : tente de réclamer un item. En modèle shardé, si ce worker n'en
    /// est pas propriétaire, renvoie [`ClaimOutcome::NotOwner`] **SANS CAS** (c'est
    /// la réduction de contention : (R−1) tentatives évitées par item).
    pub fn claim(&self, bb: &Blackboard, item: u64, agent: &AgentId) -> ClaimOutcome {
        if !self.owns(item) {
            self.skipped_not_owner.fetch_add(1, Ordering::Relaxed);
            return ClaimOutcome::NotOwner;
        }
        self.do_cas(bb, item, agent, false)
    }

    /// **WS4** : VOLE un item (CAS quel que soit l'ownership) — pour récupérer le
    /// travail d'un propriétaire lent/absent. Contention bornée : seuls les items
    /// effectivement volés génèrent d'éventuels conflits.
    pub fn steal(&self, bb: &Blackboard, item: u64, agent: &AgentId) -> ClaimOutcome {
        self.do_cas(bb, item, agent, true)
    }

    pub fn stats(&self) -> WorkQueueStats {
        WorkQueueStats {
            attempts: self.attempts.load(Ordering::Relaxed),
            won: self.won.load(Ordering::Relaxed),
            conflicts: self.conflicts.load(Ordering::Relaxed),
            stolen: self.stolen.load(Ordering::Relaxed),
            skipped_not_owner: self.skipped_not_owner.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::SwarmId;

    fn agent(s: &str) -> AgentId {
        AgentId::new(s).unwrap()
    }

    #[test]
    fn shard_owner_is_deterministic_and_in_range() {
        for item in 0..1000u64 {
            let o1 = shard_owner(item, 8);
            let o2 = shard_owner(item, 8);
            assert_eq!(o1, o2);
            assert!(o1 < 8);
        }
        assert_eq!(shard_owner(42, 1), 0);
    }

    #[test]
    fn shard_owner_partitions_items_across_all_workers() {
        let workers = 8;
        let mut counts = vec![0usize; workers];
        for item in 0..8000u64 {
            counts[shard_owner(item, workers)] += 1;
        }
        // distribution raisonnablement équilibrée (chaque shard non vide).
        assert!(counts.iter().all(|&c| c > 0));
        let max = *counts.iter().max().unwrap();
        let min = *counts.iter().min().unwrap();
        assert!(max < min * 2, "déséquilibre excessif: {counts:?}");
    }

    #[test]
    fn ownership_is_a_partition() {
        // chaque item est possédé par EXACTEMENT un worker.
        let workers = 5;
        for item in 0..500u64 {
            let owners: Vec<usize> = (0..workers)
                .filter(|&w| WorkQueue::new(ClaimModel::Sharded { stealing: false }, w, workers, "/c").owns(item))
                .collect();
            assert_eq!(owners.len(), 1, "item {item} possédé par {owners:?}");
        }
    }

    #[test]
    fn sharded_skips_non_owned_without_cas() {
        let bb = Blackboard::new(SwarmId::new());
        let wq = WorkQueue::new(ClaimModel::Sharded { stealing: false }, 0, 4, "/claim");
        let mut cas = 0;
        for item in 0..100u64 {
            match wq.claim(&bb, item, &agent("wkr")) {
                ClaimOutcome::NotOwner => {}
                _ => cas += 1,
            }
        }
        let s = wq.stats();
        assert_eq!(s.attempts, cas, "attempts == #items possédés");
        assert!(s.attempts < 100, "la plupart des items NON possédés → pas de CAS");
        assert_eq!(s.skipped_not_owner, 100 - s.attempts);
        assert_eq!(s.conflicts, 0, "pas de contention en shardé mono-worker");
    }

    #[test]
    fn broadcast_attempts_every_item() {
        let bb = Blackboard::new(SwarmId::new());
        let wq = WorkQueue::new(ClaimModel::Broadcast, 0, 4, "/claim");
        for item in 0..50u64 {
            assert_eq!(wq.claim(&bb, item, &agent("wkr")), ClaimOutcome::Won);
        }
        let s = wq.stats();
        assert_eq!(s.attempts, 50);
        assert_eq!(s.won, 50);
        assert_eq!(s.skipped_not_owner, 0);
    }

    #[test]
    fn steal_recovers_unclaimed_item() {
        let bb = Blackboard::new(SwarmId::new());
        // worker 1 vole un item possédé par un autre, non réclamé → gagne.
        let thief = WorkQueue::new(ClaimModel::Sharded { stealing: true }, 1, 4, "/claim");
        let victim_item = (0..).find(|&i| shard_owner(i, 4) != 1).unwrap();
        assert!(!thief.owns(victim_item));
        assert_eq!(thief.steal(&bb, victim_item, &agent("thief")), ClaimOutcome::Won);
        // un 2e vol sur le même item → perdu (déjà réclamé).
        let thief2 = WorkQueue::new(ClaimModel::Sharded { stealing: true }, 2, 4, "/claim");
        assert_eq!(thief2.steal(&bb, victim_item, &agent("thief2")), ClaimOutcome::Lost);
    }
}
