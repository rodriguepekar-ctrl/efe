// SPDX-License-Identifier: UNLICENSED
//! `TopologySpec` — choisir un pattern (T1/T2/T3/Consensus/Auto) + ses agents **par nom**,
//! sans connaître les internals. La résolution assemble la topologie depuis la flotte et
//! **enveloppe chaque agent** (budget + provenance).

use std::sync::Arc;

use efe_swarm_topology::{
    AgentFleet, ConsensusTopology, GenerateVerify, GvTopology, HierTopology, Hierarchical, Pattern,
    Pipeline, PipelineTopology, Topology,
};
use efe_nano_sdk::AgentId;
use serde_json::Value;

use crate::agents::{
    meter_decomposer, meter_gen, meter_judge, meter_merger, meter_solver, meter_stage, Meter,
    StageSolver,
};
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
    pub(crate) fn resolve(self, fleet: &AgentFleet, meter: &Meter) -> Result<Box<dyn Topology>, SwarmError> {
        match self {
            TopologySpec::GenerateVerify { generator, verifier, k } => {
                let g = fleet.generator(&generator).ok_or(SwarmError::UnknownAgent { role: "generator", name: generator })?;
                let v = fleet.verifier(&verifier).ok_or(SwarmError::UnknownAgent { role: "verifier", name: verifier })?;
                let k = k.max(1);
                let gen = meter_gen(g, meter.clone(), aid("t1_generator"));
                Ok(Box::new(GvTopology { gv: GenerateVerify::new(k, k.min(8)), gen, ver: v }))
            }
            TopologySpec::Pipeline { stages } => {
                if stages.is_empty() {
                    return Err(SwarmError::EmptySpec("pipeline sans étage"));
                }
                let mut pipe = Pipeline::new();
                for (i, name) in stages.iter().enumerate() {
                    let s = fleet.stage(name).ok_or(SwarmError::UnknownAgent { role: "stage", name: name.clone() })?;
                    pipe = pipe.stage(name.clone(), meter_stage(s, meter.clone(), aid(&format!("t2_stage_{i}"))));
                }
                Ok(Box::new(PipelineTopology::new(pipe)))
            }
            TopologySpec::Hierarchical { decomposer, solver, merger } => {
                let d = fleet.decomposer(&decomposer).ok_or(SwarmError::UnknownAgent { role: "decomposer", name: decomposer })?;
                // pas de rôle solver dans la flotte → on adapte une STAGE en solver.
                let s = fleet.stage(&solver).ok_or(SwarmError::UnknownAgent { role: "solver(stage)", name: solver })?;
                let m = fleet.merger(&merger).ok_or(SwarmError::UnknownAgent { role: "merger", name: merger })?;
                let dec = meter_decomposer(d, meter.clone(), aid("t3_decomposer"));
                let solv = meter_solver(Arc::new(StageSolver(s)), meter.clone(), aid("t3_solver"));
                let mer = meter_merger(m, meter.clone(), aid("t3_merger"));
                Ok(Box::new(HierTopology::new(Arc::new(Hierarchical::new(dec, solv, mer)), 3.0)))
            }
            TopologySpec::Consensus { generator, judges, k, pattern, escalate } => {
                if judges.is_empty() {
                    return Err(SwarmError::EmptySpec("consensus sans juge"));
                }
                let g = fleet.generator(&generator).ok_or(SwarmError::UnknownAgent { role: "generator", name: generator })?;
                // agent_id "generator" → matche /cand/* écrit par ConsensusTopology (ancrage).
                let gen = meter_gen(g, meter.clone(), aid("generator"));
                let mut jv = Vec::with_capacity(judges.len());
                for (i, name) in judges.iter().enumerate() {
                    let j = fleet.judge(name).ok_or(SwarmError::UnknownAgent { role: "judge", name: name.clone() })?;
                    // agent_id "judge_{i:03}" → matche /ballot/{i} (ancrage).
                    jv.push(meter_judge(j, meter.clone(), aid(&format!("judge_{i:03}"))));
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

fn aid(s: &str) -> AgentId {
    AgentId::new(s).expect("agent id SDK valide")
}
