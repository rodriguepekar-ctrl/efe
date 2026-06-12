// SPDX-License-Identifier: UNLICENSED
//! `TopologySpec` — choisir un pattern (T1/T2/T3/Consensus/Auto) + ses agents **par nom**,
//! sans connaître les internals. La résolution assemble la topologie depuis la flotte et
//! **enveloppe chaque agent** (budget + provenance).

use std::sync::Arc;

use efe_swarm_topology::{
    ConsensusTopology, ConvergenceMetric, DebateTopology, GenerateVerify, GossipTopology,
    GvTopology, HierTopology, Hierarchical, Pattern, Pipeline, PipelineTopology, Topology,
};
use serde_json::Value;

use crate::client::ClientMode;
use crate::resolver::Resolver;
use crate::result::SwarmError;

/// Spécification d'une topologie + ses agents (résolus depuis la flotte).
#[derive(Debug, Clone)]
pub enum TopologySpec {
    /// **T1 — Generate-Verify** : K générateurs + vérificateur a-posteriori (VÉRIFIABLE).
    GenerateVerify { generator: String, verifier: String, k: usize },
    /// **T2 — Pipeline** : étages nommés chaînés.
    Pipeline { stages: Vec<String> },
    /// **T3 — Hiérarchique** : décompose → résout (via une *stage*) → fusionne.
    Hierarchical { decomposer: String, solver: String, merger: String },
    /// **T-Consensus** : K générateurs + panel de juges → Vote/Borda/Condorcet.
    Consensus { generator: String, judges: Vec<String>, k: usize, pattern: Pattern, escalate: bool },
    /// **T4 — Gossip** (itératif, ADR-003) : K refiners × tours, arrêt à ε ou r_max.
    /// L'influence mutuelle — chaque agent lit les pairs du tour précédent et révise.
    Gossip { refiner: String, k: usize, max_rounds: usize, epsilon: f64, metric: ConvergenceMetric },
    /// **Debate** (itératif, ADR-005) : consensus à DOSSIER contradictoire — PRO/CONTRA
    /// argumentent sur r tours, le jury note sur dossier, l'agrégation est inchangée.
    /// `rounds = 0` dégrade en consensus pur (la borne basse budgétaire).
    Debate {
        generator: String,
        judges: Vec<String>,
        pro: String,
        contra: String,
        k: usize,
        pattern: Pattern,
        rounds: usize,
        /// Escalade à trois étages (Vote → jury → débat) — le débat n'est payé que
        /// sur les instances où l'essaim, déjà escaladé, hésite encore.
        escalate: bool,
    },
    /// **Auto** : choisit selon la tâche (vérifiable → T1 ; sinon défaut sûr).
    /// Les topologies ITÉRATIVES ne sont JAMAIS auto-sélectionnées (opt-in explicite).
    Auto,
}

impl TopologySpec {
    /// Raccourci : un gossip Variance standard (mean_pull-compatible).
    pub fn gossip(refiner: &str, k: usize, max_rounds: usize, epsilon: f64) -> Self {
        TopologySpec::Gossip {
            refiner: refiner.into(),
            k,
            max_rounds,
            epsilon,
            metric: ConvergenceMetric::Variance,
        }
    }

    /// Raccourci : un débat Borda ESCALADÉ (Vote → jury → débat) avec des
    /// instructions PRO/CONTRA génériques — le débat n'est payé qu'à l'hésitation.
    pub fn debate(generator: &str, judges: &[&str], k: usize, rounds: usize) -> Self {
        TopologySpec::Debate {
            generator: generator.into(),
            judges: judges.iter().map(|s| s.to_string()).collect(),
            pro: "Argue FOR the candidate: cite its strongest evidence.".into(),
            contra: "Argue AGAINST the candidate: cite its weakest point.".into(),
            k,
            pattern: Pattern::Borda,
            rounds,
            escalate: true,
        }
    }

    /// Raccourci : un consensus Borda standard.
    pub fn consensus(generator: &str, judges: &[&str], k: usize) -> Self {
        TopologySpec::Consensus {
            generator: generator.into(),
            judges: judges.iter().map(|s| s.to_string()).collect(),
            k,
            pattern: Pattern::Borda,
            escalate: false,
        }
    }

    /// Concrétise `Auto` selon la tâche **ET le mode**. Reflète H-SWARM (la valeur est
    /// dans la zone vérifiable) :
    /// - **Keyless** (Mode A déterministe) : T1 Generate-Verify avec des agents de la
    ///   flotte par nom (vérificateur déterministe a-posteriori).
    /// - **Self-host** (Mode C LLM) : un consensus N-juges robuste, avec des **instructions**
    ///   en langage naturel (les noms de flotte n'ont pas de sens comme prompts LLM).
    pub(crate) fn concretize(self, task: &Value, mode: &ClientMode) -> TopologySpec {
        match self {
            TopologySpec::Auto => {
                if matches!(mode, ClientMode::Keyless) {
                    let verifier = if task.get("expected").is_some() { "equals_expected" } else { "non_empty_text" };
                    TopologySpec::GenerateVerify { generator: "echo_generator".into(), verifier: verifier.into(), k: 3 }
                } else {
                    // Self-host : instructions LLM + consensus (les juges rendent le run robuste).
                    TopologySpec::Consensus {
                        generator: "Answer the task with a single short result, no preamble.".into(),
                        judges: vec![
                            "Judge how well the proposed answer fits the task.".into(),
                            "Assess the relevance and correctness of the answer.".into(),
                        ],
                        k: 3,
                        pattern: Pattern::Borda,
                        escalate: false,
                    }
                }
            }
            other => other,
        }
    }

    /// Estimation conservatrice du nombre d'appels d'agents (pour le pré-vol budget).
    pub(crate) fn estimated_calls(&self, assumed_subtasks: usize) -> usize {
        match self {
            TopologySpec::GenerateVerify { k, .. } => (*k).max(1),
            TopologySpec::Pipeline { stages } => stages.len().max(1),
            TopologySpec::Hierarchical { .. } => 1 + assumed_subtasks + 1,
            TopologySpec::Consensus { k, judges, .. } => (*k).max(1) + judges.len(),
            // I-IT4 : le multi-tours est compté AVANT le premier appel.
            TopologySpec::Gossip { k, max_rounds, .. } => (*k).max(1) * (*max_rounds).max(1),
            TopologySpec::Debate { k, judges, rounds, escalate, .. } => {
                // pire cas escaladé : K + jury① + 2K×r + jury② (les DEUX jurys).
                let jury_passes = if *escalate { 2 } else { 1 };
                (*k).max(1) + 2 * (*k).max(1) * rounds + judges.len() * jury_passes
            }
            TopologySpec::Auto => assumed_subtasks,
        }
    }

    /// Dégrade la spec pour tenir dans `max_cents` à `price` par appel (réduit K, rogne
    /// les juges). Les topos sans paramètre de taille sont laissées telles quelles
    /// (la politique `Stop` les bornera à l'exécution).
    pub(crate) fn degraded_to_fit(self, max_cents: u32, price: u32, _assumed: usize) -> TopologySpec {
        if price == 0 {
            return self;
        }
        let budget_calls = (max_cents / price) as usize;
        match self {
            TopologySpec::GenerateVerify { generator, verifier, k } => {
                TopologySpec::GenerateVerify { generator, verifier, k: k.min(budget_calls.max(1)) }
            }
            TopologySpec::Consensus { generator, judges, k, pattern, escalate } => {
                // réserve au moins 1 appel pour un juge (un consensus sans juge est invalide).
                let max_k = budget_calls.saturating_sub(1).max(1);
                let k = k.min(max_k);
                let remaining = budget_calls.saturating_sub(k).max(1);
                let judges: Vec<String> = judges.into_iter().take(remaining).collect();
                TopologySpec::Consensus { generator, judges, k, pattern, escalate }
            }
            // Itératif : on rogne les TOURS d'abord (perdre un tour coûte moins que
            // perdre des voix — l'enseignement S1), puis K / les juges.
            TopologySpec::Gossip { refiner, k, max_rounds, epsilon, metric } => {
                let k2 = k.min(budget_calls.max(1)).max(1);
                let r2 = (budget_calls / k2).clamp(1, max_rounds.max(1));
                TopologySpec::Gossip { refiner, k: k2, max_rounds: r2, epsilon, metric }
            }
            TopologySpec::Debate { generator, judges, pro, contra, k, pattern, rounds, escalate } => {
                let jury_passes = if escalate { 2 } else { 1 };
                let k2 = k.min(budget_calls.saturating_sub(1).max(1)).max(1);
                let after_base = budget_calls.saturating_sub(k2);
                let judges2: Vec<String> =
                    judges.into_iter().take((after_base / jury_passes).max(1)).collect();
                let left = after_base.saturating_sub(judges2.len() * jury_passes);
                let rounds2 = if k2 > 0 { (left / (2 * k2)).min(rounds) } else { 0 };
                TopologySpec::Debate { generator, judges: judges2, pro, contra, k: k2, pattern, rounds: rounds2, escalate }
            }
            other => other,
        }
    }

    /// Assemble la topologie : agents résolus **par nom** depuis la flotte, chacun
    /// enveloppé par le métrage (budget + provenance). Les `agent_id` des
    /// générateurs/juges du consensus matchent les écritures internes
    /// (`/cand/*`, `/ballot/*`) pour l'ancrage.
    pub(crate) fn resolve(self, r: &Resolver<'_>) -> Result<Box<dyn Topology>, SwarmError> {
        match self {
            TopologySpec::GenerateVerify { generator, verifier, k } => {
                let k = k.max(1);
                let gen = r.generator(&generator, "t1_generator")?;
                let ver = r.verifier(&verifier)?; // toujours déterministe (a-posteriori)
                Ok(Box::new(GvTopology { gv: GenerateVerify::new(k, k.min(8)), gen, ver }))
            }
            TopologySpec::Pipeline { stages } => {
                if stages.is_empty() {
                    return Err(SwarmError::EmptySpec("pipeline sans étage"));
                }
                let mut pipe = Pipeline::new();
                for (i, name) in stages.iter().enumerate() {
                    pipe = pipe.stage(name.clone(), r.stage(name, &format!("t2_stage_{i}"))?);
                }
                Ok(Box::new(PipelineTopology::new(pipe)))
            }
            TopologySpec::Hierarchical { decomposer, solver, merger } => {
                let dec = r.decomposer(&decomposer, "t3_decomposer")?;
                let solv = r.solver(&solver, "t3_solver")?; // flotte : stage adaptée ; self-host : LLM
                let mer = r.merger(&merger, "t3_merger")?;
                Ok(Box::new(HierTopology::new(Arc::new(Hierarchical::new(dec, solv, mer)), 3.0)))
            }
            TopologySpec::Consensus { generator, judges, k, pattern, escalate } => {
                if judges.is_empty() {
                    return Err(SwarmError::EmptySpec("consensus sans juge"));
                }
                // agent_id "generator" / "judge_{i:03}" → matchent /cand/* + /ballot/* (ancrage).
                let gen = r.generator(&generator, "generator")?;
                let mut jv = Vec::with_capacity(judges.len());
                for (i, name) in judges.iter().enumerate() {
                    jv.push(r.judge(name, &format!("judge_{i:03}"))?);
                }
                let mut topo = ConsensusTopology::new(gen, k.max(1), jv).with_pattern(pattern);
                if escalate {
                    topo = topo.with_escalation(0.9, pattern);
                }
                Ok(Box::new(topo))
            }
            TopologySpec::Gossip { refiner, k, max_rounds, epsilon, metric } => {
                // agent_id "refiner" → matche les écritures /gossip/<r>/<i> (ancrage).
                let rf = r.refiner(&refiner, "refiner")?;
                Ok(Box::new(GossipTopology::new(rf, k.max(1), max_rounds.max(1), epsilon, metric)))
            }
            TopologySpec::Debate { generator, judges, pro, contra, k, pattern, rounds, escalate } => {
                if judges.is_empty() {
                    return Err(SwarmError::EmptySpec("debate sans juge"));
                }
                let gen = r.generator(&generator, "generator")?;
                let mut jv = Vec::with_capacity(judges.len());
                for (i, name) in judges.iter().enumerate() {
                    jv.push(r.judge(name, &format!("judge_{i:03}"))?);
                }
                let (p, c) = r.arguers(&pro, &contra);
                let mut topo = DebateTopology::new(gen, k.max(1), p, c, jv, pattern, rounds);
                if escalate {
                    topo = topo.with_escalation(0.9, 0.9); // parité consensus (escalade 0.9)
                }
                Ok(Box::new(topo))
            }
            TopologySpec::Auto => unreachable!("Auto doit être concrétisé avant resolve"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn auto_is_mode_aware() {
        let task = json!({ "text": "x", "expected": "x" });
        // Keyless : flotte Mode A déterministe → T1.
        match TopologySpec::Auto.concretize(&task, &ClientMode::Keyless) {
            TopologySpec::GenerateVerify { generator, verifier, .. } => {
                assert_eq!(generator, "echo_generator");
                assert_eq!(verifier, "equals_expected"); // vérifiable
            }
            other => panic!("Keyless Auto attendu T1, eu {other:?}"),
        }
        // Self-host : instructions LLM + consensus (pas de noms de flotte).
        let sh = ClientMode::SelfHost { endpoint: "x".into(), model: "m".into() };
        match TopologySpec::Auto.concretize(&task, &sh) {
            TopologySpec::Consensus { generator, judges, k, .. } => {
                assert!(generator.contains(' '), "instruction LLM, pas un nom de flotte");
                assert_eq!(judges.len(), 2);
                assert_eq!(k, 3);
            }
            other => panic!("Self-host Auto attendu Consensus, eu {other:?}"),
        }
    }

    #[test]
    fn non_auto_specs_pass_through_concretize() {
        let spec = TopologySpec::Pipeline { stages: vec!["uppercase".into()] };
        match spec.concretize(&json!({}), &ClientMode::Keyless) {
            TopologySpec::Pipeline { stages } => assert_eq!(stages, vec!["uppercase".to_string()]),
            other => panic!("attendu Pipeline inchangé, eu {other:?}"),
        }
    }
}
