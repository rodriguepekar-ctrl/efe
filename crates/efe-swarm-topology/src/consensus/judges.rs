// SPDX-License-Identifier: UNLICENSED
//! Juges **Mode A** (déterministes, ~gratuits) — les « votants » du consensus.
//!
//! [`VerifierJudge`] transforme N'IMPORTE QUEL `Verifier` (Phase 2) en juge → les
//! 8 vérificateurs du catalogue deviennent des juges instantanés. + 4 juges de
//! ranking aux critères ORTHOGONAUX (la diversité borne le gain, cf. S1).

use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::Judge;
use crate::traits::Verifier;

fn text_of(v: &Value) -> String {
    v.get("text")
        .and_then(|x| x.as_str())
        .map(String::from)
        .unwrap_or_else(|| v.as_str().map(String::from).unwrap_or_default())
}
fn words(s: &str) -> BTreeSet<String> {
    s.to_lowercase().split_whitespace().map(String::from).collect()
}

/// Adapte un [`Verifier`] (score indépendant) en [`Judge`].
pub struct VerifierJudge {
    ver: Arc<dyn Verifier>,
    name: String,
}
impl VerifierJudge {
    pub fn new(name: impl Into<String>, ver: Arc<dyn Verifier>) -> Self {
        Self { ver, name: name.into() }
    }
}
#[async_trait]
impl Judge for VerifierJudge {
    async fn assess(&self, task: &Value, candidates: &[Value]) -> Vec<f64> {
        candidates.iter().map(|c| self.ver.score(task, c)).collect()
    }
    fn name(&self) -> &str {
        &self.name
    }
    /// **ADR-019 v2** : si le vérificateur sous-jacent est un ORACLE DE CORRECTION, on expose son
    /// `passes()` ABSOLU par candidat (le PASS jeté par `assess`, qui ne garde que le score relatif).
    /// Sinon `None` (un vérificateur de FORME — non-vide, est-objet, longueur — n'atteste pas la
    /// justesse). C'est ce verdict que le consensus gate pour « vérification = acceptation ».
    fn correctness_verdict(&self, task: &Value, candidates: &[Value]) -> Option<Vec<bool>> {
        if self.ver.is_correctness_oracle() {
            Some(candidates.iter().map(|c| self.ver.passes(task, c)).collect())
        } else {
            None
        }
    }
    fn is_correctness_oracle(&self) -> bool {
        self.ver.is_correctness_oracle()
    }
}

/// Préfère les candidats dont la longueur de texte est proche d'`ideal`.
pub struct LengthBandJudge {
    pub ideal: usize,
}
#[async_trait]
impl Judge for LengthBandJudge {
    async fn assess(&self, _task: &Value, candidates: &[Value]) -> Vec<f64> {
        candidates
            .iter()
            .map(|c| {
                let n = text_of(c).chars().count() as f64;
                1.0 / (1.0 + (n - self.ideal as f64).abs())
            })
            .collect()
    }
    fn name(&self) -> &str {
        "length_band"
    }
}

/// Pertinence : fraction des mots du candidat présents dans la tâche.
pub struct KeywordOverlapJudge;
#[async_trait]
impl Judge for KeywordOverlapJudge {
    async fn assess(&self, task: &Value, candidates: &[Value]) -> Vec<f64> {
        let tw = words(&text_of(task));
        candidates
            .iter()
            .map(|c| {
                let cw = words(&text_of(c));
                if cw.is_empty() {
                    0.0
                } else {
                    cw.iter().filter(|w| tw.contains(*w)).count() as f64 / cw.len() as f64
                }
            })
            .collect()
    }
    fn name(&self) -> &str {
        "keyword_overlap"
    }
}

/// Ancrage (anti-hallucination) : fraction des mots du candidat présents dans un
/// champ-source de la tâche.
pub struct GroundednessJudge {
    pub source_field: String,
}
#[async_trait]
impl Judge for GroundednessJudge {
    async fn assess(&self, task: &Value, candidates: &[Value]) -> Vec<f64> {
        let src_val = task.get(&self.source_field).cloned().unwrap_or(Value::Null);
        let src = words(&text_of(&src_val));
        candidates
            .iter()
            .map(|c| {
                let cw = words(&text_of(c));
                if cw.is_empty() {
                    0.0
                } else {
                    cw.iter().filter(|w| src.contains(*w)).count() as f64 / cw.len() as f64
                }
            })
            .collect()
    }
    fn name(&self) -> &str {
        "groundedness"
    }
}

/// Self-consistency COMME juge : score = # candidats identiques (support d'accord).
pub struct ConsistencyJudge;
#[async_trait]
impl Judge for ConsistencyJudge {
    async fn assess(&self, _task: &Value, candidates: &[Value]) -> Vec<f64> {
        candidates
            .iter()
            .map(|c| candidates.iter().filter(|&x| x == c).count() as f64)
            .collect()
    }
    fn name(&self) -> &str {
        "consistency"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::NonEmptyText;
    use serde_json::json;

    #[tokio::test]
    async fn verifier_judge_wraps_a_catalog_verifier() {
        let j = VerifierJudge::new("non_empty", Arc::new(NonEmptyText));
        let scores = j.assess(&json!({}), &[json!({"text":"x"}), json!({"text":""})]).await;
        assert_eq!(scores, vec![1.0, 0.0]);
        assert_eq!(j.name(), "non_empty");
    }

    #[tokio::test]
    async fn length_band_prefers_ideal_length() {
        let j = LengthBandJudge { ideal: 3 };
        // "abc" (3) > "abcdef" (6).
        let s = j.assess(&json!({}), &[json!({"text":"abc"}), json!({"text":"abcdef"})]).await;
        assert!(s[0] > s[1]);
        assert_eq!(s[0], 1.0); // exactement à l'idéal
    }

    #[tokio::test]
    async fn keyword_overlap_rewards_relevance() {
        let task = json!({ "text": "ai software code" });
        let j = KeywordOverlapJudge;
        let s = j.assess(&task, &[json!({"text":"ai code"}), json!({"text":"banana"})]).await;
        assert!(s[0] > s[1]);
        assert_eq!(s[0], 1.0); // tous les mots du candidat sont dans la tâche
        assert_eq!(s[1], 0.0);
    }

    #[tokio::test]
    async fn groundedness_checks_source_overlap() {
        let task = json!({ "source": "paris is the capital of france" });
        let j = GroundednessJudge { source_field: "source".into() };
        let s = j.assess(&task, &[json!({"text":"paris france"}), json!({"text":"berlin germany"})]).await;
        assert_eq!(s[0], 1.0);
        assert_eq!(s[1], 0.0);
    }

    #[tokio::test]
    async fn consistency_counts_identical_candidates() {
        let j = ConsistencyJudge;
        let s = j.assess(&json!({}), &[json!("A"), json!("A"), json!("B")]).await;
        assert_eq!(s, vec![2.0, 2.0, 1.0]);
    }
}
