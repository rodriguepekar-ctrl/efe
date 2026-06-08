// SPDX-License-Identifier: UNLICENSED
//! # `t3_hierarchical` — T3 Hiérarchique composé d'agents Mode A du catalogue.
//!
//! Map-reduce documentaire : un superviseur DÉCOMPOSE un document en phrases
//! (`catalog::SentenceSplit`), RÉSOUT chaque phrase par un mini-pipeline d'agents
//! Mode A (`Uppercase` → `WordCount`), puis AGRÈGE (`ConcatText` + total de mots).
//! Démontre aussi la RÉCURSION (superviseur de superviseurs).
//!
//! Run :  cargo run --example t3_hierarchical -p efe-swarm-topology

use std::sync::Arc;

use async_trait::async_trait;
use efe_swarm_topology::catalog::{ParagraphSplit, SentenceSplit, Uppercase, WordCount};
use efe_swarm_topology::{Decomposer, FnSolver, Hierarchical, HierSolver, Merger, Solver, Stage};
use serde_json::{json, Value};

/// Agrégateur : joint les phrases (ordonnées par idx) + somme les mots.
struct SummaryMerger;
#[async_trait]
impl Merger for SummaryMerger {
    async fn merge(&self, _task: &Value, mut results: Vec<Value>) -> Value {
        results.sort_by_key(|r| r.get("idx").and_then(|v| v.as_i64()).unwrap_or(0));
        let text = results
            .iter()
            .filter_map(|r| r.get("text").and_then(|v| v.as_str()))
            .collect::<Vec<_>>()
            .join(". ");
        let total: u64 = results
            .iter()
            .filter_map(|r| r.get("count").and_then(|v| v.as_u64()))
            .sum();
        json!({ "text": text, "total_words": total, "parts": results.len() })
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    // Solveur FEUILLE : mini-pipeline d'agents Mode A (Uppercase → WordCount).
    let leaf_solver: Arc<dyn Solver> = Arc::new(FnSolver(|subtask: Value| async move {
        let up = Uppercase.process(&subtask).await.into_iter().next().unwrap();
        WordCount.process(&up).await.into_iter().next().unwrap()
    }));

    let sentence_level = Hierarchical::new(
        Arc::new(SentenceSplit),
        leaf_solver.clone(),
        Arc::new(SummaryMerger),
    );

    println!("══════════════════════════════════════════════════════════════════");
    println!(" T3 Hiérarchique — décompose(phrases) → solve(Mode A) → agrège");
    println!("══════════════════════════════════════════════════════════════════");

    let doc = json!({ "text": "the swarm passes information. tiny agents run in parallel. the verifier selects" });
    let r = sentence_level.run(&doc).await;
    println!(" sous-tâches (phrases) : {}", r.subtasks);
    println!(" sortie agrégée       : {}", r.output["text"]);
    println!(" total de mots         : {}", r.output["total_words"]);

    // ── Récursion : document → paragraphes, chaque paragraphe résolu par le
    //    superviseur de phrases ci-dessus (HierSolver). ───────────────────────
    let inner = Arc::new(Hierarchical::new(
        Arc::new(SentenceSplit),
        leaf_solver,
        Arc::new(SummaryMerger),
    ));
    // enveloppe pour préserver l'idx du paragraphe.
    struct ParaSolver(Arc<Hierarchical>);
    #[async_trait]
    impl Solver for ParaSolver {
        async fn solve(&self, subtask: &Value) -> Value {
            let idx = subtask.get("idx").cloned().unwrap_or(json!(0));
            let mut out = self.0.run(subtask).await.output;
            out["idx"] = idx;
            out
        }
    }
    let _ = HierSolver(inner.clone()); // (adaptateur générique dispo aussi)

    struct ParaMerger;
    #[async_trait]
    impl Merger for ParaMerger {
        async fn merge(&self, _t: &Value, mut results: Vec<Value>) -> Value {
            results.sort_by_key(|r| r.get("idx").and_then(|v| v.as_i64()).unwrap_or(0));
            let total: u64 = results.iter().filter_map(|r| r.get("total_words").and_then(|v| v.as_u64())).sum();
            json!({ "paragraphs": results.len(), "grand_total_words": total })
        }
    }

    let top = Hierarchical::new(Arc::new(ParagraphSplit), Arc::new(ParaSolver(inner)), Arc::new(ParaMerger));
    let multi = json!({ "text": "first para sentence one. para one sentence two\n\nsecond paragraph here. another sentence" });
    let rt = top.run(&multi).await;
    println!("\n── Récursion (paragraphes → phrases → Mode A) ──");
    println!(" paragraphes          : {}", rt.subtasks);
    println!(" total de mots (récur) : {}", rt.output["grand_total_words"]);
    println!("══════════════════════════════════════════════════════════════════");
    println!("→ T3 superviseur + ~34 agents Mode A composables (catalogue). Récursif.");

    // évite l'avertissement de décomposeur inutilisé directement.
    let _ = SentenceSplit.decompose(&json!({"text":"x"})).await;
}
