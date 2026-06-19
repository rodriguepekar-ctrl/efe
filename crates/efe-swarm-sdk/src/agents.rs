// SPDX-License-Identifier: UNLICENSED
//! Décorateurs SDK **unifiés** : budget (facture) + provenance (capture) + **pont N17
//! par-appel** sur des agents type-effacés (`Arc<dyn Trait>`) issus de la flotte.
//!
//! Pour chaque appel : on **facture** le budget ; si débité, on exécute l'agent **dans un
//! scope N17** (qui collecte chaque appel LLM Mode C), on **capture** l'appel
//! (entrée→sortie) dans le `ProvenanceRecorder` en **liant** le `llm_provenance_id` au
//! record N17 forensique ; sinon on renvoie un **sentinel neutre** (la topologie dégrade
//! sans dépenser). On câble aussi le sink de provenance via `SwarmContext` (Phase 4).

use std::future::Future;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use efe_nano_sdk::{AgentId, ProvenanceId, SwarmId, TenantId};
use efe_swarm_provenance::{AgentKind, AgentRole, CallCapture, ProvenanceRecorder};
use efe_swarm_topology::{Decomposer, Generator, Judge, Merger, Refiner, Solver, Stage};
use serde_json::{json, Value};

use crate::budget::BudgetGuard;
use crate::n17::LlmProvenanceCollector;
use crate::observer::{AgentCall, SwarmObserver};

/// Contexte partagé par les décorateurs d'un run.
#[derive(Clone)]
pub(crate) struct Meter {
    pub rec: Arc<ProvenanceRecorder>,
    pub guard: Arc<BudgetGuard>,
    pub kind: AgentKind,
    /// Observateur de cycle de vie (dashboard live, Inc.3) — `None` ⇒ aucun coût.
    pub observer: Option<Arc<dyn SwarmObserver>>,
    /// **Pont N17 par-appel** : collecteur des `LlmProvenanceRecord` (Mode C). `None` en
    /// keyless (Mode A) ⇒ aucun scope, aucun lien.
    pub n17: Option<Arc<LlmProvenanceCollector>>,
    pub swarm_id: SwarmId,
    pub tenant: TenantId,
}

impl Meter {

    /// Lit la convention `_reads` d'un input (au niveau racine, ou sous `task`
    /// pour les inputs composés comme celui du Refiner : `{task, own, peers}`).
    fn declared_reads(input: &Value) -> Vec<(String, u64)> {
        let arr = input
            .get("_reads")
            .or_else(|| input.pointer("/task/_reads"))
            .and_then(Value::as_array);
        let Some(arr) = arr else { return Vec::new() };
        arr.iter()
            .filter_map(|r| {
                Some((
                    r.get("pointer")?.as_str()?.to_string(),
                    r.get("version")?.as_u64()?,
                ))
            })
            .collect()
    }

    fn charge(&self, id: &AgentId) -> bool {
        self.guard.charge(id.as_str())
    }

    /// Exécute l'appel d'agent **dans un scope N17** (Mode C) ; renvoie la sortie + le
    /// `ProvenanceId` du **premier** appel LLM (le lien). Mode A : aucun scope, lien `None`.
    async fn run_agent<F, T>(&self, id: &AgentId, role: AgentRole, fut: F) -> (T, Option<ProvenanceId>)
    where
        F: Future<Output = T>,
    {
        match &self.n17 {
            None => (fut.await, None),
            Some(collector) => {
                let sink = Arc::new(Mutex::new(Vec::new()));
                let out = crate::n17::scoped(sink.clone(), fut).await;
                let calls = std::mem::take(&mut *sink.lock().expect("n17 sink lock"));
                let mut first = None;
                for call in calls {
                    let pid = collector.assemble(self.swarm_id, self.tenant, id, role, call);
                    if first.is_none() {
                        first = Some(pid);
                    }
                }
                (out, first)
            }
        }
    }

    #[allow(clippy::too_many_arguments)] // capture interne : paramètres explicites du record de provenance
    fn capture(
        &self,
        id: &AgentId,
        role: AgentRole,
        variant: Option<u64>,
        input: &Value,
        output: &Value,
        llm_id: Option<ProvenanceId>,
    ) {
        let mut cap = CallCapture::new(id.clone(), role, self.kind, input, output);
        if let Some(v) = variant {
            cap = cap.with_variant(v);
        }
        // **Graphe d'influence (archi itérative §4.3)** : un input peut DÉCLARER ses
        // lectures de cellules via la convention `_reads: [{pointer, version}, …]`
        // (posée par la topologie — gossip : pairs lus ; debate : candidat + contre-
        // argument). Liftées dans le record scellé, elles sont contrôlées par le
        // check DAG de `verify_swarm` (anti-version-fantôme) : l'influence devient
        // re-prouvable cellule par cellule, pas seulement par le hash d'entrée.
        for r in Self::declared_reads(input) {
            cap = cap.reading(r.0, r.1);
        }
        if let Some(lid) = llm_id {
            cap = cap.with_llm_provenance(lid); // ← le pont N17 par-appel
        }
        self.rec.capture(cap);
        // Inc.3 : notifie l'observateur (appel facturé) — sortie + coût + budget courant.
        if let Some(o) = &self.observer {
            o.on_agent_call(AgentCall {
                agent_id: id.as_str(),
                role,
                kind: self.kind,
                variant,
                output,
                cost_cents: self.guard.price_cents(),
                budget: &self.guard.snapshot(),
            });
        }
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
        let (out, llm) = self.meter.run_agent(&self.id, AgentRole::Generator, self.inner.generate(task, variant)).await;
        self.meter.capture(&self.id, AgentRole::Generator, Some(variant as u64), task, &out, llm);
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
        let (out, llm) = self.meter.run_agent(&self.id, AgentRole::Stage, self.inner.process(input)).await;
        self.meter.capture(&self.id, AgentRole::Stage, None, input, &json!(out), llm);
        out
    }
}
pub(crate) fn meter_stage(inner: Arc<dyn Stage>, meter: Meter, id: AgentId) -> Arc<dyn Stage> {
    Arc::new(MeteredStage { inner, meter, id })
}

// ─────────────────────────────────────────────────────────────────────────────
//  Refiner (T4 Gossip) — l'influence mutuelle, métrée comme tout le monde
// ─────────────────────────────────────────────────────────────────────────────
struct MeteredRefiner {
    inner: Arc<dyn Refiner>,
    meter: Meter,
    id: AgentId,
}
#[async_trait]
impl Refiner for MeteredRefiner {
    async fn refine(&self, task: &Value, own: Option<&Value>, peers: &[Value]) -> Value {
        if !self.meter.charge(&self.id) {
            // budget épuisé : on MAINTIENT sa position (sentinel conservateur — la
            // convergence continue sans cet appel, rien n'est inventé).
            return own.cloned().unwrap_or(Value::Null);
        }
        // L'input capturé COMMET l'influence : la valeur propre + les pairs lus
        // entrent dans le hash d'entrée du record (le contenu lu est prouvé).
        let input = json!({ "task": task, "own": own, "peers": peers });
        let (out, llm) =
            self.meter.run_agent(&self.id, AgentRole::Stage, self.inner.refine(task, own, peers)).await;
        self.meter.capture(&self.id, AgentRole::Stage, None, &input, &out, llm);
        out
    }
}
pub(crate) fn meter_refiner(inner: Arc<dyn Refiner>, meter: Meter, id: AgentId) -> Arc<dyn Refiner> {
    Arc::new(MeteredRefiner { inner, meter, id })
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
        let (out, llm) = self.meter.run_agent(&self.id, AgentRole::Decomposer, self.inner.decompose(task)).await;
        self.meter.capture(&self.id, AgentRole::Decomposer, None, task, &json!(out), llm);
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
        let (out, llm) = self.meter.run_agent(&self.id, AgentRole::Solver, self.inner.solve(subtask)).await;
        self.meter.capture(&self.id, AgentRole::Solver, None, subtask, &out, llm);
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
        let (out, llm) = self.meter.run_agent(&self.id, AgentRole::Merger, self.inner.merge(task, results)).await;
        self.meter.capture(&self.id, AgentRole::Merger, None, &input, &out, llm);
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
        // Un juge fait N appels LLM (un par candidat) ; le scope les collecte TOUS, le record
        // d'essaim se lie au premier (les N forensiques sont dans le collecteur).
        let (scores, llm) = self.meter.run_agent(&self.id, AgentRole::Judge, self.inner.assess(task, candidates)).await;
        let input = json!({ "task": task, "candidates": candidates });
        self.meter.capture(&self.id, AgentRole::Judge, None, &input, &json!(scores), llm);
        scores
    }
    fn name(&self) -> &str {
        self.inner.name()
    }
    /// **ADR-019 v2** : transparent au verdict de correction du juge enveloppé — sinon le consensus
    /// ne verrait jamais l'oracle (il opère sur des `MeteredJudge`). Pas de métrage : déterministe,
    /// sans appel LLM (le `passes()` du vérificateur sous-jacent).
    fn correctness_verdict(&self, task: &Value, candidates: &[Value]) -> Option<Vec<bool>> {
        self.inner.correctness_verdict(task, candidates)
    }
}
pub(crate) fn meter_judge(inner: Arc<dyn Judge>, meter: Meter, id: AgentId) -> Arc<dyn Judge> {
    Arc::new(MeteredJudge { inner, meter, id })
}
