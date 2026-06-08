// SPDX-License-Identifier: UNLICENSED
//! **T3 — Hiérarchique (superviseurs)** : un superviseur DÉCOMPOSE une tâche en
//! sous-tâches, les RÉSOUT en parallèle (sur le substrat, via un T2 mono-étage),
//! puis AGRÈGE. **Récursif** : un `Solver` peut être un T1, un T2, ou un autre T3
//! (verify-of-verify, superviseur-de-superviseurs).
//!
//! Sert à ramener une tâche DURE (hors enveloppe de capacité, cf. F2 de Lot S2)
//! dans l'enveloppe : on la découpe en sous-tâches cheap résolubles par T1.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use crate::generate_verify::GenerateVerify;
use crate::pipeline::Pipeline;
use crate::traits::{Decomposer, Generator, Merger, Solver, Stage, Verifier};

/// Résultat d'un run [`Hierarchical`].
#[derive(Debug, Clone)]
pub struct HierResult {
    pub output: Value,
    /// Nombre de sous-tâches décomposées.
    pub subtasks: usize,
}

/// **T3** : superviseur = décomposeur + solveur (récursif) + agrégateur.
pub struct Hierarchical {
    decomposer: Arc<dyn Decomposer>,
    solver: Arc<dyn Solver>,
    merger: Arc<dyn Merger>,
    max_concurrency: usize,
    timeout: Duration,
}

impl std::fmt::Debug for Hierarchical {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hierarchical")
            .field("max_concurrency", &self.max_concurrency)
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl Hierarchical {
    pub fn new(
        decomposer: Arc<dyn Decomposer>,
        solver: Arc<dyn Solver>,
        merger: Arc<dyn Merger>,
    ) -> Self {
        Self {
            decomposer,
            solver,
            merger,
            max_concurrency: 16,
            timeout: Duration::from_secs(30),
        }
    }

    pub fn with_concurrency(mut self, c: usize) -> Self {
        self.max_concurrency = c.max(1);
        self
    }
    pub fn with_timeout(mut self, t: Duration) -> Self {
        self.timeout = t;
        self
    }

    /// Exécute T3 : décompose, résout les sous-tâches EN PARALLÈLE sur le substrat
    /// (T2 mono-étage → streaming + quiescence), puis agrège.
    pub async fn run(&self, task: &Value) -> HierResult {
        let subtasks = self.decomposer.decompose(task).await;
        if subtasks.is_empty() {
            let output = self.merger.merge(task, vec![]).await;
            return HierResult { output, subtasks: 0 };
        }
        let n = subtasks.len();
        // Résolution parallèle bornée, portée par le substrat (T2 mono-étage).
        let results = Pipeline::new()
            .with_concurrency(self.max_concurrency)
            .stage("solve", Arc::new(SolverStage(self.solver.clone())))
            .run_batch(subtasks, self.timeout)
            .await;
        let output = self.merger.merge(task, results).await;
        HierResult { output, subtasks: n }
    }
}

/// Adaptateur interne : expose un [`Solver`] comme un [`Stage`] (1→1) pour T2.
struct SolverStage(Arc<dyn Solver>);
#[async_trait]
impl Stage for SolverStage {
    async fn process(&self, input: &Value) -> Vec<Value> {
        vec![self.0.solve(input).await]
    }
}

// ─── Adaptateurs de Solver (composition récursive) ───────────────────────────

/// Solveur FEUILLE : résout via une fonction async pure (agent Mode A).
pub struct FnSolver<F>(pub F);
#[async_trait]
impl<F, Fut> Solver for FnSolver<F>
where
    F: Fn(Value) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Value> + Send + 'static,
{
    async fn solve(&self, subtask: &Value) -> Value {
        (self.0)(subtask.clone()).await
    }
}

/// Solveur T1 : résout chaque sous-tâche par Generate-Verify.
pub struct GvSolver {
    pub gv: GenerateVerify,
    pub gen: Arc<dyn Generator>,
    pub ver: Arc<dyn Verifier>,
}
#[async_trait]
impl Solver for GvSolver {
    async fn solve(&self, subtask: &Value) -> Value {
        self.gv
            .run(subtask, self.gen.clone(), self.ver.as_ref())
            .await
            .selected
            .unwrap_or(Value::Null)
    }
}

/// Solveur T3 récursif : résout chaque sous-tâche par un sous-superviseur.
pub struct HierSolver(pub Arc<Hierarchical>);
#[async_trait]
impl Solver for HierSolver {
    async fn solve(&self, subtask: &Value) -> Value {
        self.0.run(subtask).await.output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // mocks Mode A ────────────────────────────────────────────────────────────
    struct WordsDecomp;
    #[async_trait]
    impl Decomposer for WordsDecomp {
        async fn decompose(&self, task: &Value) -> Vec<Value> {
            let text = task.get("text").and_then(|v| v.as_str()).unwrap_or("");
            text.split_whitespace()
                .enumerate()
                .map(|(i, w)| json!({ "text": w, "idx": i }))
                .collect()
        }
    }
    struct UpperSolver;
    #[async_trait]
    impl Solver for UpperSolver {
        async fn solve(&self, subtask: &Value) -> Value {
            let t = subtask.get("text").and_then(|v| v.as_str()).unwrap_or("");
            json!({ "text": t.to_uppercase(), "idx": subtask.get("idx").cloned().unwrap_or(json!(0)) })
        }
    }
    struct JoinMerger;
    #[async_trait]
    impl Merger for JoinMerger {
        async fn merge(&self, _task: &Value, mut results: Vec<Value>) -> Value {
            results.sort_by_key(|r| r.get("idx").and_then(|v| v.as_i64()).unwrap_or(0));
            let joined = results
                .iter()
                .filter_map(|r| r.get("text").and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
                .join(" ");
            json!({ "text": joined })
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn t3_map_reduce_decompose_solve_merge() {
        let h = Hierarchical::new(Arc::new(WordsDecomp), Arc::new(UpperSolver), Arc::new(JoinMerger));
        let r = h.run(&json!({ "text": "hello swarm world" })).await;
        assert_eq!(r.subtasks, 3);
        assert_eq!(r.output["text"], "HELLO SWARM WORLD", "map-reduce ordonné par idx");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn t3_empty_decomposition_merges_empty() {
        let h = Hierarchical::new(Arc::new(WordsDecomp), Arc::new(UpperSolver), Arc::new(JoinMerger));
        let r = h.run(&json!({ "text": "" })).await;
        assert_eq!(r.subtasks, 0);
        assert_eq!(r.output["text"], "");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn t3_recursive_supervisor_of_supervisors() {
        // niveau bas : majuscule par mot. niveau haut : décompose par LIGNES,
        // chaque ligne résolue par le superviseur bas (récursion via HierSolver).
        struct LinesDecomp;
        #[async_trait]
        impl Decomposer for LinesDecomp {
            async fn decompose(&self, task: &Value) -> Vec<Value> {
                let text = task.get("text").and_then(|v| v.as_str()).unwrap_or("");
                text.lines()
                    .enumerate()
                    .map(|(i, l)| json!({ "text": l, "idx": i }))
                    .collect()
            }
        }
        struct LinesMerger;
        #[async_trait]
        impl Merger for LinesMerger {
            async fn merge(&self, _t: &Value, mut results: Vec<Value>) -> Value {
                results.sort_by_key(|r| r.get("idx").and_then(|v| v.as_i64()).unwrap_or(0));
                let joined = results
                    .iter()
                    .filter_map(|r| r.get("text").and_then(|v| v.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n");
                json!({ "text": joined })
            }
        }

        let inner = Arc::new(Hierarchical::new(
            Arc::new(WordsDecomp),
            Arc::new(UpperSolver),
            Arc::new(JoinMerger),
        ));
        // le solveur du niveau haut EST le superviseur bas (mais il faut préserver idx
        // de la ligne) → on enveloppe pour réinjecter l'idx.
        struct InnerWithIdx(Arc<Hierarchical>);
        #[async_trait]
        impl Solver for InnerWithIdx {
            async fn solve(&self, subtask: &Value) -> Value {
                let idx = subtask.get("idx").cloned().unwrap_or(json!(0));
                let mut out = self.0.run(subtask).await.output;
                out["idx"] = idx;
                out
            }
        }
        let top = Hierarchical::new(
            Arc::new(LinesDecomp),
            Arc::new(InnerWithIdx(inner)),
            Arc::new(LinesMerger),
        );
        let r = top.run(&json!({ "text": "hello world\nfoo bar" })).await;
        assert_eq!(r.subtasks, 2);
        assert_eq!(r.output["text"], "HELLO WORLD\nFOO BAR", "récursion T3 → T3");
    }
}
