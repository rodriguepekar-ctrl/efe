// SPDX-License-Identifier: UNLICENSED
//! `TopologySpec` — choisir un pattern (T1/T2/T3/Consensus/Auto) + ses agents **par nom**,
//! sans connaître les internals. La résolution assemble la topologie depuis la flotte et
//! **enveloppe chaque agent** (budget + provenance).

use std::sync::Arc;

use efe_swarm_topology::{
    ConsensusTopology, GenerateVerify, GvTopology, HierTopology, Hierarchical, Pattern, Pipeline,
    PipelineTopology, Topology,
};
use serde_json::Value;

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
    /// **Auto** : choisit selon la tâche (vérifiable → T1 ; sinon défaut sûr).
    Auto,
}

impl TopologySpec {
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

    /// Concrétise `Auto` selon la tâche (sinon renvoie `self`). Reflète H-SWARM : la
    /// valeur est dans la zone **vérifiable**.
    pub(crate) fn concretize(self, task: &Value) -> TopologySpec {
        match self {
            TopologySpec::Auto => {
                let verifier = if task.get("expected").is_some() { "equals_expected" } else { "non_empty_text" };
                TopologySpec::GenerateVerify { generator: "echo_generator".into(), verifier: verifier.into(), k: 3 }
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
            TopologySpec::Auto => unreachable!("Auto doit être concrétisé avant resolve"),
        }
    }
}
