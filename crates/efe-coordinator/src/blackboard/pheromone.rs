// SPDX-License-Identifier: UNLICENSED
//! `PheromoneBlackboard` — implémentation bornée + stigmergique.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{CoordError, CoordResult};

use super::entry::{BlackboardEntry, EntryKind};
use super::{MAX_BLACKBOARD_BYTES, MAX_ENTRIES, PHEROMONE_GC_THRESHOLD};

/// État interne (read-mostly).
#[derive(Default)]
struct Inner {
    /// Map UUID v7 → entry (BTreeMap = ordre temporel garanti).
    entries: BTreeMap<Uuid, BlackboardEntry>,
    /// Compteur d'octets vivants.
    bytes_used: usize,
}

/// Blackboard concurrente bornée 256 Ko, stigmergique.
///
/// ## Garanties
///
/// - **I2** : `bytes_used <= MAX_BLACKBOARD_BYTES` à tout moment.
/// - **I7** : `current_intensity` est monotone décroissante par entrée.
/// - Lecture concurrente sans contention (RwLock partagé).
/// - Itération ordonnée par `entry_id` (UUID v7 = temps).
#[derive(Clone, Default)]
pub struct PheromoneBlackboard {
    inner: Arc<RwLock<Inner>>,
}

impl PheromoneBlackboard {
    /// Construit une blackboard vide.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Dépose une phéromone.  Échoue si la blackboard est pleine.
    ///
    /// Garbage-collecte d'abord les entrées sous le seuil pour faire de la
    /// place (cf. [`PHEROMONE_GC_THRESHOLD`]).
    pub fn deposit(&self, entry: BlackboardEntry) -> CoordResult<()> {
        let mut guard = self.inner.write();
        let new_size = entry.size_bytes;

        // GC opportuniste avant d'évaluer la borne.
        self.gc_locked(&mut guard, Utc::now());

        // Si on déborde encore en octets, on essaie un GC LRU agressif
        // (suppression de la plus vieille entrée jusqu'à passer sous le seuil).
        while guard.bytes_used + new_size > MAX_BLACKBOARD_BYTES
            && !guard.entries.is_empty()
        {
            if let Some((&oldest_id, oldest)) = guard.entries.iter().next() {
                let freed = oldest.size_bytes;
                guard.bytes_used = guard.bytes_used.saturating_sub(freed);
                guard.entries.remove(&oldest_id);
            } else {
                break;
            }
        }

        if guard.bytes_used + new_size > MAX_BLACKBOARD_BYTES {
            return Err(CoordError::BlackboardFull {
                used_bytes: guard.bytes_used,
                max_bytes: MAX_BLACKBOARD_BYTES,
                entry_bytes: new_size,
            });
        }

        // Borne entrées.
        if guard.entries.len() >= MAX_ENTRIES {
            if let Some((&oldest_id, oldest)) = guard.entries.iter().next() {
                let freed = oldest.size_bytes;
                guard.bytes_used = guard.bytes_used.saturating_sub(freed);
                guard.entries.remove(&oldest_id);
            }
        }

        guard.bytes_used += new_size;
        guard.entries.insert(entry.entry_id, entry);
        Ok(())
    }

    /// Lit toutes les entrées d'une certaine `kind`, ordonnées par dépôt
    /// croissant, en retournant l'intensité courante (post-decay).
    ///
    /// Les entrées sous le seuil GC sont filtrées.
    #[must_use]
    pub fn read_kind(&self, kind: EntryKind) -> Vec<(BlackboardEntry, f32)> {
        let now = Utc::now();
        let guard = self.inner.read();
        guard
            .entries
            .values()
            .filter(|e| e.kind == kind)
            .filter_map(|e| {
                let i = e.current_intensity(now);
                (i >= PHEROMONE_GC_THRESHOLD).then(|| (e.clone(), i))
            })
            .collect()
    }

    /// Lit toutes les entrées d'un auteur donné.
    #[must_use]
    pub fn read_author(&self, author: &str) -> Vec<(BlackboardEntry, f32)> {
        let now = Utc::now();
        let guard = self.inner.read();
        guard
            .entries
            .values()
            .filter(|e| e.author == author)
            .filter_map(|e| {
                let i = e.current_intensity(now);
                (i >= PHEROMONE_GC_THRESHOLD).then(|| (e.clone(), i))
            })
            .collect()
    }

    /// Lit l'entrée la plus récente pour une `kind` donnée (None si vide).
    #[must_use]
    pub fn read_latest(&self, kind: EntryKind) -> Option<BlackboardEntry> {
        let guard = self.inner.read();
        guard
            .entries
            .values()
            .rev()
            .find(|e| e.kind == kind)
            .cloned()
    }

    /// Snapshot statistique pour observabilité.
    #[must_use]
    pub fn stats(&self) -> BlackboardStats {
        let guard = self.inner.read();
        let now = Utc::now();
        let mut by_kind: std::collections::HashMap<EntryKind, usize> =
            std::collections::HashMap::new();
        for e in guard.entries.values() {
            *by_kind.entry(e.kind).or_insert(0) += 1;
        }
        let mut alive = 0;
        let mut min_int = 1.0_f32;
        let mut max_int = 0.0_f32;
        for e in guard.entries.values() {
            let i = e.current_intensity(now);
            if i >= PHEROMONE_GC_THRESHOLD {
                alive += 1;
                if i < min_int {
                    min_int = i;
                }
                if i > max_int {
                    max_int = i;
                }
            }
        }
        if alive == 0 {
            min_int = 0.0;
        }
        BlackboardStats {
            total_entries: guard.entries.len(),
            alive_entries: alive,
            bytes_used: guard.bytes_used,
            bytes_max: MAX_BLACKBOARD_BYTES,
            by_kind,
            min_intensity: min_int,
            max_intensity: max_int,
        }
    }

    /// Snapshot complet sérialisable (utile pour audit + tests).
    #[must_use]
    pub fn snapshot(&self) -> BlackboardSnapshot {
        let guard = self.inner.read();
        BlackboardSnapshot {
            entries: guard.entries.values().cloned().collect(),
            bytes_used: guard.bytes_used,
            captured_at: Utc::now(),
        }
    }

    /// Décroît manuellement toutes les entrées (utile pour les tests
    /// déterministes).  En usage normal, le decay est calculé à la lecture.
    pub fn force_decay(&self, factor: f32) {
        let factor = factor.clamp(0.0, 1.0);
        let mut guard = self.inner.write();
        for e in guard.entries.values_mut() {
            e.intensity = (e.intensity * factor).clamp(0.0, 1.0);
        }
        self.gc_locked(&mut guard, Utc::now());
    }

    /// Garbage collect : supprime les entrées dont l'intensité (avec decay
    /// temporel) est sous le seuil.  Doit être appelé avec lock W tenu.
    fn gc_locked(&self, guard: &mut Inner, now: DateTime<Utc>) {
        let dead: Vec<Uuid> = guard
            .entries
            .iter()
            .filter(|(_, e)| e.current_intensity(now) < PHEROMONE_GC_THRESHOLD)
            .map(|(id, _)| *id)
            .collect();
        for id in dead {
            if let Some(e) = guard.entries.remove(&id) {
                guard.bytes_used = guard.bytes_used.saturating_sub(e.size_bytes);
            }
        }
    }

    /// Vide entièrement la blackboard (utile pour les tests d'isolation).
    pub fn clear(&self) {
        let mut guard = self.inner.write();
        guard.entries.clear();
        guard.bytes_used = 0;
    }

    /// Octets actuellement consommés (snapshot atomique).
    #[must_use]
    pub fn bytes_used(&self) -> usize {
        self.inner.read().bytes_used
    }

    /// Nombre d'entrées (vivantes + mortes-en-attente-de-GC).
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.read().entries.len()
    }

    /// Blackboard est-elle vide ?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.read().entries.is_empty()
    }
}

/// Statistiques pour observabilité.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlackboardStats {
    /// Nombre total d'entrées (incluant celles qui vont être GCées).
    pub total_entries: usize,
    /// Nombre d'entrées vivantes (intensité >= seuil GC).
    pub alive_entries: usize,
    /// Octets consommés.
    pub bytes_used: usize,
    /// Borne max.
    pub bytes_max: usize,
    /// Comptage par catégorie.
    pub by_kind: std::collections::HashMap<EntryKind, usize>,
    /// Intensité min observée.
    pub min_intensity: f32,
    /// Intensité max observée.
    pub max_intensity: f32,
}

/// Snapshot complet (sérialisable pour audit + replay).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlackboardSnapshot {
    /// Toutes les entrées ordonnées par UUID (= temps).
    pub entries: Vec<BlackboardEntry>,
    /// Octets consommés au moment du snapshot.
    pub bytes_used: usize,
    /// Estampille du snapshot.
    pub captured_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small_entry(author: &str, kind: EntryKind) -> BlackboardEntry {
        BlackboardEntry::new(author, kind, serde_json::json!({"k":"v"}))
    }

    #[test]
    fn deposit_then_read_kind_works() {
        let bb = PheromoneBlackboard::new();
        bb.deposit(small_entry("analyzer", EntryKind::Entity)).unwrap();
        bb.deposit(small_entry("analyzer", EntryKind::Entity)).unwrap();
        bb.deposit(small_entry("verifier", EntryKind::Verdict)).unwrap();
        let ents = bb.read_kind(EntryKind::Entity);
        assert_eq!(ents.len(), 2);
        let verds = bb.read_kind(EntryKind::Verdict);
        assert_eq!(verds.len(), 1);
    }

    #[test]
    fn read_author_filters_correctly() {
        let bb = PheromoneBlackboard::new();
        bb.deposit(small_entry("a", EntryKind::Entity)).unwrap();
        bb.deposit(small_entry("b", EntryKind::Entity)).unwrap();
        assert_eq!(bb.read_author("a").len(), 1);
        assert_eq!(bb.read_author("b").len(), 1);
        assert!(bb.read_author("nobody").is_empty());
    }

    #[test]
    fn read_latest_returns_most_recent() {
        let bb = PheromoneBlackboard::new();
        bb.deposit(small_entry("x", EntryKind::Trace)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let second = small_entry("y", EntryKind::Trace);
        let second_id = second.entry_id;
        bb.deposit(second).unwrap();
        let last = bb.read_latest(EntryKind::Trace).unwrap();
        assert_eq!(last.entry_id, second_id);
    }

    #[test]
    fn full_blackboard_rejects() {
        let bb = PheromoneBlackboard::new();
        // Crée une entrée énorme (300 Ko) — devrait échouer.
        let big_payload = serde_json::json!({"big": "x".repeat(300_000)});
        let big = BlackboardEntry::new("x", EntryKind::Trace, big_payload);
        let r = bb.deposit(big);
        assert!(matches!(r, Err(CoordError::BlackboardFull { .. })));
    }

    #[test]
    fn lru_eviction_when_bytes_overflow() {
        let bb = PheromoneBlackboard::new();
        // 5 entrées de ~60 Ko chacune = 300 Ko total → la blackboard doit
        // évincer les anciennes pour rester sous 256 Ko.
        for i in 0..5 {
            let payload = serde_json::json!({"i": i, "pad": "x".repeat(60_000)});
            let e = BlackboardEntry::new(format!("a{i}"), EntryKind::Trace, payload);
            bb.deposit(e).unwrap();
        }
        assert!(bb.bytes_used() <= MAX_BLACKBOARD_BYTES);
        assert!(bb.len() < 5, "expected evictions, got {} entries", bb.len());
    }

    #[test]
    fn stats_account_for_alive_dead() {
        let bb = PheromoneBlackboard::new();
        bb.deposit(small_entry("x", EntryKind::Entity)).unwrap();
        bb.deposit(small_entry("y", EntryKind::Verdict)).unwrap();
        let s = bb.stats();
        assert_eq!(s.total_entries, 2);
        assert_eq!(s.alive_entries, 2);
        assert!(s.bytes_used > 0);
        assert_eq!(s.bytes_max, MAX_BLACKBOARD_BYTES);
    }

    #[test]
    fn snapshot_serializes() {
        let bb = PheromoneBlackboard::new();
        bb.deposit(small_entry("x", EntryKind::Trace)).unwrap();
        let snap = bb.snapshot();
        let s = serde_json::to_string(&snap).unwrap();
        let back: BlackboardSnapshot = serde_json::from_str(&s).unwrap();
        assert_eq!(back.entries.len(), 1);
    }

    #[test]
    fn force_decay_reduces_intensity() {
        let bb = PheromoneBlackboard::new();
        bb.deposit(small_entry("x", EntryKind::Trace)).unwrap();
        bb.force_decay(0.01); // 1% → sous le seuil 5% → GC
        let still = bb.read_kind(EntryKind::Trace);
        assert!(still.is_empty(), "should have GCed sub-threshold entries");
    }

    #[test]
    fn clear_resets_state() {
        let bb = PheromoneBlackboard::new();
        bb.deposit(small_entry("x", EntryKind::Trace)).unwrap();
        assert!(!bb.is_empty());
        bb.clear();
        assert!(bb.is_empty());
        assert_eq!(bb.bytes_used(), 0);
    }

    #[test]
    fn cloned_handles_share_state() {
        let bb1 = PheromoneBlackboard::new();
        let bb2 = bb1.clone();
        bb1.deposit(small_entry("x", EntryKind::Trace)).unwrap();
        assert_eq!(bb2.len(), 1, "clones must share state");
    }

    #[test]
    fn concurrent_deposits_are_safe() {
        use std::sync::Arc;
        let bb = Arc::new(PheromoneBlackboard::new());
        let mut handles = vec![];
        for i in 0..20 {
            let bb_c = bb.clone();
            handles.push(std::thread::spawn(move || {
                for j in 0..10 {
                    let e = BlackboardEntry::new(
                        format!("t{i}"),
                        EntryKind::Trace,
                        serde_json::json!({"i":i,"j":j}),
                    );
                    let _ = bb_c.deposit(e);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        // 20 × 10 = 200 dépôts mais une éviction LRU peut avoir lieu en
        // cas d'oversize ; on contrôle juste qu'on est dans la borne.
        assert!(bb.len() <= 200);
        assert!(bb.bytes_used() <= MAX_BLACKBOARD_BYTES);
    }
}
