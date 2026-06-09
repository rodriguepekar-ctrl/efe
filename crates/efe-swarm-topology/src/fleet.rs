// SPDX-License-Identifier: UNLICENSED
//! **Phase 2 — `AgentFleet`** : registry interrogeable d'agents par rôle.
//!
//! Expose le catalogue Mode A (et tout agent custom) comme une flotte nommée,
//! consultable par les topologies via le [`crate::topology::SwarmContext`].

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::catalog;
use crate::consensus::{
    ConsistencyJudge, GroundednessJudge, Judge, KeywordOverlapJudge, LengthBandJudge, VerifierJudge,
};
use crate::traits::{Decomposer, Generator, Merger, Stage, Verifier};

/// Registry d'agents par rôle (noms snake_case).
#[derive(Default)]
pub struct AgentFleet {
    generators: BTreeMap<String, Arc<dyn Generator>>,
    verifiers: BTreeMap<String, Arc<dyn Verifier>>,
    stages: BTreeMap<String, Arc<dyn Stage>>,
    decomposers: BTreeMap<String, Arc<dyn Decomposer>>,
    mergers: BTreeMap<String, Arc<dyn Merger>>,
    judges: BTreeMap<String, Arc<dyn Judge>>,
}

impl std::fmt::Debug for AgentFleet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentFleet")
            .field("generators", &self.generators.len())
            .field("verifiers", &self.verifiers.len())
            .field("stages", &self.stages.len())
            .field("decomposers", &self.decomposers.len())
            .field("mergers", &self.mergers.len())
            .field("judges", &self.judges.len())
            .field("total", &self.len())
            .finish()
    }
}

impl AgentFleet {
    pub fn new() -> Self {
        Self::default()
    }

    // ── enregistrement (builder chaînable) ──────────────────────────────────────
    pub fn register_generator(&mut self, name: impl Into<String>, a: Arc<dyn Generator>) -> &mut Self {
        self.generators.insert(name.into(), a);
        self
    }
    pub fn register_verifier(&mut self, name: impl Into<String>, a: Arc<dyn Verifier>) -> &mut Self {
        self.verifiers.insert(name.into(), a);
        self
    }
    pub fn register_stage(&mut self, name: impl Into<String>, a: Arc<dyn Stage>) -> &mut Self {
        self.stages.insert(name.into(), a);
        self
    }
    pub fn register_decomposer(&mut self, name: impl Into<String>, a: Arc<dyn Decomposer>) -> &mut Self {
        self.decomposers.insert(name.into(), a);
        self
    }
    pub fn register_merger(&mut self, name: impl Into<String>, a: Arc<dyn Merger>) -> &mut Self {
        self.mergers.insert(name.into(), a);
        self
    }
    pub fn register_judge(&mut self, name: impl Into<String>, a: Arc<dyn Judge>) -> &mut Self {
        self.judges.insert(name.into(), a);
        self
    }

    // ── consultation ────────────────────────────────────────────────────────────
    pub fn generator(&self, name: &str) -> Option<Arc<dyn Generator>> {
        self.generators.get(name).cloned()
    }
    pub fn verifier(&self, name: &str) -> Option<Arc<dyn Verifier>> {
        self.verifiers.get(name).cloned()
    }
    pub fn stage(&self, name: &str) -> Option<Arc<dyn Stage>> {
        self.stages.get(name).cloned()
    }
    pub fn decomposer(&self, name: &str) -> Option<Arc<dyn Decomposer>> {
        self.decomposers.get(name).cloned()
    }
    pub fn merger(&self, name: &str) -> Option<Arc<dyn Merger>> {
        self.mergers.get(name).cloned()
    }
    pub fn judge(&self, name: &str) -> Option<Arc<dyn Judge>> {
        self.judges.get(name).cloned()
    }

    /// **Assemble un PANEL de juges PAR NOM** (doctrine §3) — chaque nom présent est
    /// résolu ; les noms inconnus sont ignorés silencieusement. Les répétitions sont
    /// honorées (`["j_equals_expected"; 5]` → 5 `Arc` vers le même juge), ce qui permet
    /// à une [`crate::consensus_topology::ConsensusTopology`] d'être **entièrement**
    /// montée depuis la flotte sans construire un seul agent à la main.
    pub fn judge_panel(&self, names: &[&str]) -> Vec<Arc<dyn Judge>> {
        names.iter().filter_map(|n| self.judge(n)).collect()
    }

    /// Nombre total d'agents (tous rôles).
    pub fn len(&self) -> usize {
        self.generators.len()
            + self.verifiers.len()
            + self.stages.len()
            + self.decomposers.len()
            + self.mergers.len()
            + self.judges.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Tous les noms enregistrés (triés).
    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .generators
            .keys()
            .chain(self.verifiers.keys())
            .chain(self.stages.keys())
            .chain(self.decomposers.keys())
            .chain(self.mergers.keys())
            .chain(self.judges.keys())
            .cloned()
            .collect();
        v.sort();
        v
    }

    /// **Flotte standard** : tous les ~34 agents Mode A du catalogue.
    pub fn standard() -> Self {
        let mut f = Self::new();
        // décomposeurs (7)
        f.register_decomposer("sentence_split", Arc::new(catalog::SentenceSplit))
            .register_decomposer("line_split", Arc::new(catalog::LineSplit))
            .register_decomposer("word_split", Arc::new(catalog::WordSplit))
            .register_decomposer("paragraph_split", Arc::new(catalog::ParagraphSplit))
            .register_decomposer("char_chunk", Arc::new(catalog::CharChunk { size: 64 }))
            .register_decomposer("json_array_split", Arc::new(catalog::JsonArraySplit))
            .register_decomposer("fixed_count_split", Arc::new(catalog::FixedCountSplit { n: 4 }));
        // transformations (10)
        f.register_stage("trim", Arc::new(catalog::Trim))
            .register_stage("lowercase", Arc::new(catalog::Lowercase))
            .register_stage("uppercase", Arc::new(catalog::Uppercase))
            .register_stage("normalize_whitespace", Arc::new(catalog::NormalizeWhitespace))
            .register_stage("filter_nonempty", Arc::new(catalog::FilterNonEmpty))
            .register_stage("extract_field", Arc::new(catalog::ExtractField { field: "text".into() }))
            .register_stage("rename_field", Arc::new(catalog::RenameField { from: "value".into(), to: "text".into() }))
            .register_stage("extract_integers", Arc::new(catalog::ExtractIntegers))
            .register_stage("word_count", Arc::new(catalog::WordCount))
            .register_stage("prefix_text", Arc::new(catalog::PrefixText { prefix: String::new() }));
        // vérificateurs (8)
        f.register_verifier("non_empty_text", Arc::new(catalog::NonEmptyText))
            .register_verifier("is_object", Arc::new(catalog::IsObject))
            .register_verifier("has_field", Arc::new(catalog::HasField { field: "text".into() }))
            .register_verifier("numeric_equals", Arc::new(catalog::NumericEquals { target: 0.0 }))
            .register_verifier("length_bounded", Arc::new(catalog::LengthBounded { min: 0, max: 1_000_000 }))
            .register_verifier("keyword_present", Arc::new(catalog::KeywordPresent { keyword: "the".into() }))
            .register_verifier("required_fields", Arc::new(catalog::RequiredFields { fields: vec!["text".into()] }))
            .register_verifier("equals_expected", Arc::new(catalog::EqualsExpected));
        // agrégateurs (7)
        f.register_merger("collect_array", Arc::new(catalog::CollectArray))
            .register_merger("concat_text", Arc::new(catalog::ConcatText { sep: " ".into() }))
            .register_merger("join_lines", Arc::new(catalog::JoinLines))
            .register_merger("sum_values", Arc::new(catalog::SumValues))
            .register_merger("majority_value", Arc::new(catalog::MajorityValue))
            .register_merger("first_result", Arc::new(catalog::FirstResult))
            .register_merger("max_by_score_field", Arc::new(catalog::MaxByScoreField));
        // générateurs (2)
        f.register_generator("echo_generator", Arc::new(catalog::EchoGenerator))
            .register_generator("const_generator", Arc::new(catalog::ConstGenerator { value: serde_json::Value::Null }));
        // juges Mode A (9) — 4 critères orthogonaux + 5 vérificateurs-juges du catalogue.
        // `j_equals_expected` est le juge DISCRIMINANT (correct == attendu) : c'est lui que
        // la vitrine consensus monte PAR NOM (doctrine §3) au lieu de le bâtir à la main.
        f.register_judge("length_band", Arc::new(LengthBandJudge { ideal: 64 }))
            .register_judge("keyword_overlap", Arc::new(KeywordOverlapJudge))
            .register_judge("groundedness", Arc::new(GroundednessJudge { source_field: "source".into() }))
            .register_judge("consistency", Arc::new(ConsistencyJudge))
            .register_judge("j_non_empty", Arc::new(VerifierJudge::new("j_non_empty", Arc::new(catalog::NonEmptyText))))
            .register_judge("j_is_object", Arc::new(VerifierJudge::new("j_is_object", Arc::new(catalog::IsObject))))
            .register_judge("j_keyword", Arc::new(VerifierJudge::new("j_keyword", Arc::new(catalog::KeywordPresent { keyword: "the".into() }))))
            .register_judge("j_length", Arc::new(VerifierJudge::new("j_length", Arc::new(catalog::LengthBounded { min: 0, max: 1_000_000 }))))
            .register_judge("j_equals_expected", Arc::new(VerifierJudge::new("j_equals_expected", Arc::new(catalog::EqualsExpected))));
        f
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn standard_fleet_has_catalog_plus_judges() {
        let f = AgentFleet::standard();
        // 34 agents du catalogue + 9 juges Mode A.
        assert_eq!(f.len(), catalog::MODE_A_AGENT_COUNT + 9);
        assert_eq!(f.len(), 43);
        assert_eq!(f.names().len(), 43);
        assert!(f.judge("consistency").is_some(), "juge Mode A présent");
        assert!(f.judge("j_equals_expected").is_some(), "juge discriminant présent (§3)");
        assert!(f.stage("uppercase").is_some(), "agent catalogue présent");
    }

    #[test]
    fn judge_panel_assembles_by_name_with_repeats_and_skips_unknown() {
        let f = AgentFleet::standard();
        // 5 copies du juge discriminant, montées PAR NOM (doctrine §3).
        let panel = f.judge_panel(&["j_equals_expected"; 5]);
        assert_eq!(panel.len(), 5, "répétitions honorées");
        // un nom inconnu est ignoré silencieusement (pas de panique).
        let mixed = f.judge_panel(&["consistency", "does_not_exist", "length_band"]);
        assert_eq!(mixed.len(), 2, "noms inconnus ignorés");
    }

    #[tokio::test]
    async fn lookup_by_name_returns_agents() {
        let f = AgentFleet::standard();
        assert!(f.stage("uppercase").is_some());
        assert!(f.decomposer("word_split").is_some());
        assert!(f.merger("sum_values").is_some());
        assert!(f.verifier("equals_expected").is_some());
        assert!(f.generator("echo_generator").is_some());
        assert!(f.stage("does_not_exist").is_none());

        // un agent récupéré est fonctionnel.
        let up = f.stage("uppercase").unwrap();
        assert_eq!(up.process(&json!({"text":"hi"})).await[0]["text"], "HI");
    }

    #[test]
    fn custom_registration() {
        let mut f = AgentFleet::new();
        f.register_stage("up", Arc::new(catalog::Uppercase));
        assert_eq!(f.len(), 1);
        assert!(f.stage("up").is_some());
    }
}
