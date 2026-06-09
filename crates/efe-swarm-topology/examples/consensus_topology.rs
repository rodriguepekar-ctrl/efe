// SPDX-License-Identifier: UNLICENSED
//! # `consensus_topology` — T-Consensus câblée sur le substrat (Phase 3, inc.3).
//!
//! Démontre [`ConsensusTopology`] : K générateurs → candidats au Blackboard →
//! juges-agents écrivent leurs ballots (substrat) → agrégation ADAPTATIVE
//! (Vote si accord, sinon Borda). Montre que sur une tâche à LEURRE MAJORITAIRE,
//! le consensus extrait le correct là où le vote échouerait.
//!
//! Run :  cargo run --release --example consensus_topology -p efe-swarm-topology

use std::sync::Arc;

use async_trait::async_trait;
use efe_swarm_topology::catalog::EqualsExpected;
use efe_swarm_topology::consensus::VerifierJudge;
use efe_swarm_topology::{
    agreement, ConsensusTopology, Generator, Judge, MajorityVote, Pattern, PatternSelector,
    SwarmContext, Topology, Verifier,
};
use efe_swarm_topology::Aggregator;
use serde_json::{json, Value};

/// Générateur : `n_correct` premiers variants renvoient l'attendu ; le reste un leurre.
struct CtrlGen;
#[async_trait]
impl Generator for CtrlGen {
    async fn generate(&self, task: &Value, variant: usize) -> Value {
        let n_correct = task.get("n_correct").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let expected = task.get("expected").cloned().unwrap_or(Value::Null);
        if variant < n_correct {
            json!({ "value": expected })
        } else {
            json!({ "value": "__lure__" })
        }
    }
}

fn judges(n: usize) -> Vec<Arc<dyn Judge>> {
    (0..n)
        .map(|i| {
            let v: Arc<dyn Verifier> = Arc::new(EqualsExpected);
            Arc::new(VerifierJudge::new(format!("j{i}"), v)) as Arc<dyn Judge>
        })
        .collect()
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let k = 7;
    let ctx = SwarmContext::standard();
    let topo = ConsensusTopology::new(Arc::new(CtrlGen), k, judges(5))
        .with_selector(PatternSelector::default()); // adaptatif

    println!("══════════════════════════════════════════════════════════════════════════");
    println!(" T-Consensus (substrat) — K={k} générateurs · 5 juges · agrégation ADAPTATIVE");
    println!("══════════════════════════════════════════════════════════════════════════");
    println!(" {:<28} {:>9} {:>10} {:>9} {:>9}", "tâche", "accord", "pattern", "T-cons", "vote-seul");

    // tâches : n_correct candidats corrects sur K (le reste = leurre commun).
    for (label, n_correct) in [("leurre majoritaire (3/7)", 3u64), ("accord fort (7/7)", 7), ("dispersé (1/7)", 1)] {
        let expected = json!("tech");
        let task = json!({ "expected": expected, "n_correct": n_correct });

        // T-Consensus (adaptatif, sur le substrat).
        let out = topo.run(&task, &ctx).await;
        let t_cons = out.output.get("value").cloned().unwrap_or(Value::Null);

        // reconstruit les candidats pour calculer accord + vote-seul (comparaison).
        let cands: Vec<Value> = (0..k)
            .map(|v| {
                if (v as u64) < n_correct {
                    json!({ "value": expected })
                } else {
                    json!({ "value": "__lure__" })
                }
            })
            .collect();
        let ag = agreement(&cands);
        let pattern = PatternSelector::default().select(&cands);
        let vote = MajorityVote.aggregate(&task, &cands, &[]).selected;
        let vote_val = vote.get("value").cloned().unwrap_or(Value::Null);

        let pname = match pattern {
            Pattern::Vote => "vote",
            Pattern::Borda => "borda",
            Pattern::Condorcet => "condorcet",
        };
        let ok = |v: &Value| if *v == expected { "✓ tech" } else { "✗ leurre" };
        println!(
            " {:<28} {:>9.2} {:>10} {:>9} {:>9}",
            label, ag, pname, ok(&t_cons), ok(&vote_val)
        );
    }
    println!("──────────────────────────────────────────────────────────────────────────");
    println!("→ Sur leurre majoritaire : T-Consensus (juges via le substrat) extrait le correct,");
    println!("  le vote-seul échoue. Sur accord fort : l'adaptatif choisit le vote (gratuit).");
    println!("══════════════════════════════════════════════════════════════════════════");
}
