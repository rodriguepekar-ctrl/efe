// SPDX-License-Identifier: UNLICENSED
//! `Resolver` — résout un nom d'agent en `Arc<dyn Trait>` **selon le mode** :
//! - **flotte** (Mode A, keyless, déterministe) si pas de backend ;
//! - **agents LLM** (Mode C, self-host) si un backend est présent (le nom = instruction).
//!
//! Chaque agent résolu est **enveloppé** (budget + provenance). Le **vérificateur** (T1)
//! reste toujours déterministe (a-posteriori, ADR-002) — depuis la flotte.

use std::sync::Arc;

use efe_nano_sdk::AgentId;
use efe_swarm_topology::{AgentFleet, Decomposer, Generator, Judge, Merger, Solver, Stage, Verifier};

use crate::agents::{
    meter_decomposer, meter_gen, meter_judge, meter_merger, meter_solver, meter_stage, Meter,
    StageSolver,
};
use crate::llm::{llm_decomposer, llm_gen, llm_judge, llm_merger, llm_solver, llm_stage, OllamaBackend};
use crate::result::SwarmError;

pub(crate) struct Resolver<'a> {
    pub fleet: &'a AgentFleet,
    /// `Some` ⇒ self-host : on construit des agents LLM (nom = instruction).
    pub backend: Option<Arc<OllamaBackend>>,
    pub meter: Meter,
}

fn aid(s: &str) -> AgentId {
    AgentId::new(s).expect("agent id SDK valide")
}

impl Resolver<'_> {
    pub fn generator(&self, name: &str, id: &str) -> Result<Arc<dyn Generator>, SwarmError> {
        let g = match &self.backend {
            Some(b) => llm_gen(b.clone(), name.to_string()),
            None => self.fleet.generator(name).ok_or_else(|| SwarmError::UnknownAgent { role: "generator", name: name.to_string() })?,
        };
        Ok(meter_gen(g, self.meter.clone(), aid(id)))
    }

    /// Vérificateur : **toujours déterministe** (flotte), même en self-host.
    pub fn verifier(&self, name: &str) -> Result<Arc<dyn Verifier>, SwarmError> {
        self.fleet.verifier(name).ok_or_else(|| SwarmError::UnknownAgent { role: "verifier", name: name.to_string() })
    }

    pub fn stage(&self, name: &str, id: &str) -> Result<Arc<dyn Stage>, SwarmError> {
        let s = match &self.backend {
            Some(b) => llm_stage(b.clone(), name.to_string()),
            None => self.fleet.stage(name).ok_or_else(|| SwarmError::UnknownAgent { role: "stage", name: name.to_string() })?,
        };
        Ok(meter_stage(s, self.meter.clone(), aid(id)))
    }

    pub fn decomposer(&self, name: &str, id: &str) -> Result<Arc<dyn Decomposer>, SwarmError> {
        let d = match &self.backend {
            Some(b) => llm_decomposer(b.clone(), name.to_string()),
            None => self.fleet.decomposer(name).ok_or_else(|| SwarmError::UnknownAgent { role: "decomposer", name: name.to_string() })?,
        };
        Ok(meter_decomposer(d, self.meter.clone(), aid(id)))
    }

    pub fn solver(&self, name: &str, id: &str) -> Result<Arc<dyn Solver>, SwarmError> {
        // pas de rôle solver dans la flotte → on adapte une STAGE (Mode A) ; en self-host,
        // un solveur LLM direct.
        let s: Arc<dyn Solver> = match &self.backend {
            Some(b) => llm_solver(b.clone(), name.to_string()),
            None => Arc::new(StageSolver(self.fleet.stage(name).ok_or_else(|| SwarmError::UnknownAgent { role: "solver(stage)", name: name.to_string() })?)),
        };
        Ok(meter_solver(s, self.meter.clone(), aid(id)))
    }

    pub fn merger(&self, name: &str, id: &str) -> Result<Arc<dyn Merger>, SwarmError> {
        let m = match &self.backend {
            Some(b) => llm_merger(b.clone(), name.to_string()),
            None => self.fleet.merger(name).ok_or_else(|| SwarmError::UnknownAgent { role: "merger", name: name.to_string() })?,
        };
        Ok(meter_merger(m, self.meter.clone(), aid(id)))
    }

    pub fn judge(&self, name: &str, id: &str) -> Result<Arc<dyn Judge>, SwarmError> {
        let j = match &self.backend {
            Some(b) => llm_judge(b.clone(), name.to_string()),
            None => self.fleet.judge(name).ok_or_else(|| SwarmError::UnknownAgent { role: "judge", name: name.to_string() })?,
        };
        Ok(meter_judge(j, self.meter.clone(), aid(id)))
    }
}
