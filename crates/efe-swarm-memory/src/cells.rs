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
//! Le `content_hash` d'un instantané est `BLAKE3(canonical_json(cellules))`. Un `f64` dans le
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

    /// **ADR-019 (read-back A)** : les signaux BRUTS du prior pour le biais de pattern — purs,
    /// déterministes, sans politique (le clamp/échelle vit au resolver, qui connaît la base du
    /// sélecteur). `escalation_rate_milli` dit à quelle fréquence l'accord des candidats s'est
    /// révélé insuffisant (famille piégeuse vs fiable) ; `dominant_dispersion` dit, parmi les
    /// méthodes de DISPERSION (borda/condorcet — jamais le vote), laquelle a gagné nettement
    /// (marge ≥ `DISPERSION_MARGIN_MILLI`), ou `None` si aucune ne se détache ; `total` est la
    /// CONFIANCE. Sans donnée (`total == 0`) ⇒ tout à zéro/`None` → le resolver n'applique aucun
    /// biais (I-PAT2).
    pub fn bias(&self) -> PatternBias {
        let b = self.wins.get("borda").copied().unwrap_or(0);
        let c = self.wins.get("condorcet").copied().unwrap_or(0);
        let dom = self.dominant_dispersion(b, c);
        PatternBias { escalation_rate_milli: self.escalation_rate_milli(), dominant_dispersion: dom, total: self.total }
    }

    /// Méthode de dispersion dominante (borda vs condorcet) si la marge relative est NETTE.
    /// Déterministe ; égalité ou marge faible ⇒ `None` (la mémoire ne tranche pas, I-MEM4).
    fn dominant_dispersion(&self, borda: u64, condorcet: u64) -> Option<String> {
        let sum = borda + condorcet;
        if sum == 0 {
            return None;
        }
        let (lead, name) = if borda >= condorcet { (borda, "borda") } else { (condorcet, "condorcet") };
        let margin_milli = ((2 * lead).saturating_sub(sum).saturating_mul(1000)) / sum; // |b−c|/sum en milli
        if margin_milli >= DISPERSION_MARGIN_MILLI as u64 {
            Some(name.to_string())
        } else {
            None
        }
    }
}

/// **ADR-019 (read-back A)** : les signaux bruts du prior de pattern lus par le resolver pour
/// calculer le seuil de vote effectif (clampé) et le choix de méthode de dispersion. Données
/// pures — aucune politique, aucun flottant (déterminisme octet).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternBias {
    /// Taux d'escalade historique en milli — élevé ⇒ famille piégeuse (exiger plus d'accord).
    pub escalation_rate_milli: u32,
    /// Méthode de dispersion historiquement dominante (`"borda"`/`"condorcet"`), si marge nette.
    pub dominant_dispersion: Option<String>,
    /// Nombre de runs observés (la CONFIANCE qui échelonne le biais ; 0 ⇒ aucun biais).
    pub total: u64,
}

/// Marge relative minimale (milli) pour qu'une méthode de dispersion soit déclarée dominante.
const DISPERSION_MARGIN_MILLI: u32 = 200; // 20 % d'écart relatif borda↔condorcet

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

    /// **ADR-019 (read-back B, R-a)** : les **poids de routage** par-variante (milli), pour
    /// pondérer l'agrégation. Entier, déterministe, byte-stable (aucun flottant) — l'optimisme
    /// sous incertitude est rendu par un bonus **hyperbolique** `ROUTE_OPTIMISM/(total+1)` qui
    /// remplace le `√(ln T / tᵢ)` d'UCB (incalculable sans flottant) en gardant sa vertu : une
    /// variante **peu observée** n'est pas pénalisée, donc explorée. Garanties (I-REP2) :
    /// - poids **clampé** à `[ROUTE_W_LO, ROUTE_W_HI]` — **jamais zéro**, aucune variante éliminée ;
    /// - **cold-start neutre** : une variante absente/jamais vue ⇒ poids `1000` (neutre) ;
    /// - **éprouvée gagnante** favorisée ; **éprouvée perdante** down-weightée, jamais supprimée.
    ///
    /// Poids uniformes (toutes neutres) ⇒ l'agrégation pondérée est **byte-identique** (I-REP2).
    pub fn routing_weights(&self, variant_ids: &[String]) -> Vec<u32> {
        variant_ids
            .iter()
            .map(|id| {
                let (win_rate, total_v) = self.of(id).map(|r| (r.win_rate_milli(), r.total)).unwrap_or((0, 0));
                // bonus d'optimisme : grand quand peu observé (cold-start exploré, pas puni).
                let optimism = (ROUTE_OPTIMISM_MILLI as u64 / (total_v + 1)) as i64;
                let rep_score = win_rate as i64 + optimism; // « bonté » de la variante, en milli
                let delta = rep_score - ROUTE_REF_MILLI as i64; // écart au neutre (peut être négatif)
                let w = 1000 + (ROUTE_LAMBDA_MILLI as i64 * delta) / 1000;
                w.clamp(ROUTE_W_LO_MILLI as i64, ROUTE_W_HI_MILLI as i64) as u32
            })
            .collect()
    }

    /// **ADR-019 (read-back B, R-b/R-c)** : l'**ordonnancement de profils** du sizer adaptatif —
    /// quel profil d'échantillonnage `generator_{i:03}` essayer à quelle **position de vague**, pour
    /// que l'arrêt séquentiel (ADR-014) garde **d'abord les meilleurs** (R-b, priorisation) et
    /// n'atteigne le bas du classement **que s'il le faut** (R-c, gating implicite par arrêt
    /// anticipé — JAMAIS de suppression dure). Renvoie une **permutation** de `[0, k_max)` :
    /// `schedule[p]` = le profil exécuté à la position `p` de la séquence de vagues.
    ///
    /// Fonction **pure** de `(self, total, k_min, k_max)` — tous **pinnés dans l'instantané** (la
    /// réputation pour les poids, `total` pour la rotation, les bornes pour la spec) : la même
    /// entrée re-dérive le même ordonnancement hors-ligne (R1, ADR-007 — ni horloge, ni aléa).
    ///
    /// Garanties :
    /// - **permutation totale** — chaque profil exactement une fois ⇒ R-c ne **supprime jamais** un
    ///   profil : il le repousse au-delà de l'arrêt probable ; un run ultérieur (réputation qui
    ///   bouge) ou la rotation le **ré-atteint** ;
    /// - **priorisation (R-b)** — profils triés par **poids de routage décroissant**
    ///   ([`Self::routing_weights`], la même réputation/UCB que R-a) : les meilleurs d'abord ⇒
    ///   accord plus tôt ⇒ `K_used` plus petit **à iso-qualité** ;
    /// - **exploration déterministe** — le profil `e = total mod k_max` (**rotation keyée sur
    ///   `total`**, le compteur de folds pinné) est **forcé dans la région garantie-atteinte** (les
    ///   `k_min` premières positions) s'il n'y est pas déjà : AUCUN profil — fût-il éprouvé-perdant —
    ///   n'est affamé plus de `k_max` runs (couverture cyclique complète) ;
    /// - **genèse = identité** — `total == 0` sur une réputation vide ⇒ poids tous neutres ⇒ ordre
    ///   d'exploitation `[0,1,…)` et `e = 0` déjà en tête : ordonnancement = **identité**
    ///   (`variant = position`, le comportement historique I-AK3 ; aucun biais sans donnée, parité
    ///   avec I-REP2).
    pub fn profile_schedule(&self, total: u64, k_min: usize, k_max: usize) -> Vec<usize> {
        if k_max == 0 {
            return Vec::new();
        }
        // poids par profil — réutilise EXACTEMENT la cellule R-a (même réputation, même UCB clampé).
        let ids: Vec<String> = (0..k_max).map(|i| format!("generator_{i:03}")).collect();
        let weights = self.routing_weights(&ids);
        // ① EXPLOIT (R-b) : tri par poids DÉCROISSANT, départage par index CROISSANT (déterministe).
        let mut order: Vec<usize> = (0..k_max).collect();
        order.sort_by(|&a, &b| weights[b].cmp(&weights[a]).then(a.cmp(&b)));
        // ② EXPLORE (rotation keyée sur `total`) : forcer `e` dans la région garantie SEULEMENT s'il
        //    est relégué dans la queue gatée — sinon on ne perturbe rien (la genèse reste identité).
        let e = (total % k_max as u64) as usize;
        let k_min = k_min.clamp(1, k_max);
        if let Some(pos) = order.iter().position(|&p| p == e) {
            if pos >= k_min {
                let prof = order.remove(pos);
                order.insert(k_min - 1, prof); // dernière position garantie-atteinte (coût d'exploration minimal)
            }
        }
        order
    }
}

// ── read-back B : constantes de routage (milli) — la bande sans dégénérescence (I-REP2) ──
/// Référence « neutre » du score de réputation (= cold-start : poids 1000).
const ROUTE_REF_MILLI: u32 = 500;
/// Bonus d'optimisme à zéro observation (décroît en `1/(total+1)` — explore le peu-vu).
const ROUTE_OPTIMISM_MILLI: u32 = 500;
/// Gain : un écart plein au neutre atteint la borne de la bande (mappe l'écart sur `[W_LO,W_HI]`).
const ROUTE_LAMBDA_MILLI: u32 = 1000;
/// Plancher de poids — **jamais zéro** : aucune variante n'est jamais éliminée (anti-effondrement).
const ROUTE_W_LO_MILLI: u32 = 500;
/// Plafond de poids — la mémoire amorce, ne gouverne pas (une bonne variante ne peut tout rafler).
const ROUTE_W_HI_MILLI: u32 = 1500;

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
    /// Le `content_hash` **déterministe** des cellules (BLAKE3 du JSON canonique). L'épingle
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
    fn snapshot_pin_is_blake3_not_sha256() {
        // ADR-019 §P1.5 : l'épingle d'instantané est BLAKE3 du contenu canonique — cohérent avec
        // tout le reste de la couche (input/output hash, adresse d'exemplaire). VERROUILLE
        // l'algorithme : une régression vers SHA-256 (l'ancien état) échouerait ici.
        let mut c = MemoryCells::default();
        c.size.histogram.insert(6, 2);
        let v = serde_json::to_value(&c).expect("cellules sérialisables");
        let blake3 = efe_swarm_provenance::blake3_value(&v).0;
        let sha256 = efe_swarm_provenance::sha256(&efe_swarm_provenance::canonical_bytes(&v));
        assert_eq!(c.content_hash(), blake3, "l'épingle == BLAKE3(canonical) — l'ADR-017 appliqué");
        assert_ne!(c.content_hash(), sha256, "ce n'est PLUS du SHA-256 (anti-régression)");
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

    #[test]
    fn pattern_bias_exposes_raw_signals_and_no_data_is_inert() {
        // read-back A : sans donnée → aucun signal (I-PAT2 ; le resolver n'appliquera aucun biais).
        let b0 = PatternPrior::default().bias();
        assert_eq!((b0.escalation_rate_milli, b0.total), (0, 0));
        assert_eq!(b0.dominant_dispersion, None);
        // avec donnée : taux d'escalade + dispersion dominante NETTE.
        let mut p = PatternPrior { total: 10, escalations: 3, ..Default::default() };
        p.wins.insert("borda".into(), 8);
        p.wins.insert("condorcet".into(), 2);
        let b = p.bias();
        assert_eq!(b.escalation_rate_milli, 300);
        assert_eq!(b.dominant_dispersion.as_deref(), Some("borda"), "borda nettement dominant");
        assert_eq!(b.total, 10);
        // marge faible (égalité) → aucune dominante (la mémoire ne tranche pas, I-MEM4).
        let mut q = PatternPrior::default();
        q.wins.insert("borda".into(), 5);
        q.wins.insert("condorcet".into(), 5);
        assert_eq!(q.bias().dominant_dispersion, None, "égalité → aucun dominant");
    }

    #[test]
    fn routing_weights_are_clamped_optimistic_and_never_zero() {
        // read-back B (I-REP2) : éprouvée-gagnante favorisée, éprouvée-perdante down-weightée,
        // jamais-vue NEUTRE (cold-start), tout clampé [500,1500] (jamais zéro).
        let mut rep = Reputation::default();
        rep.agents.insert("generator_000".into(), AgentRep { verified: 20, won: 18, total: 20 }); // win 900
        rep.agents.insert("generator_001".into(), AgentRep { verified: 20, won: 1, total: 20 }); // win 50
        let ids = vec!["generator_000".to_string(), "generator_001".to_string(), "generator_002".to_string()];
        let w = rep.routing_weights(&ids);
        assert!(w[0] > 1000, "éprouvée gagnante favorisée : {}", w[0]);
        assert!(w[1] < 1000, "éprouvée perdante down-weightée : {}", w[1]);
        assert_eq!(w[2], 1000, "jamais vue → NEUTRE (cold-start, anti-effondrement)");
        assert!(w.iter().all(|&x| (500..=1500).contains(&x)), "borné [500,1500], jamais zéro : {w:?}");
        // toutes absentes → toutes neutres ⇒ agrégation pondérée byte-identique (I-REP2).
        assert!(Reputation::default().routing_weights(&ids).iter().all(|&x| x == 1000));
    }

    /// R-b/R-c : sans donnée (genèse), l'ordonnancement est l'**identité** — aucun biais, parité
    /// avec I-REP2 (le sizer se comporte EXACTEMENT comme avant la mémoire).
    #[test]
    fn profile_schedule_genesis_is_identity() {
        let sched = Reputation::default().profile_schedule(0, 4, 12);
        assert_eq!(sched, (0..12).collect::<Vec<_>>(), "genèse total=0 + réput vide ⇒ identité");
        // k_max == 0 → vide (pas de panique).
        assert!(Reputation::default().profile_schedule(0, 1, 0).is_empty());
    }

    /// R-b : les profils **éprouvés-gagnants** passent en tête (exploitation) — accord plus tôt.
    #[test]
    fn profile_schedule_prioritizes_proven_winners_first() {
        let mut rep = Reputation::default();
        for i in [6, 7, 8, 9] {
            rep.agents.insert(format!("generator_{i:03}"), AgentRep { verified: 20, won: 18, total: 20 });
        }
        for i in [0, 1, 2, 3, 4, 5, 10, 11] {
            rep.agents.insert(format!("generator_{i:03}"), AgentRep { verified: 1, won: 1, total: 20 });
        }
        // total=6 → rotation e=6 (∈ gagnants, déjà en tête) : pas de perturbation, exploit pur.
        let sched = rep.profile_schedule(6, 4, 12);
        assert_eq!(&sched[..4], &[6, 7, 8, 9], "les 4 gagnants d'abord (R-b)");
        let mut sorted = sched.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..12).collect::<Vec<_>>(), "permutation totale : aucun profil supprimé (R-c doux)");
    }

    /// R-c (exploration déterministe) : la rotation keyée sur `total` garantit que CHAQUE profil —
    /// fût-il éprouvé-perdant relégué en queue — entre dans la région garantie-atteinte (`k_min`
    /// premiers) au moins une fois par cycle de `k_max` runs : aucune famine, aucun effondrement.
    #[test]
    fn profile_schedule_rotation_reaches_every_profile_within_one_cycle() {
        let (k_min, k_max) = (4usize, 12usize);
        let mut rep = Reputation::default();
        for i in [6, 7, 8, 9] {
            rep.agents.insert(format!("generator_{i:03}"), AgentRep { verified: 20, won: 18, total: 20 });
        }
        // profil 0 est éprouvé-perdant → jamais dans le top par exploitation : seule la rotation l'atteint.
        rep.agents.insert("generator_000".into(), AgentRep { verified: 0, won: 0, total: 50 });
        let mut covered = std::collections::BTreeSet::new();
        for total in 0..k_max as u64 {
            let sched = rep.profile_schedule(total, k_min, k_max);
            let e = (total % k_max as u64) as usize;
            assert!(sched[..k_min].contains(&e), "rotation total={total} : profil {e} dans la région garantie");
            // déterminisme : re-dériver donne le même ordonnancement (re-prouvable, R1).
            assert_eq!(sched, rep.profile_schedule(total, k_min, k_max), "pur : même entrée → même sortie");
            covered.extend(sched[..k_min].iter().copied());
        }
        assert_eq!(covered, (0..k_max).collect(), "couverture cyclique COMPLÈTE : aucun profil affamé");
    }
}
