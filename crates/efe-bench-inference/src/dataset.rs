// SPDX-License-Identifier: UNLICENSED
//! **B1.1 — la vérité-terrain : le dataset labellisé, et l'aveuglement des générateurs.**
//!
//! Un dataset B1 est une suite de [`BenchCase`] — `{ id, task, label, scorer }` — chargée depuis du
//! JSONL (une ligne par cas). Le `task` est l'entrée donnée à l'essaim ; le `label` est la
//! vérité-terrain ; le `scorer` ([`ScorerKind`]) nomme la manière de comparer. Ce format est
//! **délibérément distinct** des scénarios `CB-*` du loadgen gRPC (charge, pas justesse).
//!
//! **I-BENCH-BLIND** — l'invariant qui décide de l'honnêteté de toute la mesure : les générateurs ne
//! voient **jamais** le label. En Mode C, le prompt d'un générateur est bâti à partir de la tâche
//! JSON ; si la tâche contenait la réponse, l'exactitude serait *frauduleuse* (le modèle recopierait
//! une réponse soufflée). [`BenchCase::blind_task`] **retire** donc, par construction, toute clé de
//! label (`expected` / `label` / `__oracle_label__`) avant de remettre la tâche à l'essaim ; le
//! label n'est appliqué qu'**après coup**, par le scorer (cf. [`crate::runner`]).
//!
//! Le seul cas où l'oracle *interne* (consensus « vérification = acceptation », ADR-019 v2) a besoin
//! de voir le label est l'expérience anti-poison : [`BenchCase::oracle_task`] le ré-injecte sous
//! `expected`, avec le caveat documenté (sur modèles réels, cela fuiterait aux générateurs). Ce
//! chemin est **opt-in** par config (cf. [`crate::runner::BenchConfig::with_oracle_label`]) et la
//! mesure d'exactitude de tête d'affiche reste **toujours** sur le scorer post-hoc aveugle.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::scorer::ScorerKind;

/// Les clés qui portent (potentiellement) la vérité-terrain et que l'on **bannit** d'une tâche
/// remise aux générateurs (I-BENCH-BLIND). `expected` est la convention oracle du SDK ;
/// `__oracle_label__` est le canal latéral proposé par l'architecture ; `label` est le nôtre.
const LABEL_KEYS: [&str; 3] = ["expected", "__oracle_label__", "label"];

/// Un cas du dataset : une tâche, sa vérité-terrain, et comment les comparer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchCase {
    /// Identifiant stable du cas (clé de tri canonique + traçabilité dans le bundle).
    pub id: String,
    /// L'entrée donnée à l'essaim. **Ne doit pas** contenir le label ; [`blind_task`](Self::blind_task)
    /// le garantit de toute façon par construction (défense en profondeur).
    pub task: Value,
    /// La vérité-terrain (la bonne réponse). Détenue par le harnais ; jamais montrée aux générateurs.
    pub label: Value,
    /// La manière de comparer la réponse gagnante au label. Absent ⇒ [`ScorerKind::Exact`].
    #[serde(default)]
    pub scorer: ScorerKind,
}

impl BenchCase {
    /// Construit un cas (pratique pour les tests et la construction programmatique).
    pub fn new(id: impl Into<String>, task: Value, label: Value, scorer: ScorerKind) -> Self {
        Self { id: id.into(), task, label, scorer }
    }

    /// La tâche **aveugle** remise à l'essaim : `task` débarrassée de toute clé de label
    /// (I-BENCH-BLIND). Idempotent ; ne touche que les objets JSON (les autres formes sont rendues
    /// telles quelles — elles ne peuvent pas porter de clé de label).
    pub fn blind_task(&self) -> Value {
        let mut t = self.task.clone();
        if let Value::Object(map) = &mut t {
            for k in LABEL_KEYS {
                map.remove(k);
            }
        }
        t
    }

    /// La tâche pour l'**oracle interne** : la tâche aveugle **plus** `expected = label`, afin que le
    /// juge-oracle de consensus puisse trancher l'acceptation (ADR-019 v2). **Caveat** : sur modèles
    /// réels, `expected` apparaîtrait dans le prompt des générateurs (fuite) — d'où le statut
    /// opt-in. La mesure d'exactitude de tête d'affiche n'emprunte jamais ce chemin.
    pub fn oracle_task(&self) -> Value {
        let mut t = self.blind_task();
        if let Value::Object(map) = &mut t {
            map.insert("expected".to_string(), self.label.clone());
        } else {
            // Tâche non-objet : on enveloppe pour porter l'attendu sans perdre l'entrée.
            t = serde_json::json!({ "text": t, "expected": self.label.clone() });
        }
        t
    }
}

/// Un corpus labellisé.
#[derive(Debug, Clone, Default)]
pub struct Dataset {
    pub cases: Vec<BenchCase>,
}

impl Dataset {
    /// Construit un dataset depuis des cas en mémoire.
    pub fn new(cases: Vec<BenchCase>) -> Self {
        Self { cases }
    }

    /// Charge un dataset depuis du **JSONL** : une ligne = un [`BenchCase`]. Les lignes vides et
    /// celles commençant par `#` (commentaires) sont ignorées. Erreur typée pointant la ligne fautive.
    pub fn from_jsonl(s: &str) -> Result<Self, DatasetError> {
        let mut cases = Vec::new();
        for (i, raw) in s.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let case: BenchCase = serde_json::from_str(line)
                .map_err(|e| DatasetError::Parse { line: i + 1, msg: e.to_string() })?;
            cases.push(case);
        }
        Ok(Self { cases })
    }

    pub fn len(&self) -> usize {
        self.cases.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cases.is_empty()
    }

    /// Empreinte **blake3** du dataset (cas triés par `id`, sérialisation canonique). Lie les labels
    /// dans le bundle re-prouvable (I-BENCH-REPROVE) : un label retouché change l'empreinte.
    pub fn digest(&self) -> String {
        let mut cases: Vec<&BenchCase> = self.cases.iter().collect();
        cases.sort_by(|a, b| a.id.cmp(&b.id));
        let mut hasher = blake3::Hasher::new();
        for c in cases {
            // serde_json::Value sérialise ses objets en ordre de clés trié (BTreeMap) → déterministe.
            let line = serde_json::to_string(c).unwrap_or_default();
            hasher.update(line.as_bytes());
            hasher.update(b"\n");
        }
        hasher.finalize().to_hex().to_string()
    }
}

/// Erreur de chargement de dataset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatasetError {
    /// Une ligne JSONL n'a pas pu être désérialisée.
    Parse { line: usize, msg: String },
}

impl fmt::Display for DatasetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DatasetError::Parse { line, msg } => write!(f, "dataset JSONL ligne {line} : {msg}"),
        }
    }
}

impl std::error::Error for DatasetError {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn blind_task_strips_every_label_key() {
        // I-BENCH-BLIND : même si la tâche contient accidentellement un label, il est retiré.
        let case = BenchCase::new(
            "c1",
            json!({ "text": "the central bank raised rates", "expected": "finance", "label": "x", "__oracle_label__": "y" }),
            json!("finance"),
            ScorerKind::Exact,
        );
        let blind = case.blind_task();
        assert_eq!(blind, json!({ "text": "the central bank raised rates" }), "aucune clé de label ne survit");
        for k in LABEL_KEYS {
            assert!(blind.get(k).is_none(), "{k} retiré");
        }
    }

    #[test]
    fn oracle_task_reinjects_expected_only_on_the_opt_in_path() {
        let case = BenchCase::new("c1", json!({ "text": "t" }), json!("finance"), ScorerKind::Exact);
        // Aveugle : pas d'attendu.
        assert!(case.blind_task().get("expected").is_none());
        // Oracle (opt-in) : attendu présent pour le juge-oracle interne.
        assert_eq!(case.oracle_task(), json!({ "text": "t", "expected": "finance" }));
    }

    #[test]
    fn from_jsonl_parses_and_skips_blanks_and_comments() {
        let jsonl = "\
# un commentaire
{\"id\":\"a\",\"task\":{\"text\":\"x\"},\"label\":\"finance\"}

{\"id\":\"b\",\"task\":{\"text\":\"y\"},\"label\":42,\"scorer\":{\"numeric\":{\"tolerance\":0.5}}}
";
        let ds = Dataset::from_jsonl(jsonl).expect("parse");
        assert_eq!(ds.len(), 2);
        assert_eq!(ds.cases[0].id, "a");
        assert_eq!(ds.cases[0].scorer, ScorerKind::Exact, "scorer absent → Exact (défaut)");
        assert_eq!(ds.cases[1].scorer, ScorerKind::Numeric { tolerance: 0.5 });
    }

    #[test]
    fn from_jsonl_reports_the_faulty_line() {
        let jsonl = "{\"id\":\"a\",\"task\":{},\"label\":\"x\"}\nNOT JSON";
        let err = Dataset::from_jsonl(jsonl).expect_err("doit échouer");
        match err {
            DatasetError::Parse { line, .. } => assert_eq!(line, 2, "pointe la 2e ligne"),
        }
    }

    #[test]
    fn starter_corpora_cover_the_three_v1_scorers() {
        // Archi §12 B1.1 : classification (exact) + arithmétique (numeric) + extraction (set).
        let topics = Dataset::from_jsonl(include_str!("../fixtures/topics.jsonl")).expect("topics");
        assert!(!topics.is_empty() && topics.cases.iter().all(|c| c.scorer == ScorerKind::Exact));
        let arith = Dataset::from_jsonl(include_str!("../fixtures/arithmetic.jsonl")).expect("arith");
        assert!(!arith.is_empty() && arith.cases.iter().all(|c| matches!(c.scorer, ScorerKind::Numeric { .. })));
        let extr = Dataset::from_jsonl(include_str!("../fixtures/extraction.jsonl")).expect("extraction");
        assert!(!extr.is_empty() && extr.cases.iter().all(|c| c.scorer == ScorerKind::Set));
        // Et aucun label ne fuite dans la tâche aveugle (I-BENCH-BLIND), tous corpus confondus.
        for ds in [&topics, &arith, &extr] {
            for c in &ds.cases {
                let blind = c.blind_task();
                assert!(blind.get("expected").is_none() && blind.get("label").is_none());
            }
        }
    }

    #[test]
    fn digest_is_order_independent_and_label_sensitive() {
        let a = BenchCase::new("a", json!({ "text": "x" }), json!("finance"), ScorerKind::Exact);
        let b = BenchCase::new("b", json!({ "text": "y" }), json!("sport"), ScorerKind::Exact);
        let d1 = Dataset::new(vec![a.clone(), b.clone()]).digest();
        let d2 = Dataset::new(vec![b.clone(), a.clone()]).digest(); // ordre inversé
        assert_eq!(d1, d2, "tri par id → empreinte stable quel que soit l'ordre");
        // Un label modifié change l'empreinte (les labels sont liés).
        let b_tampered = BenchCase::new("b", json!({ "text": "y" }), json!("MUSIC"), ScorerKind::Exact);
        let d3 = Dataset::new(vec![a, b_tampered]).digest();
        assert_ne!(d1, d3, "label retouché → empreinte différente");
    }
}
