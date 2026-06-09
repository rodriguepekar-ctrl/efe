// SPDX-License-Identifier: UNLICENSED
//! Décorateurs SDK **unifiés** : budget (facture) + provenance (capture) sur des agents
//! type-effacés (`Arc<dyn Trait>`) issus de la flotte.
//!
//! Pour chaque appel : on **facture** le budget ; si débité, on exécute l'agent et on
//! **capture** l'appel (entrée→sortie) dans le `ProvenanceRecorder` ; sinon on renvoie un
//! **sentinel neutre** (la topologie dégrade sans dépenser). On câble aussi le sink de
//! provenance via `SwarmContext` (Phase 4) — donc les topologies à blackboard (T2,
//! T-Consensus) sont en plus **ancrées** (seam A).

use std::sync::Arc;

use async_trait::async_trait;
use efe_nano_sdk::AgentId;
use efe_swarm_provenance::{AgentKind, AgentRole, CallCapture, ProvenanceRecorder};
use efe_swarm_topology::{Decomposer, Generator, Judge, Merger, Solver, Stage};
use serde_json::{json, Value};

use crate::budget::BudgetGuard;

/// Contexte partagé par les décorateurs d'un run.
#[derive(Clone)]
pub(crate) struct Meter {
    pub rec: Arc<ProvenanceRecorder>,
    pub guard: Arc<BudgetGuard>,
    pub kind: AgentKind,
}

impl Meter {
    fn charge(&self, id: &AgentId) -> bool {
        self.guard.charge(id.as_str())
    }
    fn capture(&self, id: &AgentId, role: AgentRole, variant: Option<u64>, input: &Value, output: &Value) {
        let mut cap = CallCapture::new(id.clone(), role, self.kind, input, output);
        if let Some(v) = variant {
            cap = cap.with_variant(v);
        }
        self.rec.capture(cap);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Générateur
// ─────────────────────────────────────────────────────────────────────────────
struct MeteredGen {
    inner: Arc<dyn Generator>,
    meter: Meter,
    id: AgentId,
}
#[async_trait]
impl Generator for MeteredGen {
    async fn generate(&self, task: &Value, variant: usize) -> Value {
        if !self.meter.charge(&self.id) {
            return Value::Null; // budget épuisé → sentinel
        }
        let out = self.inner.generate(task, variant).await;
        self.meter.capture(&self.id, AgentRole::Generator, Some(variant as u64), task, &out);
        out
    }
}
pub(crate) fn meter_gen(inner: Arc<dyn Generator>, meter: Meter, id: AgentId) -> Arc<dyn Generator> {
    Arc::new(MeteredGen { inner, meter, id })
}

// ─────────────────────────────────────────────────────────────────────────────
//  Étage (T2)
// ─────────────────────────────────────────────────────────────────────────────
struct MeteredStage {
    inner: Arc<dyn Stage>,
    meter: Meter,
    id: AgentId,
}
#[async_trait]
impl Stage for MeteredStage {
    async fn process(&self, input: &Value) -> Vec<Value> {
        if !self.meter.charge(&self.id) {
            return vec![input.clone()]; // pass-through (préserve le flux sans dépenser)
        }
        let out = self.inner.process(input).await;
        self.meter.capture(&self.id, AgentRole::Stage, None, input, &json!(out));
        out
    }
}
pub(crate) fn meter_stage(inner: Arc<dyn Stage>, meter: Meter, id: AgentId) -> Arc<dyn Stage> {
    Arc::new(MeteredStage { inner, meter, id })
}

// ─────────────────────────────────────────────────────────────────────────────
//  Décomposeur (T3)
// ─────────────────────────────────────────────────────────────────────────────
struct MeteredDecomposer {
    inner: Arc<dyn Decomposer>,
    meter: Meter,
    id: AgentId,
}
#[async_trait]
impl Decomposer for MeteredDecomposer {
    async fn decompose(&self, task: &Value) -> Vec<Value> {
        if !self.meter.charge(&self.id) {
            return vec![task.clone()]; // pas de décomposition
        }
        let out = self.inner.decompose(task).await;
        self.meter.capture(&self.id, AgentRole::Decomposer, None, task, &json!(out));
        out
    }
}
pub(crate) fn meter_decomposer(inner: Arc<dyn Decomposer>, meter: Meter, id: AgentId) -> Arc<dyn Decomposer> {
    Arc::new(MeteredDecomposer { inner, meter, id })
}

// ─────────────────────────────────────────────────────────────────────────────
//  Solveur (T3) — la flotte n'a PAS de rôle solver → on adapte une STAGE
// ─────────────────────────────────────────────────────────────────────────────
/// Adapte un [`Stage`] (1→N) en [`Solver`] (1→1) : prend la 1ʳᵉ sortie.
pub(crate) struct StageSolver(pub Arc<dyn Stage>);
#[async_trait]
impl Solver for StageSolver {
    async fn solve(&self, subtask: &Value) -> Value {
        self.0.process(subtask).await.into_iter().next().unwrap_or(Value::Null)
    }
}

struct MeteredSolver {
    inner: Arc<dyn Solver>,
    meter: Meter,
    id: AgentId,
}
#[async_trait]
impl Solver for MeteredSolver {
    async fn solve(&self, subtask: &Value) -> Value {
        if !self.meter.charge(&self.id) {
            return Value::Null;
        }
        let out = self.inner.solve(subtask).await;
        self.meter.capture(&self.id, AgentRole::Solver, None, subtask, &out);
        out
    }
}
pub(crate) fn meter_solver(inner: Arc<dyn Solver>, meter: Meter, id: AgentId) -> Arc<dyn Solver> {
    Arc::new(MeteredSolver { inner, meter, id })
}

// ─────────────────────────────────────────────────────────────────────────────
//  Agrégateur (T3)
// ─────────────────────────────────────────────────────────────────────────────
struct MeteredMerger {
    inner: Arc<dyn Merger>,
    meter: Meter,
    id: AgentId,
}
#[async_trait]
impl Merger for MeteredMerger {
    async fn merge(&self, task: &Value, results: Vec<Value>) -> Value {
        if !self.meter.charge(&self.id) {
            return Value::Null;
        }
        let input = json!({ "task": task, "results": results });
        let out = self.inner.merge(task, results).await;
        self.meter.capture(&self.id, AgentRole::Merger, None, &input, &out);
        out
    }
}
pub(crate) fn meter_merger(inner: Arc<dyn Merger>, meter: Meter, id: AgentId) -> Arc<dyn Merger> {
    Arc::new(MeteredMerger { inner, meter, id })
}

// ─────────────────────────────────────────────────────────────────────────────
//  Juge (T-Consensus)
// ─────────────────────────────────────────────────────────────────────────────
struct MeteredJudge {
    inner: Arc<dyn Judge>,
    meter: Meter,
    id: AgentId,
}
#[async_trait]
impl Judge for MeteredJudge {
    async fn assess(&self, task: &Value, candidates: &[Value]) -> Vec<f64> {
        if !self.meter.charge(&self.id) {
            return vec![0.0; candidates.len()]; // ballot neutre
        }
        let scores = self.inner.assess(task, candidates).await;
        let input = json!({ "task": task, "candidates": candidates });
        self.meter.capture(&self.id, AgentRole::Judge, None, &input, &json!(scores));
        scores
    }
    fn name(&self) -> &str {
        self.inner.name()
    }
}
pub(crate) fn meter_judge(inner: Arc<dyn Judge>, meter: Meter, id: AgentId) -> Arc<dyn Judge> {
    Arc::new(MeteredJudge { inner, meter, id })
}
