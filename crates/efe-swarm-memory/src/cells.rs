// SPDX-License-Identifier: UNLICENSED
//! # `cells` — les cinq espèces de cellules apprises + l'instantané gelé
//!
//! Une cellule mémoire est une donnée **typée**, accumulée par famille. MEM-1 en porte cinq
//! (architecture §3.3), ordonnées du plus sûr au plus riche :
//! 1. [`SizePrior`] — la distribution de `k_used` (warm-start du sizer).
//! 2. [`PatternPrior`] — le pattern gagnant + le taux d'escalade.
//! 3. [`Exemplar`]s — les meilleurs candidats vérifiés (few-shot), **bornés** + quality-weighted.
//! 4. [`Reputation`] — taux vérifié / gagnant par agent.
//! 5. [`FailureMemory`] — les familles/agents qui échouent.
//!
//! ## Pourquoi tout est en ENTIERS (et jamais en `f64`)
//! Le `content_hash` d'un instantané est `SHA-256(canonical_json(cellules))`. Un `f64` dans le
//! contenu haché menacerait la **stabilité octet-à-octet** (représentations, précision). Toutes
//! les grandeurs « fractionnaires » (qualité, taux) sont donc des **milli-unités** `u32`
//! (`x ∈ [0, 1000]`), calculées de façon déterministe. Le `content_hash` est ainsi reproductible
//! par un tiers — condition de l'épinglage (I-MEM1).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{content_hash_of, hex32};

/// Plafond par défaut d'exemplaires retenus par famille (borne spatiale, §7.1). Au-delà,
/// éviction **déterministe** quality-weighted (les meilleurs gardés ; à égalité, le hash tranche).
pub const DEFAULT_EXEMPLAR_CAP: usize = 8;

// ─────────────────────────────────────────────────────────────────────────────
//  ① SizePrior — la calibration de K
// ─────────────────────────────────────────────────────────────────────────────

/// Distribution de `k_used` des runs passés d'une famille : un histogramme `k → compte`.
/// Associatif (fusion = somme), borné (les `k` sont ≤ `k_max` du run, lui-même plafonné).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SizePrior {
    /// `k_used` observé → nombre de runs l'ayant atteint.
    pub histogram: BTreeMap<u32, u64>,
}

impl SizePrior {
    /// Nombre total d'observations.
    pub fn total(&self) -> u64 {
        self.histogram.values().copied().sum()
    }

    /// Le `k` au `pct`-ième percentile (déterministe : on marche les buckets triés). `None`
    /// si aucune observation.
    pub fn percentile(&self, pct: u32) -> Option<u32> {
        let total = self.total();
        if total == 0 {
            return None;
        }
        let pct = pct.min(100) as u64;
        // cible = plafond(total * pct / 100), au moins 1.
        let target = total.saturating_mul(pct).div_ceil(100).max(1);
        let mut acc = 0u64;
        for (&k, &count) in &self.histogram {
            acc += count;
            if acc >= target {
                return Some(k);
            }
        }
        self.histogram.keys().next_back().copied()
    }

    /// **Warm-start BORNÉ de `k_min` (I-MEM4)** : un percentile conservateur (p25 par défaut)
    /// du K historique, **clampé** à `[floor, ceil]` — les bornes dures du run (`k_min`,
    /// `k_max`). La mémoire ne peut donc JAMAIS proposer un `k_min` hors des bornes I-AK : elle
    /// **amorce**, elle ne gouverne pas. `None` (= pas d'amorce, départ standard) si aucune donnée.
    pub fn suggested_k_min(&self, floor: u32, ceil: u32) -> Option<u32> {
        let p = self.percentile(25)?;
        Some(p.clamp(floor.min(ceil), ceil.max(floor)))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  ② PatternPrior — le pattern d'agrégation gagnant
// ─────────────────────────────────────────────────────────────────────────────

/// Comptes du pattern gagnant (`"vote"`/`"borda"`/`"condorcet"`) + escalades observées.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatternPrior {
    /// pattern → nombre de runs où il a gagné.
    pub wins: BTreeMap<String, u64>,
    /// Nombre de runs ayant escaladé (vote peu confiant → juges).
    pub escalations: u64,
    /// Total de runs observés (pour le taux d'escalade).
    pub total: u64,
}

impl PatternPrior {
    /// Le pattern le plus souvent gagnant (à égalité : le nom le plus petit, déterministe).
    /// `None` si aucune donnée — la mémoire n'impose alors rien (I-MEM4).
    pub fn suggested_pattern(&self) -> Option<String> {
        self.wins
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
            .map(|(p, _)| p.clone())
    }

    /// Taux d'escalade en milli-unités `[0,1000]` (déterministe, entier).
    pub fn escalation_rate_milli(&self) -> u32 {
        if self.total == 0 {
            return 0;
        }
        ((self.escalations.saturating_mul(1000)) / self.total) as u32
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  ③ Exemplars — les candidats few-shot (bornés, quality-weighted)
// ─────────────────────────────────────────────────────────────────────────────

/// Un exemplaire : un candidat **vérifié/gagnant** d'un run passé, content-addressed.
/// `quality_milli ∈ [0,1000]` (entier — byte-stable au hachage).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exemplar {
    /// Hash de contenu (hex) du candidat — l'identité stable, jamais la valeur brute hors record.
    pub content_hash: String,
    /// Aperçu court (pour l'injection few-shot et le dashboard) — peut être caviardé (PII).
    pub preview: String,
    /// Qualité en milli-unités `[0,1000]` (taux de vérification/victoire du candidat).
    pub quality_milli: u32,
}

/// Ensemble **borné** d'exemplaires d'une famille. Tri **déterministe** par
/// `(quality_milli desc, content_hash asc)` ; éviction au-delà de `cap`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exemplars {
    pub items: Vec<Exemplar>,
    pub cap: usize,
}

impl Default for Exemplars {
    fn default() -> Self {
        Self { items: Vec::new(), cap: DEFAULT_EXEMPLAR_CAP }
    }
}

impl Exemplars {
    /// Dédup **un-par-`content_hash`** (en gardant la MEILLEURE qualité), puis ordonne
    /// (qualité décroissante, hash croissant) et tronque à `cap`. **Idempotent et
    /// déterministe** — appelé après chaque insertion par le fold.
    ///
    /// Le dédup est **indépendant** du tri d'éviction : `dedup_by` ne collapse que les
    /// éléments ADJACENTS, donc on trie d'ABORD par `(hash asc, qualité desc)` pour rendre
    /// les doublons d'un même hash contigus (la meilleure qualité en tête de groupe → c'est
    /// elle que `dedup_by` garde), AVANT de réordonner pour l'éviction. Sans cette première
    /// passe, un même candidat ré-appris avec une autre qualité survivrait en double et
    /// évincerait un exemplaire distinct sous le cap (contrat « identité = content_hash »).
    pub(crate) fn normalize(&mut self) {
        // ① dédup par hash, gardant la meilleure qualité (doublons rendus adjacents).
        self.items.sort_by(|a, b| {
            a.content_hash.cmp(&b.content_hash).then_with(|| b.quality_milli.cmp(&a.quality_milli))
        });
        self.items.dedup_by(|a, b| a.content_hash == b.content_hash);
        // ② ordre d'éviction : (qualité desc, hash asc), borné au cap.
        self.items.sort_by(|a, b| {
            b.quality_milli.cmp(&a.quality_milli).then_with(|| a.content_hash.cmp(&b.content_hash))
        });
        if self.items.len() > self.cap {
            self.items.truncate(self.cap);
        }
    }

    /// Les `n` meilleurs exemplaires (déjà en ordre déterministe).
    pub fn top(&self, n: usize) -> &[Exemplar] {
        &self.items[..n.min(self.items.len())]
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  ④ Reputation — la performance par agent
// ─────────────────────────────────────────────────────────────────────────────

/// Compteurs de réputation d'un agent (entiers, associatifs).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRep {
    /// Fois où l'agent a produit un candidat **vérifié**.
    pub verified: u64,
    /// Fois où l'agent a produit le candidat **gagnant**.
    pub won: u64,
    /// Total d'apparitions.
    pub total: u64,
}

impl AgentRep {
    /// Taux de vérification en milli-unités `[0,1000]`.
    pub fn verified_rate_milli(&self) -> u32 {
        if self.total == 0 { 0 } else { ((self.verified.saturating_mul(1000)) / self.total) as u32 }
    }
    /// Taux de victoire en milli-unités `[0,1000]`.
    pub fn win_rate_milli(&self) -> u32 {
        if self.total == 0 { 0 } else { ((self.won.saturating_mul(1000)) / self.total) as u32 }
    }
}

/// Réputation par agent (clé = `agent_id`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reputation {
    pub agents: BTreeMap<String, AgentRep>,
}

impl Reputation {
    pub fn of(&self, agent: &str) -> Option<&AgentRep> {
        self.agents.get(agent)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  ⑤ FailureMemory — ce qui échoue systématiquement
// ─────────────────────────────────────────────────────────────────────────────

/// Compteur d'échec d'une clé (famille ou agent).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailRec {
    pub fails: u64,
    pub runs: u64,
}

impl FailRec {
    /// Taux d'échec en milli-unités `[0,1000]`.
    pub fn fail_rate_milli(&self) -> u32 {
        if self.runs == 0 { 0 } else { ((self.fails.saturating_mul(1000)) / self.runs) as u32 }
    }
}

/// Mémoire d'échec (clé → compteur). Décroissance temporelle = hors MEM-1 (le temps est
/// non-déterministe ; une décroissance par-compte viendra plus tard).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureMemory {
    pub keys: BTreeMap<String, FailRec>,
}

impl FailureMemory {
    pub fn of(&self, key: &str) -> Option<&FailRec> {
        self.keys.get(key)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  MemoryCells + MemorySnapshot
// ─────────────────────────────────────────────────────────────────────────────

/// L'état mémoire **matérialisé** d'une famille (les cinq espèces). Sérialisable en JSON
/// canonique → `content_hash` byte-stable. C'est la valeur que le fold produit et que
/// l'instantané gèle.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryCells {
    pub size: SizePrior,
    pub pattern: PatternPrior,
    pub exemplars: Exemplars,
    pub reputation: Reputation,
    pub failure: FailureMemory,
}

impl MemoryCells {
    /// Le `content_hash` **déterministe** des cellules (SHA-256 du JSON canonique). L'épingle
    /// d'ADR-017, appliquée à la mémoire : il identifie l'état EXACT lu par un run (I-MEM1).
    pub fn content_hash(&self) -> [u8; 32] {
        content_hash_of(self)
    }
}

/// L'**instantané gelé** lu par un run : la version, le `content_hash` (l'épingle), et les
/// cellules en lecture seule. C'est l'entrée mémoire du run — comme la tâche, identifiée par hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemorySnapshot {
    /// Version monotone (= nombre de replis pliés). Informatif ; l'identité est le `content_hash`.
    pub version: u64,
    /// Hash de contenu des cellules — l'épingle (I-MEM1/I-MEM7).
    pub content_hash: [u8; 32],
    /// Les cellules gelées (lecture seule pour la durée du run).
    pub cells: MemoryCells,
}

impl MemorySnapshot {
    /// Gèle des cellules à une version (calcule le `content_hash`).
    pub fn freeze(version: u64, cells: MemoryCells) -> Self {
        let content_hash = cells.content_hash();
        Self { version, content_hash, cells }
    }
    /// L'instantané genèse (mémoire vide) — ce que lit un run sur une famille jamais vue.
    pub fn genesis() -> Self {
        Self::freeze(0, MemoryCells::default())
    }
    /// `content_hash` en hex (persistance / provenance / fil).
    pub fn content_hash_hex(&self) -> String {
        hex32(&self.content_hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_is_deterministic_and_order_insensitive() {
        // I-MEM1 : deux cellules de même CONTENU (insérées dans des ordres différents) ont le
        // MÊME content_hash — les BTreeMap canonicalisent, le JSON canonique trie. L'épingle
        // est reproductible par un tiers.
        let mut a = MemoryCells::default();
        a.size.histogram.insert(6, 3);
        a.size.histogram.insert(4, 1);
        a.reputation.agents.insert("g".into(), AgentRep { verified: 2, won: 1, total: 3 });
        let mut b = MemoryCells::default();
        b.reputation.agents.insert("g".into(), AgentRep { verified: 2, won: 1, total: 3 });
        b.size.histogram.insert(4, 1);
        b.size.histogram.insert(6, 3);
        assert_eq!(a.content_hash(), b.content_hash(), "même contenu → même hash (ordre neutre)");
        // Un changement de contenu change le hash.
        b.size.histogram.insert(8, 1);
        assert_ne!(a.content_hash(), b.content_hash(), "contenu différent → hash différent");
    }

    #[test]
    fn size_prior_percentile_and_bounded_warm_start() {
        // p25 conservateur + clamp aux bornes du run (I-MEM4 : amorce, ne gouverne pas).
        let mut s = SizePrior::default();
        for k in [4u32, 4, 6, 6, 6, 8, 8, 12, 12, 24] {
            *s.histogram.entry(k).or_insert(0) += 1;
        }
        assert_eq!(s.total(), 10);
        // p25 de [4,4,6,6,6,8,8,12,12,24] : cible = ceil(10*25/100)=3 → 3e élément trié = 6.
        assert_eq!(s.percentile(25), Some(6));
        // warm-start clampé à [4, 24] → 6 (dans les bornes).
        assert_eq!(s.suggested_k_min(4, 24), Some(6));
        // BORNÉ : si les bornes du run sont [4, 5], la proposition NE PEUT PAS dépasser 5.
        assert_eq!(s.suggested_k_min(4, 5), Some(5), "clampé au k_max du run — jamais au-delà (I-MEM4)");
        // Pas de données → pas d'amorce (départ standard).
        assert_eq!(SizePrior::default().suggested_k_min(4, 24), None);
    }

    #[test]
    fn pattern_prior_argmax_is_deterministic() {
        let mut p = PatternPrior::default();
        p.wins.insert("borda".into(), 5);
        p.wins.insert("vote".into(), 5); // égalité → nom le plus petit ("borda")
        p.wins.insert("condorcet".into(), 2);
        p.total = 12;
        p.escalations = 3;
        assert_eq!(p.suggested_pattern(), Some("borda".into()), "égalité tranchée par le nom (déterministe)");
        assert_eq!(p.escalation_rate_milli(), 250, "3/12 = 250‰");
    }

    #[test]
    fn exemplars_are_bounded_and_quality_ordered() {
        let mut e = Exemplars { items: vec![], cap: 3 };
        for (h, q) in [("aa", 100u32), ("bb", 900), ("cc", 500), ("dd", 900), ("ee", 50)] {
            e.items.push(Exemplar { content_hash: h.into(), preview: h.into(), quality_milli: q });
        }
        e.normalize();
        assert_eq!(e.items.len(), 3, "borné au cap");
        // ordre : (900,bb),(900,dd) [égalité qualité → hash croissant], (500,cc).
        let hashes: Vec<&str> = e.items.iter().map(|x| x.content_hash.as_str()).collect();
        assert_eq!(hashes, vec!["bb", "dd", "cc"], "qualité desc, hash asc (déterministe)");
        assert_eq!(e.top(2).len(), 2);
    }

    #[test]
    fn normalize_dedups_one_per_hash_keeping_best_quality() {
        // RÉGRESSION (revue MEM-1) : un MÊME content_hash ré-appris avec une autre qualité ne
        // doit survivre qu'UNE fois, en gardant la MEILLEURE qualité — même quand un autre
        // exemplaire intercale les deux dans l'ordre de qualité (doublons non adjacents).
        let mut e = Exemplars { items: vec![], cap: 8 };
        for (h, q) in [("xx", 999u32), ("cc", 850), ("xx", 10)] {
            e.items.push(Exemplar { content_hash: h.into(), preview: h.into(), quality_milli: q });
        }
        e.normalize();
        let xx: Vec<&Exemplar> = e.items.iter().filter(|x| x.content_hash == "xx").collect();
        assert_eq!(xx.len(), 1, "un seul exemplaire par content_hash");
        assert_eq!(xx[0].quality_milli, 999, "on garde la MEILLEURE qualité du hash");
    }

    #[test]
    fn normalize_duplicate_never_evicts_a_distinct_exemplar_under_cap() {
        // RÉGRESSION : le repli incrémental (extend + normalize par run) ne doit pas laisser un
        // doublon fantôme occuper un slot du cap et évincer un exemplaire DISTINCT.
        let mk = |h: &str, q: u32| Exemplar { content_hash: h.into(), preview: h.into(), quality_milli: q };
        let mut e = Exemplars { items: vec![], cap: 3 };
        // run 1 : trois distincts.
        e.items.extend([mk("xx", 999), mk("dd", 900), mk("ee", 880)]);
        e.normalize();
        // run 2 : le même 'xx' ré-appris (qualité moindre).
        e.items.push(mk("xx", 890));
        e.normalize();
        let kept: Vec<&str> = e.items.iter().map(|x| x.content_hash.as_str()).collect();
        assert_eq!(kept, vec!["xx", "dd", "ee"], "top-3 distincts ; le doublon n'évince PAS ee");
        assert_eq!(e.items.len(), 3);
        assert_eq!(e.items[0].quality_milli, 999, "xx garde sa meilleure qualité");
    }

    #[test]
    fn rates_are_integer_milli_units() {
        let r = AgentRep { verified: 3, won: 1, total: 4 };
        assert_eq!(r.verified_rate_milli(), 750);
        assert_eq!(r.win_rate_milli(), 250);
        assert_eq!(AgentRep::default().verified_rate_milli(), 0, "0 total → 0, jamais de division par zéro");
        let f = FailRec { fails: 1, runs: 8 };
        assert_eq!(f.fail_rate_milli(), 125);
    }

    #[test]
    fn genesis_snapshot_is_empty_and_versioned_zero() {
        let g = MemorySnapshot::genesis();
        assert_eq!(g.version, 0);
        assert_eq!(g.cells, MemoryCells::default());
        assert_eq!(g.content_hash, MemoryCells::default().content_hash());
    }
}
