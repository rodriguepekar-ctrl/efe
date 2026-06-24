// SPDX-License-Identifier: UNLICENSED
//! **B1.2 — le scorer : la mesure de justesse, un seul noyau partagé avec l'oracle.**
//!
//! Un scorer répond à UNE question binaire par tâche : la réponse gagnante de l'essaim
//! **égale-t-elle** la vérité-terrain ? Agrégé sur un corpus, c'est l'**exactitude**. Le principe
//! directeur est la **non-duplication** : il n'existe, dans tout EFE, qu'**une** définition de
//! « cette réponse correspond à cette vérité-terrain ».
//!
//! Le scorer [`ScorerKind::Exact`] réutilise *littéralement* le noyau de l'oracle interne
//! [`EqualsExpected`] — il construit une tâche synthétique `{ "expected": <label> }` et appelle son
//! `score()`. C'est **I-BENCH-SAME-ORACLE** réalisé dans le code : « noté juste par le banc » et
//! « accepté par le juge-oracle interne » sont, à l'octet, la **même** comparaison, sans dérive
//! possible. Les scorers `Numeric`/`Set` sont des extensions B1 pour les réponses fermées non
//! exactes (chiffres à tolérance, multi-étiquettes sans ordre) ; la notation floue/sémantique reste
//! une v2 nommée, non livrée (cf. l'architecture, §4).
//!
//! **I-BENCH-TRUTH (corollaire)** : le scorer doit correspondre au *type* de tâche. La v1 est
//! fermée (exact/numérique/ensembliste). Un dataset à réponse libre noté par égalité produirait une
//! exactitude artificiellement basse — l'erreur que cette invariant interdit.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use efe_swarm_topology::{catalog::EqualsExpected, Verifier};

/// La manière de comparer une réponse gagnante à la vérité-terrain.
///
/// Sérialisation (JSONL, champ `scorer`) : chaîne nue pour les variantes unitaires
/// (`"exact"`, `"set"`) ; objet pour la variante paramétrée (`{ "numeric": { "tolerance": 0.5 } }`).
/// Absent ⇒ [`ScorerKind::Exact`] (le défaut).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScorerKind {
    /// Égalité exacte — **réutilise le noyau de l'oracle** [`EqualsExpected`] (I-BENCH-SAME-ORACLE).
    /// Le défaut : un signal incontestable sur réponses fermées.
    #[default]
    Exact,
    /// Égalité numérique à une tolérance absolue (réponses chiffrées).
    Numeric { tolerance: f64 },
    /// Égalité d'ensembles (réponses multi-étiquettes, ordre indifférent).
    Set,
    /// **Texte TOLÉRANT** (éval de vrais modèles) : casse/espaces/ponctuation ignorés, et le label
    /// compte comme correct s'il **apparaît comme un mot** de la réponse (« Finance. », « the topic
    /// is finance » → `finance`). Mesure le FOND, pas la forme. **Permissif** par construction (une
    /// réponse bavarde contenant le mot compte) ; à n'employer qu'avec un prompt strict (« un seul
    /// mot »). Distinct de l'oracle exact (I-BENCH-SAME-ORACLE ne s'applique qu'à `Exact`).
    Lenient,
}

impl ScorerKind {
    /// Étiquette stable (logs/rapports).
    pub fn label(&self) -> &'static str {
        match self {
            ScorerKind::Exact => "exact",
            ScorerKind::Numeric { .. } => "numeric",
            ScorerKind::Set => "set",
            ScorerKind::Lenient => "lenient",
        }
    }

    /// La réponse gagnante de l'essaim est-elle **correcte** au regard de la vérité-terrain ?
    ///
    /// `winner` est la sortie brute du run (typiquement `{ "value": … }`) ; `label` est la
    /// vérité-terrain (depuis le dataset, jamais montrée aux générateurs — I-BENCH-BLIND).
    pub fn is_correct(&self, winner: &Value, label: &Value) -> bool {
        match self {
            // I-BENCH-SAME-ORACLE : on appelle LITTÉRALEMENT le noyau de l'oracle interne. Son
            // `score()` extrait `winner.value | winner.text | winner` et compare à `task.expected`
            // — exactement la sémantique d'acceptation du juge-oracle de consensus. On pèle d'abord
            // un éventuel emballage de diagnostic (consensus adaptatif / gossip / débat) pour que la
            // note tienne quelle que soit la topologie, pas seulement le K fixe.
            ScorerKind::Exact => EqualsExpected.score(&json!({ "expected": label }), unwrap_diag(winner)) >= 1.0,
            ScorerKind::Numeric { tolerance } => match (as_number(winner), as_number(label)) {
                (Some(got), Some(exp)) => (got - exp).abs() <= *tolerance,
                _ => false,
            },
            ScorerKind::Set => match (as_set(winner), as_set(label)) {
                (Some(got), Some(exp)) => got == exp,
                _ => false,
            },
            ScorerKind::Lenient => {
                let label_n = lenient_norm(&value_text(label));
                if label_n.is_empty() {
                    return false;
                }
                let answer = value_text(extract(winner));
                lenient_norm(&answer) == label_n || answer.split_whitespace().any(|t| lenient_norm(t) == label_n)
            }
        }
    }
}

/// Le texte d'une valeur JSON : la chaîne nue si c'en est une, sinon sa forme sérialisée.
fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Normalisation tolérante : minuscule, espaces rognés, ponctuation de bord retirée.
fn lenient_norm(s: &str) -> String {
    s.trim().to_lowercase().trim_matches(|c: char| !c.is_alphanumeric()).to_string()
}

/// Pèle un **emballage de diagnostic** : le consensus adaptatif rend `{ value, sizing }`, le gossip
/// `{ value, gossip }`, le débat `{ value, debate }` (la grammaire « on rend l'observable, pas
/// seulement la décision »). La réponse réelle est alors sous `value`. On ne pèle QUE si un de ces
/// frères diagnostiques est présent à côté de `value` — un objet-réponse légitime ne les porte pas,
/// donc aucun faux positif. Le K fixe (gagnant nu) traverse inchangé.
fn unwrap_diag(v: &Value) -> &Value {
    if let Value::Object(m) = v {
        if m.contains_key("value") && (m.contains_key("sizing") || m.contains_key("gossip") || m.contains_key("debate")) {
            return &m["value"];
        }
    }
    v
}

/// Extraction de la réponse — **même ordre que l'oracle** : `value` → `text` → valeur brute,
/// après avoir pelé un éventuel emballage de diagnostic.
fn extract(v: &Value) -> &Value {
    let v = unwrap_diag(v);
    v.get("value").or_else(|| v.get("text")).unwrap_or(v)
}

/// Interprète une réponse comme un nombre : nombre JSON, ou chaîne parsable (tolérante au trim).
fn as_number(v: &Value) -> Option<f64> {
    let a = extract(v);
    a.as_f64().or_else(|| a.as_str().and_then(|s| s.trim().parse::<f64>().ok()))
}

/// Interprète une réponse comme un ensemble : un tableau JSON → ensemble de représentations canoniques.
fn as_set(v: &Value) -> Option<BTreeSet<String>> {
    match extract(v) {
        Value::Array(items) => Some(items.iter().map(|x| x.to_string()).collect()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_reuses_the_oracle_kernel_value_text_raw() {
        // Le gagnant peut être enveloppé {value:…}, {text:…}, ou brut — comme l'oracle l'extrait.
        let s = ScorerKind::Exact;
        assert!(s.is_correct(&json!({ "value": "finance" }), &json!("finance")));
        assert!(s.is_correct(&json!({ "text": "finance" }), &json!("finance")));
        assert!(s.is_correct(&json!("finance"), &json!("finance")));
        assert!(!s.is_correct(&json!({ "value": "tumor" }), &json!("health")));
        // I-BENCH-SAME-ORACLE prouvé : MÊME verdict que l'oracle appelé directement.
        let oracle_verdict = EqualsExpected.score(&json!({ "expected": "finance" }), &json!({ "value": "finance" })) >= 1.0;
        assert_eq!(s.is_correct(&json!({ "value": "finance" }), &json!("finance")), oracle_verdict);
    }

    #[test]
    fn exact_peels_adaptive_and_gossip_diagnostic_wrappers() {
        let s = ScorerKind::Exact;
        // Consensus ADAPTATIF : sortie = { value: <gagnant>, sizing: … } — le gagnant est lui-même {value}.
        let adaptive = json!({ "value": { "value": "finance" }, "sizing": { "k_used": 6 } });
        assert!(s.is_correct(&adaptive, &json!("finance")), "le wrapper de sizing est pelé");
        // Gossip : { value, gossip: … }.
        let gossip = json!({ "value": { "value": 100.0 }, "gossip": { "rounds": 3 } });
        assert!(ScorerKind::Numeric { tolerance: 0.1 }.is_correct(&gossip, &json!(100.0)), "wrapper gossip pelé");
        // Faux positif impossible : un objet-réponse {value} SANS frère diagnostique n'est pas pelé.
        assert!(s.is_correct(&json!({ "value": "finance" }), &json!("finance")));
    }

    #[test]
    fn numeric_honours_tolerance() {
        let s = ScorerKind::Numeric { tolerance: 0.5 };
        assert!(s.is_correct(&json!({ "value": 42.3 }), &json!(42.0)));
        assert!(s.is_correct(&json!({ "value": "42.3" }), &json!(42.0)), "chaîne parsable acceptée");
        assert!(!s.is_correct(&json!({ "value": 43.0 }), &json!(42.0)));
        assert!(!s.is_correct(&json!({ "value": "not a number" }), &json!(42.0)));
    }

    #[test]
    fn set_is_order_insensitive() {
        let s = ScorerKind::Set;
        assert!(s.is_correct(&json!({ "value": ["b", "a"] }), &json!(["a", "b"])));
        assert!(!s.is_correct(&json!({ "value": ["a"] }), &json!(["a", "b"])));
        assert!(!s.is_correct(&json!({ "value": "a" }), &json!(["a"])), "non-tableau → faux");
    }

    #[test]
    fn lenient_ignores_form_and_finds_the_word() {
        let s = ScorerKind::Lenient;
        // Casse + ponctuation ignorées.
        assert!(s.is_correct(&json!({ "value": "Finance." }), &json!("finance")));
        assert!(s.is_correct(&json!({ "value": "  FINANCE  " }), &json!("finance")));
        // Le mot apparaît dans une réponse bavarde (prompt strict supposé, mais robuste).
        assert!(s.is_correct(&json!({ "value": "The topic is finance." }), &json!("finance")));
        // Un vrai écart reste faux.
        assert!(!s.is_correct(&json!({ "value": "sports" }), &json!("sport")), "sports ≠ sport");
        assert!(!s.is_correct(&json!({ "value": "health" }), &json!("finance")));
    }

    #[test]
    fn scorer_kind_serde_roundtrip() {
        // Variantes unitaires = chaînes nues ; paramétrée = objet.
        assert_eq!(serde_json::from_str::<ScorerKind>("\"exact\"").unwrap(), ScorerKind::Exact);
        assert_eq!(serde_json::from_str::<ScorerKind>("\"set\"").unwrap(), ScorerKind::Set);
        let n: ScorerKind = serde_json::from_str("{\"numeric\":{\"tolerance\":0.25}}").unwrap();
        assert_eq!(n, ScorerKind::Numeric { tolerance: 0.25 });
        assert_eq!(ScorerKind::default(), ScorerKind::Exact);
    }
}
