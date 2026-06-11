// SPDX-License-Identifier: UNLICENSED
//! # Catalogue d'agents **Mode A** (pur Rust, déterministe, quasi gratuit).
//!
//! ~34 petits agents composables — le cœur « beaucoup de petits agents, peu de
//! paramètres » bon marché. Chacun implémente un trait de topologie :
//! [`Decomposer`] · [`Stage`] · [`Verifier`] · [`Merger`] · [`Generator`].
//!
//! Familles : **décomposeurs** (7) · **transformations** (10) · **vérificateurs** (8)
//! · **agrégateurs** (7) · **générateurs** (2).

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::traits::{Decomposer, Generator, Merger, Stage, Verifier};

/// Nombre d'agents Mode A du catalogue.
pub const MODE_A_AGENT_COUNT: usize = 34;

// ─── helpers ─────────────────────────────────────────────────────────────────
fn text_of(v: &Value) -> String {
    v.get("text")
        .and_then(|x| x.as_str())
        .map(String::from)
        .unwrap_or_else(|| v.as_str().map(String::from).unwrap_or_default())
}
fn num_of(v: &Value) -> f64 {
    v.get("value")
        .and_then(|x| x.as_f64())
        .or_else(|| v.as_f64())
        .unwrap_or(0.0)
}
fn idx_of(v: &Value) -> i64 {
    v.get("idx").and_then(|x| x.as_i64()).unwrap_or(0)
}
fn with_idx(text: String, idx: usize) -> Value {
    json!({ "text": text, "idx": idx })
}

// ═══════════════════════════════════════════════════════════════════════════
//  DÉCOMPOSEURS (7) — task {text} → Vec<{text, idx}>
// ═══════════════════════════════════════════════════════════════════════════

/// Découpe en phrases (`.!?`).
pub struct SentenceSplit;
#[async_trait]
impl Decomposer for SentenceSplit {
    async fn decompose(&self, task: &Value) -> Vec<Value> {
        text_of(task)
            .split(['.', '!', '?'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .enumerate()
            .map(|(i, s)| with_idx(s.to_string(), i))
            .collect()
    }
}

/// Découpe en lignes.
pub struct LineSplit;
#[async_trait]
impl Decomposer for LineSplit {
    async fn decompose(&self, task: &Value) -> Vec<Value> {
        text_of(task)
            .lines()
            .enumerate()
            .map(|(i, s)| with_idx(s.to_string(), i))
            .collect()
    }
}

/// Découpe en mots.
pub struct WordSplit;
#[async_trait]
impl Decomposer for WordSplit {
    async fn decompose(&self, task: &Value) -> Vec<Value> {
        text_of(task)
            .split_whitespace()
            .enumerate()
            .map(|(i, s)| with_idx(s.to_string(), i))
            .collect()
    }
}

/// Découpe en paragraphes (lignes vides).
pub struct ParagraphSplit;
#[async_trait]
impl Decomposer for ParagraphSplit {
    async fn decompose(&self, task: &Value) -> Vec<Value> {
        text_of(task)
            .split("\n\n")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .enumerate()
            .map(|(i, s)| with_idx(s.to_string(), i))
            .collect()
    }
}

/// Découpe en chunks de `size` caractères.
pub struct CharChunk {
    pub size: usize,
}
#[async_trait]
impl Decomposer for CharChunk {
    async fn decompose(&self, task: &Value) -> Vec<Value> {
        let t = text_of(task);
        let chars: Vec<char> = t.chars().collect();
        let size = self.size.max(1);
        chars
            .chunks(size)
            .enumerate()
            .map(|(i, c)| with_idx(c.iter().collect::<String>(), i))
            .collect()
    }
}

/// Découpe un tableau JSON (`task.items` ou `task` array) en éléments.
pub struct JsonArraySplit;
#[async_trait]
impl Decomposer for JsonArraySplit {
    async fn decompose(&self, task: &Value) -> Vec<Value> {
        let arr = task
            .get("items")
            .and_then(|v| v.as_array())
            .or_else(|| task.as_array())
            .cloned()
            .unwrap_or_default();
        arr.into_iter()
            .enumerate()
            .map(|(i, mut v)| {
                if v.is_object() {
                    v["idx"] = json!(i);
                    v
                } else {
                    json!({ "value": v, "idx": i })
                }
            })
            .collect()
    }
}

/// Découpe le texte en `n` parts ~égales (par mots).
pub struct FixedCountSplit {
    pub n: usize,
}
#[async_trait]
impl Decomposer for FixedCountSplit {
    async fn decompose(&self, task: &Value) -> Vec<Value> {
        let t = text_of(task);
        let words: Vec<&str> = t.split_whitespace().collect();
        let n = self.n.max(1).min(words.len().max(1));
        let per = words.len().div_ceil(n).max(1);
        words
            .chunks(per)
            .enumerate()
            .map(|(i, c)| with_idx(c.join(" "), i))
            .collect()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  TRANSFORMATIONS (10) — Stage : {text} → Vec<{text}>
// ═══════════════════════════════════════════════════════════════════════════

/// Supprime les espaces de bord.
pub struct Trim;
#[async_trait]
impl Stage for Trim {
    async fn process(&self, input: &Value) -> Vec<Value> {
        vec![json!({ "text": text_of(input).trim(), "idx": idx_of(input) })]
    }
}

/// Minuscule.
pub struct Lowercase;
#[async_trait]
impl Stage for Lowercase {
    async fn process(&self, input: &Value) -> Vec<Value> {
        vec![json!({ "text": text_of(input).to_lowercase(), "idx": idx_of(input) })]
    }
}

/// Majuscule.
pub struct Uppercase;
#[async_trait]
impl Stage for Uppercase {
    async fn process(&self, input: &Value) -> Vec<Value> {
        vec![json!({ "text": text_of(input).to_uppercase(), "idx": idx_of(input) })]
    }
}

/// Normalise les espaces multiples en un seul.
pub struct NormalizeWhitespace;
#[async_trait]
impl Stage for NormalizeWhitespace {
    async fn process(&self, input: &Value) -> Vec<Value> {
        let t = text_of(input).split_whitespace().collect::<Vec<_>>().join(" ");
        vec![json!({ "text": t, "idx": idx_of(input) })]
    }
}

/// Filtre : ne laisse passer que le texte non vide (1→0/1).
pub struct FilterNonEmpty;
#[async_trait]
impl Stage for FilterNonEmpty {
    async fn process(&self, input: &Value) -> Vec<Value> {
        if text_of(input).trim().is_empty() {
            vec![]
        } else {
            vec![input.clone()]
        }
    }
}

/// Projette un champ vers `{text: <champ>}`.
pub struct ExtractField {
    pub field: String,
}
#[async_trait]
impl Stage for ExtractField {
    async fn process(&self, input: &Value) -> Vec<Value> {
        let v = input.get(&self.field).cloned().unwrap_or(Value::Null);
        vec![json!({ "text": v.as_str().map(String::from).unwrap_or_else(|| v.to_string()), "idx": idx_of(input) })]
    }
}

/// Renomme un champ `from` → `to`.
pub struct RenameField {
    pub from: String,
    pub to: String,
}
#[async_trait]
impl Stage for RenameField {
    async fn process(&self, input: &Value) -> Vec<Value> {
        let mut o = input.clone();
        if let Some(v) = o.get(&self.from).cloned() {
            if let Some(map) = o.as_object_mut() {
                map.remove(&self.from);
                map.insert(self.to.clone(), v);
            }
        }
        vec![o]
    }
}

/// Extrait les entiers du texte → `{value: <1er>, ints: [...]}`.
pub struct ExtractIntegers;
#[async_trait]
impl Stage for ExtractIntegers {
    async fn process(&self, input: &Value) -> Vec<Value> {
        let t = text_of(input);
        let ints: Vec<i64> = t
            .split(|c: char| !c.is_ascii_digit() && c != '-')
            .filter_map(|s| s.parse::<i64>().ok())
            .collect();
        let first = ints.first().copied().unwrap_or(0);
        vec![json!({ "value": first, "ints": ints, "idx": idx_of(input) })]
    }
}

/// Compte les mots → `{count, text}`.
pub struct WordCount;
#[async_trait]
impl Stage for WordCount {
    async fn process(&self, input: &Value) -> Vec<Value> {
        let t = text_of(input);
        vec![json!({ "count": t.split_whitespace().count(), "text": t, "idx": idx_of(input) })]
    }
}

/// Préfixe le texte.
pub struct PrefixText {
    pub prefix: String,
}
#[async_trait]
impl Stage for PrefixText {
    async fn process(&self, input: &Value) -> Vec<Value> {
        vec![json!({ "text": format!("{}{}", self.prefix, text_of(input)), "idx": idx_of(input) })]
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  VÉRIFICATEURS (8) — Verifier : score + passes
// ═══════════════════════════════════════════════════════════════════════════

/// Passe si le texte est non vide.
pub struct NonEmptyText;
impl Verifier for NonEmptyText {
    fn score(&self, _t: &Value, c: &Value) -> f64 {
        if text_of(c).trim().is_empty() {
            0.0
        } else {
            1.0
        }
    }
}

/// Passe si le candidat est un objet JSON.
pub struct IsObject;
impl Verifier for IsObject {
    fn score(&self, _t: &Value, c: &Value) -> f64 {
        if c.is_object() {
            1.0
        } else {
            0.0
        }
    }
}

/// Passe si le candidat possède le champ.
pub struct HasField {
    pub field: String,
}
impl Verifier for HasField {
    fn score(&self, _t: &Value, c: &Value) -> f64 {
        if c.get(&self.field).is_some() {
            1.0
        } else {
            0.0
        }
    }
}

/// Score 1 si `value == target` (à tolérance près).
pub struct NumericEquals {
    pub target: f64,
}
impl Verifier for NumericEquals {
    fn score(&self, _t: &Value, c: &Value) -> f64 {
        if (num_of(c) - self.target).abs() < 1e-9 {
            1.0
        } else {
            0.0
        }
    }
}

/// Passe si la longueur du texte ∈ [min, max].
pub struct LengthBounded {
    pub min: usize,
    pub max: usize,
}
impl Verifier for LengthBounded {
    fn score(&self, _t: &Value, c: &Value) -> f64 {
        let n = text_of(c).chars().count();
        if n >= self.min && n <= self.max {
            1.0
        } else {
            0.0
        }
    }
}

/// Passe si le texte contient le mot-clé (insensible à la casse).
pub struct KeywordPresent {
    pub keyword: String,
}
impl Verifier for KeywordPresent {
    fn score(&self, _t: &Value, c: &Value) -> f64 {
        if text_of(c).to_lowercase().contains(&self.keyword.to_lowercase()) {
            1.0
        } else {
            0.0
        }
    }
}

/// Style gold-test/schéma : passe si tous les champs requis sont présents.
pub struct RequiredFields {
    pub fields: Vec<String>,
}
impl Verifier for RequiredFields {
    fn score(&self, _t: &Value, c: &Value) -> f64 {
        let all = self.fields.iter().all(|f| c.get(f).is_some());
        if all {
            1.0
        } else {
            0.0
        }
    }
}

/// Vérificateur ORACLE : passe si `candidate.value|text == task.expected`.
pub struct EqualsExpected;
impl Verifier for EqualsExpected {
    fn score(&self, task: &Value, c: &Value) -> f64 {
        let exp = task.get("expected").cloned().unwrap_or(Value::Null);
        let got = c
            .get("value")
            .or_else(|| c.get("text"))
            .cloned()
            .unwrap_or_else(|| c.clone());
        if got == exp {
            1.0
        } else {
            0.0
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  AGRÉGATEURS (7) — Merger : (task, Vec<result>) → output
// ═══════════════════════════════════════════════════════════════════════════

/// Collecte tous les résultats dans un tableau.
pub struct CollectArray;
#[async_trait]
impl Merger for CollectArray {
    async fn merge(&self, _t: &Value, results: Vec<Value>) -> Value {
        json!({ "items": results })
    }
}

/// Concatène les textes (ordonnés par idx) avec un séparateur.
pub struct ConcatText {
    pub sep: String,
}
#[async_trait]
impl Merger for ConcatText {
    async fn merge(&self, _t: &Value, mut results: Vec<Value>) -> Value {
        results.sort_by_key(idx_of);
        let joined = results
            .iter()
            .map(text_of)
            .collect::<Vec<_>>()
            .join(&self.sep);
        json!({ "text": joined })
    }
}

/// Joint les textes par retour à la ligne (ordonnés par idx).
pub struct JoinLines;
#[async_trait]
impl Merger for JoinLines {
    async fn merge(&self, _t: &Value, mut results: Vec<Value>) -> Value {
        results.sort_by_key(idx_of);
        json!({ "text": results.iter().map(text_of).collect::<Vec<_>>().join("\n") })
    }
}

/// Somme les `value`.
pub struct SumValues;
#[async_trait]
impl Merger for SumValues {
    async fn merge(&self, _t: &Value, results: Vec<Value>) -> Value {
        json!({ "value": results.iter().map(num_of).sum::<f64>() })
    }
}

/// Valeur (`value` ou `text`) la plus fréquente.
pub struct MajorityValue;
#[async_trait]
impl Merger for MajorityValue {
    async fn merge(&self, _t: &Value, results: Vec<Value>) -> Value {
        use std::collections::HashMap;
        let mut counts: HashMap<String, (usize, Value)> = HashMap::new();
        for r in &results {
            let key_val = r.get("value").or_else(|| r.get("text")).cloned().unwrap_or(Value::Null);
            let key = key_val.to_string();
            let e = counts.entry(key).or_insert((0, key_val));
            e.0 += 1;
        }
        counts
            .into_values()
            .max_by_key(|(n, _)| *n)
            .map(|(_, v)| json!({ "value": v }))
            .unwrap_or(Value::Null)
    }
}

/// Premier résultat.
pub struct FirstResult;
#[async_trait]
impl Merger for FirstResult {
    async fn merge(&self, _t: &Value, results: Vec<Value>) -> Value {
        results.into_iter().next().unwrap_or(Value::Null)
    }
}

/// Résultat de score maximal (champ `score`).
pub struct MaxByScoreField;
#[async_trait]
impl Merger for MaxByScoreField {
    async fn merge(&self, _t: &Value, results: Vec<Value>) -> Value {
        results
            .into_iter()
            .max_by(|a, b| {
                let sa = a.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let sb = b.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
                sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(Value::Null)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  GÉNÉRATEURS (2) — Generator : produit un candidat
// ═══════════════════════════════════════════════════════════════════════════

/// Renvoie la tâche telle quelle (marquée du variant).
pub struct EchoGenerator;
#[async_trait]
impl Generator for EchoGenerator {
    async fn generate(&self, task: &Value, variant: usize) -> Value {
        let mut o = task.clone();
        if let Some(map) = o.as_object_mut() {
            map.insert("variant".into(), json!(variant));
        }
        o
    }
}

/// Renvoie toujours une valeur constante.
pub struct ConstGenerator {
    pub value: Value,
}
#[async_trait]
impl Generator for ConstGenerator {
    async fn generate(&self, _task: &Value, _variant: usize) -> Value {
        self.value.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn decomposers_split_correctly() {
        assert_eq!(WordSplit.decompose(&json!({"text":"a b c"})).await.len(), 3);
        assert_eq!(SentenceSplit.decompose(&json!({"text":"Hi. Bye!"})).await.len(), 2);
        assert_eq!(LineSplit.decompose(&json!({"text":"x\ny"})).await.len(), 2);
        assert_eq!(CharChunk { size: 2 }.decompose(&json!({"text":"abcde"})).await.len(), 3);
        assert_eq!(FixedCountSplit { n: 2 }.decompose(&json!({"text":"a b c d"})).await.len(), 2);
        assert_eq!(JsonArraySplit.decompose(&json!({"items":[1,2,3]})).await.len(), 3);
    }

    #[tokio::test]
    async fn transforms_apply() {
        assert_eq!(Uppercase.process(&json!({"text":"ab"})).await[0]["text"], "AB");
        assert_eq!(Lowercase.process(&json!({"text":"AB"})).await[0]["text"], "ab");
        assert_eq!(Trim.process(&json!({"text":"  x "})).await[0]["text"], "x");
        assert!(FilterNonEmpty.process(&json!({"text":"  "})).await.is_empty());
        assert_eq!(ExtractIntegers.process(&json!({"text":"a 12 b 7"})).await[0]["value"], 12);
        assert_eq!(WordCount.process(&json!({"text":"a b c"})).await[0]["count"], 3);
    }

    #[test]
    fn verifiers_score() {
        assert_eq!(NonEmptyText.score(&json!({}), &json!({"text":"x"})), 1.0);
        assert!(!NonEmptyText.passes(&json!({}), &json!({"text":""})));
        assert_eq!(NumericEquals { target: 4.0 }.score(&json!({}), &json!({"value":4})), 1.0);
        assert!(RequiredFields { fields: vec!["a".into(), "b".into()] }
            .passes(&json!({}), &json!({"a":1,"b":2})));
        assert!(EqualsExpected.passes(&json!({"expected":"tech"}), &json!({"value":"tech"})));
        assert!(KeywordPresent { keyword: "AI".into() }.passes(&json!({}), &json!({"text":"the ai system"})));
    }

    #[tokio::test]
    async fn mergers_aggregate() {
        let rs = vec![json!({"value":1}), json!({"value":2}), json!({"value":3})];
        assert_eq!(SumValues.merge(&json!({}), rs.clone()).await["value"], 6.0);
        assert_eq!(CollectArray.merge(&json!({}), rs.clone()).await["items"].as_array().unwrap().len(), 3);
        let texts = vec![json!({"text":"b","idx":1}), json!({"text":"a","idx":0})];
        assert_eq!(ConcatText { sep: " ".into() }.merge(&json!({}), texts).await["text"], "a b");
        let maj = vec![json!({"value":"x"}), json!({"value":"x"}), json!({"value":"y"})];
        assert_eq!(MajorityValue.merge(&json!({}), maj).await["value"], "x");
    }

    #[tokio::test]
    async fn generators_produce() {
        assert_eq!(EchoGenerator.generate(&json!({"text":"q"}), 3).await["variant"], 3);
        assert_eq!(ConstGenerator { value: json!(42) }.generate(&json!({}), 0).await, json!(42));
    }

    #[test]
    fn catalog_has_about_thirty_agents() {
        assert_eq!(MODE_A_AGENT_COUNT, 34, "catalogue Mode A : 34 agents");
    }
}
