// SPDX-License-Identifier: UNLICENSED
//! **HA.2 — le corpus pilote de substitution (T-MATH + T-EXTRACT), anti-contamination par construction.**
//!
//! Pour mesurer si un petit modèle substitue un gros (H-A, §7 de l'architecture), il faut un corpus
//! **labellisé**, **fermé-scorable**, **dimensionné pour la puissance**, et surtout **propre de toute
//! contamination** — la faille n°1 des benchmarks de SLM (I-HA-CONTAM). La stratégie la plus forte :
//! **générer** les items synthétiquement, à paramètres aléatoires mais **déterministes** (graine fixe).
//! Les instances spécifiques (ces nombres-là, ces entités-là, ces formulations-là) n'ont **jamais**
//! existé dans aucun corpus d'entraînement → aucune réponse n'a pu être mémorisée → la justesse
//! mesure la **capacité** (arithmétique, extraction), pas le rappel. Bonus : on dimensionne à tout n
//! (puissance), la difficulté est contrôlée (tiers 1→3 pour discriminer petit vs gros), et le corpus
//! est **re-dérivable** (graine → corpus → empreinte stable → lié au bundle).
//!
//! Deux classes, alignées sur les scorers fermés de B1 :
//! - **T-MATH** : problèmes arithmétiques (1 à 3 opérations), réponse = entier → [`ScorerKind::Numeric`].
//! - **T-EXTRACT** : « lister les {catégorie} mentionnés » parmi des distracteurs d'autres catégories,
//!   réponse = ensemble trié → [`ScorerKind::Set`].
//!
//! Chaque item est **auto-contenu** (la question est dans le texte) et **aveugle** (le label n'est
//! jamais une clé de la tâche — I-HA-BLIND/I-BENCH-BLIND). Module **pur** : aucune inférence.

use serde_json::{json, Value};

use crate::dataset::BenchCase;
use crate::scorer::ScorerKind;

// ─────────────────────────────────────────────────────────────────────────────
//  PRNG déterministe (SplitMix64) — re-prouvabilité du corpus
// ─────────────────────────────────────────────────────────────────────────────

struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
    /// Entier uniforme dans [lo, hi] **inclus** (suppose hi ≥ lo).
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next_u64() % (hi - lo + 1) as u64) as i64
    }
    fn pick<'a>(&mut self, pool: &[&'a str]) -> &'a str {
        pool[self.below(pool.len())]
    }
    /// `k` éléments DISTINCTS d'un pool (Fisher-Yates partiel).
    fn pick_distinct<'a>(&mut self, pool: &[&'a str], k: usize) -> Vec<&'a str> {
        let k = k.min(pool.len());
        let mut idx: Vec<usize> = (0..pool.len()).collect();
        for i in 0..k {
            let j = i + self.below(pool.len() - i);
            idx.swap(i, j);
        }
        idx[..k].iter().map(|&i| pool[i]).collect()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Lexiques (surface variée → nouveauté ; pools d'extraction DISJOINTS)
// ─────────────────────────────────────────────────────────────────────────────

const NAMES: &[&str] = &["Mia", "Leo", "Ava", "Noah", "Zoe", "Kai", "Ivy", "Eli", "Lena", "Omar", "Nina", "Theo"];
const ITEMS: &[&str] = &["apples", "marbles", "stickers", "coins", "books", "pencils", "cards", "cookies", "stamps", "beads"];

const ANIMALS: &[&str] = &["cat", "dog", "fox", "owl", "bear", "wolf", "deer", "hawk", "seal", "mouse", "goat", "swan"];
const FRUITS: &[&str] = &["apple", "banana", "cherry", "grape", "lemon", "mango", "peach", "plum", "melon", "kiwi", "pear", "fig"];
const VEHICLES: &[&str] = &["car", "bus", "bike", "train", "boat", "truck", "van", "jeep", "tram", "ferry", "scooter", "cart"];
const PLACES: &[&str] = &["market", "harbor", "station", "park", "fair", "museum"];

fn art(w: &str) -> &'static str {
    match w.chars().next() {
        Some(c) if "aeiou".contains(c) => "an",
        _ => "a",
    }
}

/// "a fox, a deer and an owl"
fn join_articled(words: &[&str]) -> String {
    let parts: Vec<String> = words.iter().map(|w| format!("{} {}", art(w), w)).collect();
    match parts.len() {
        0 => String::new(),
        1 => parts[0].clone(),
        _ => format!("{} and {}", parts[..parts.len() - 1].join(", "), parts[parts.len() - 1]),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  T-MATH — arithmétique fermée, réponse = entier positif (3 tiers de difficulté)
// ─────────────────────────────────────────────────────────────────────────────

/// Un item arithmétique tier 1 (une opération). Réponse positive garantie.
fn math_tier1(r: &mut Rng) -> (String, i64) {
    let (name, item) = (r.pick(NAMES), r.pick(ITEMS));
    match r.below(3) {
        0 => {
            let (a, b) = (r.range(11, 99), r.range(11, 99));
            (format!("{name} picked {a} {item} in the morning and {b} {item} in the afternoon. How many {item} did {name} pick in total?"), a + b)
        }
        1 => {
            let a = r.range(50, 99);
            let b = r.range(11, a - 10);
            (format!("{name} had {a} {item} and gave away {b}. How many {item} does {name} have left?"), a - b)
        }
        _ => {
            let (a, b) = (r.range(3, 12), r.range(11, 40));
            (format!("There are {a} boxes with {b} {item} in each. How many {item} are there altogether?"), a * b)
        }
    }
}

/// Tier 2 (deux opérations).
fn math_tier2(r: &mut Rng) -> (String, i64) {
    let (name, item) = (r.pick(NAMES), r.pick(ITEMS));
    match r.below(2) {
        0 => {
            let (a, b, c) = (r.range(2, 9), r.range(10, 30), r.range(10, 60));
            (format!("{name} bought {a} packs of {item} with {b} in each pack, and found {c} more {item}. How many {item} does {name} have?"), a * b + c)
        }
        _ => {
            let (a, b) = (r.range(3, 9), r.range(11, 25));
            let c = r.range(10, a * b - 10);
            (format!("A shelf holds {a} rows of {b} {item}. {c} {item} were taken away. How many {item} remain?"), a * b - c)
        }
    }
}

/// Tier 3 (trois opérations).
fn math_tier3(r: &mut Rng) -> (String, i64) {
    let (name, item) = (r.pick(NAMES), r.pick(ITEMS));
    match r.below(2) {
        0 => {
            let (a, b, c) = (r.range(20, 60), r.range(2, 6), r.range(10, 30));
            let base = a + b * c;
            let d = r.range(10, base - 10);
            (format!("{name} started with {a} {item}, then got {b} bags holding {c} {item} each, and gave {d} {item} away. How many {item} does {name} have now?"), base - d)
        }
        _ => {
            let (a, b, c, d) = (r.range(2, 6), r.range(10, 20), r.range(2, 6), r.range(10, 20));
            (format!("{name} has {a} small jars with {b} {item} each and {c} large jars with {d} {item} each. How many {item} in total?"), a * b + c * d)
        }
    }
}

/// Génère un corpus **T-MATH** de `n` items (tiers 1→3 en round-robin pour un gradient de difficulté
/// garanti). Scorer numérique exact. Réponses = entiers positifs.
pub fn math(n: usize, seed: u64) -> Vec<BenchCase> {
    let mut r = Rng::new(seed ^ 0x4D41_5448); // "MATH"
    (0..n)
        .map(|idx| {
            let tier = idx % 3 + 1;
            let (text, ans) = match tier {
                1 => math_tier1(&mut r),
                2 => math_tier2(&mut r),
                _ => math_tier3(&mut r),
            };
            BenchCase::new(format!("m{tier}-{idx:04}"), json!({ "text": text }), json!(ans), ScorerKind::Numeric { tolerance: 0.0 })
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
//  T-EXTRACT — lister les {catégorie} parmi des distracteurs ; réponse = ensemble trié
// ─────────────────────────────────────────────────────────────────────────────

/// Génère un corpus **T-EXTRACT** de `n` items. Pour chaque item : une catégorie cible (animaux /
/// fruits / véhicules), 2 à 4 entités cibles DISTINCTES, et 2 distracteurs des AUTRES catégories
/// (disjointes → jamais confondables), tissés dans une phrase auto-contenue. Réponse = l'ensemble
/// trié des cibles. Scorer ensembliste (ordre indifférent).
pub fn extract(n: usize, seed: u64) -> Vec<BenchCase> {
    let mut r = Rng::new(seed ^ 0x4558_5452); // "EXTR"
    let cats: [(&str, &[&str]); 3] = [("animals", ANIMALS), ("fruits", FRUITS), ("vehicles", VEHICLES)];
    (0..n)
        .map(|idx| {
            let ci = r.below(3);
            let (cat_name, pool) = cats[ci];
            let k = r.range(2, 4) as usize;
            let mut targets = r.pick_distinct(pool, k);
            // Distracteurs : un de chacune des DEUX autres catégories (clairement hors-cible).
            let mut distractors: Vec<&str> = Vec::new();
            for (j, (_, other)) in cats.iter().enumerate() {
                if j != ci {
                    distractors.push(r.pick(other));
                }
            }
            let name = r.pick(NAMES);
            let place = r.pick(PLACES);
            let list = join_articled(&targets);
            let distract = join_articled(&distractors);
            let text = match r.below(3) {
                0 => format!("On the way to the {place}, {name} noticed {list}, and also {distract}. List all the {cat_name} mentioned."),
                1 => format!("{name}'s notebook described {list}, with {distract} nearby. List all the {cat_name} mentioned."),
                _ => format!("At the {place}, {name} pointed at {list}; {distract} stood just behind. List all the {cat_name} mentioned."),
            };
            // Le label : l'ensemble trié des cibles (tableau JSON de mots).
            targets.sort_unstable();
            let label: Value = json!(targets);
            BenchCase::new(format!("e-{idx:04}"), json!({ "text": text }), label, ScorerKind::Set)
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
//  Sérialisation JSONL (avec en-tête de provenance/anti-contamination)
// ─────────────────────────────────────────────────────────────────────────────

/// Sérialise un corpus en **JSONL** précédé d'un en-tête `#` (ignoré par [`crate::Dataset::from_jsonl`])
/// documentant la provenance — la déclaration explicite qu'exige I-HA-CONTAM.
pub fn to_jsonl(cases: &[BenchCase], header_lines: &[String]) -> String {
    let mut s = String::new();
    for h in header_lines {
        s.push_str("# ");
        s.push_str(h);
        s.push('\n');
    }
    for c in cases {
        s.push_str(&serde_json::to_string(c).expect("BenchCase sérialisable"));
        s.push('\n');
    }
    s
}

/// En-tête de provenance standard (I-HA-CONTAM : synthétique, novel, re-dérivable).
pub fn provenance_header(class: &str, n: usize, seed: u64) -> Vec<String> {
    vec![
        format!("EFE HA.2 — corpus pilote {class} (généré, anti-contamination par construction)."),
        format!("n={n} · seed={seed} · re-dérivable : `corpus::{class}({n}, {seed})`."),
        "Synthétique : instances à paramètres aléatoires → JAMAIS dans un corpus d'entraînement public.".to_string(),
        "Réponse fermée, label hors de la tâche (I-HA-BLIND). Provenance des labels = ce générateur.".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── déterminisme & re-prouvabilité ──

    #[test]
    fn generation_is_deterministic() {
        let a = to_jsonl(&math(50, 7), &[]);
        let b = to_jsonl(&math(50, 7), &[]);
        assert_eq!(a, b, "même graine → corpus byte-identique (re-dérivable)");
        let c = to_jsonl(&math(50, 8), &[]);
        assert_ne!(a, c, "graine ≠ → corpus ≠ (variété)");
        // Idem extraction.
        assert_eq!(to_jsonl(&extract(50, 3), &[]), to_jsonl(&extract(50, 3), &[]));
    }

    // ── T-MATH bien posé ──

    #[test]
    fn math_items_are_well_posed_and_self_consistent() {
        let cases = math(300, 42);
        assert_eq!(cases.len(), 300);
        for c in &cases {
            // Le label est un entier strictement positif.
            let ans = c.label.as_i64().expect("label entier");
            assert!(ans > 0, "réponse positive : {ans} pour {}", c.id);
            // Auto-cohérence : le label, présenté comme réponse, est scoré CORRECT par son scorer.
            assert!(c.scorer.is_correct(&json!({ "value": ans }), &c.label), "self-consistent : {}", c.id);
            // Le scorer est numérique exact.
            assert_eq!(c.scorer, ScorerKind::Numeric { tolerance: 0.0 });
            // I-HA-BLIND : aucune clé de label dans la tâche. (NB : pour l'arithmétique, le NOMBRE
            // réponse peut coïncider avec un chiffre d'un opérande présent dans l'énoncé — ce n'est
            // PAS une fuite : les opérandes sont la donnée, et le modèle doit toujours CALCULER quelle
            // combinaison les relie. L'invariant aveugle porte sur l'absence de clé de label, pas sur
            // l'absence du chiffre-réponse.)
            assert!(c.task.get("expected").is_none() && c.task.get("label").is_none());
            assert!(c.task.get("text").and_then(|t| t.as_str()).is_some(), "tâche auto-contenue : {}", c.id);
        }
    }

    #[test]
    fn math_spans_all_difficulty_tiers() {
        let cases = math(300, 1);
        for tier in 1..=3 {
            assert!(cases.iter().any(|c| c.id.starts_with(&format!("m{tier}-"))), "tier {tier} présent");
        }
        // Round-robin → ~100 par tier.
        let t3 = cases.iter().filter(|c| c.id.starts_with("m3-")).count();
        assert!((95..=105).contains(&t3), "≈100 items tier 3, eu {t3}");
    }

    // ── T-EXTRACT bien posé ──

    #[test]
    fn extract_items_are_well_posed_and_self_consistent() {
        let cases = extract(300, 42);
        assert_eq!(cases.len(), 300);
        for c in &cases {
            let arr = c.label.as_array().expect("label tableau");
            // Non vide, 2..=4, trié, sans doublon.
            assert!((2..=4).contains(&arr.len()), "2..4 cibles : {} a {}", c.id, arr.len());
            let words: Vec<&str> = arr.iter().map(|v| v.as_str().unwrap()).collect();
            let mut sorted = words.clone();
            sorted.sort_unstable();
            assert_eq!(words, sorted, "label trié : {}", c.id);
            let uniq: std::collections::BTreeSet<&str> = words.iter().copied().collect();
            assert_eq!(uniq.len(), words.len(), "label sans doublon : {}", c.id);
            // Auto-cohérence : le label scoré contre lui-même = correct (scorer ensembliste).
            assert!(c.scorer.is_correct(&json!({ "value": &c.label }), &c.label), "self-consistent : {}", c.id);
            assert_eq!(c.scorer, ScorerKind::Set);
            // Toutes les cibles APPARAISSENT dans le texte (extraction légitime) ...
            let text = c.task["text"].as_str().unwrap();
            for w in &words {
                assert!(text.contains(w), "cible '{w}' présente dans le texte : {}", c.id);
            }
            // ... et la tâche reste aveugle (aucune clé de label).
            assert!(c.task.get("expected").is_none() && c.task.get("label").is_none());
        }
    }

    #[test]
    fn extract_targets_and_distractors_are_disjoint_categories() {
        // Un distracteur (autre catégorie) ne doit jamais apparaître dans le label.
        let cases = extract(200, 9);
        for c in &cases {
            let label: std::collections::BTreeSet<String> =
                c.label.as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
            // Le label est entièrement inclus dans UNE seule catégorie (cibles homogènes).
            let in_animals = label.iter().all(|w| ANIMALS.contains(&w.as_str()));
            let in_fruits = label.iter().all(|w| FRUITS.contains(&w.as_str()));
            let in_vehicles = label.iter().all(|w| VEHICLES.contains(&w.as_str()));
            assert!(in_animals || in_fruits || in_vehicles, "label homogène d'une catégorie : {} {:?}", c.id, label);
        }
    }

    #[test]
    fn jsonl_roundtrips_through_b1_dataset() {
        // Le corpus généré se RECHARGE par le loader B1 (chargeurs JSONL) avec le bon scorer.
        let jsonl = to_jsonl(&math(20, 5), &provenance_header("math", 20, 5));
        let ds = crate::Dataset::from_jsonl(&jsonl).expect("rechargé");
        assert_eq!(ds.len(), 20, "l'en-tête # est ignoré, 20 cas chargés");
        assert!(ds.cases.iter().all(|c| matches!(c.scorer, ScorerKind::Numeric { .. })));
        let ex = to_jsonl(&extract(15, 5), &provenance_header("extract", 15, 5));
        let de = crate::Dataset::from_jsonl(&ex).expect("rechargé");
        assert_eq!(de.len(), 15);
        assert!(de.cases.iter().all(|c| c.scorer == ScorerKind::Set));
    }
}
