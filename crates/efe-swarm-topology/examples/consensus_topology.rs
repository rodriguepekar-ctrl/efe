// SPDX-License-Identifier: UNLICENSED
//! # `consensus_topology` — T-Consensus câblée sur le substrat (Phase 3, inc.3 + §5).
//!
//! Démontre [`ConsensusTopology`] sous DEUX politiques d'agrégation, le panel de
//! juges étant monté **PAR NOM depuis la flotte** (doctrine §3 — aucun agent bâti à
//! la main) :
//!
//! 1. **ADAPTATIVE** (`with_selector`) : Vote si accord fort, sinon Borda. Sur une
//!    tâche à LEURRE MAJORITAIRE, le consensus extrait le correct là où le vote échoue.
//! 2. **ESCALADE par CONFIANCE** (`with_escalation`, §5) : tente le Vote (GRATUIT,
//!    0 juge) ; n'appelle les juges QUE si la confiance du vote < seuil. On voit le
//!    coût chuter à K sur l'accord fort (juges économisés) et monter à K+J quand il
//!    faut vraiment trancher.
//!
//! Run :  cargo run --release --example consensus_topology -p efe-swarm-topology

use std::sync::Arc;

use async_trait::async_trait;
use efe_swarm_topology::Aggregator;
use efe_swarm_topology::{
    agreement, ConsensusTopology, Generator, MajorityVote, Pattern, PatternSelector, SwarmContext,
    Topology,
};
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

/// Reconstruit les K candidats d'un profil (pour les baselines de comparaison).
fn rebuild(k: usize, n_correct: u64, expected: &Value) -> Vec<Value> {
    (0..k)
        .map(|v| {
            if (v as u64) < n_correct {
                json!({ "value": expected })
            } else {
                json!({ "value": "__lure__" })
            }
        })
        .collect()
}

const PROFILES: &[(&str, u64)] =
    &[("leurre majoritaire (3/7)", 3), ("accord fort (7/7)", 7), ("dispersé (1/7)", 1)];

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let k = 7;
    let ctx = SwarmContext::standard();
    let expected = json!("tech");

    // §3 : le panel de 5 juges discriminants est monté PAR NOM depuis la flotte.
    let panel = ctx.fleet.judge_panel(&["j_equals_expected"; 5]);

    println!("══════════════════════════════════════════════════════════════════════════");
    println!(" T-Consensus (substrat) — K={k} · 5 juges montés PAR NOM depuis la flotte (§3)");
    println!("══════════════════════════════════════════════════════════════════════════");

    // ── 1. AGRÉGATION ADAPTATIVE (Vote si accord, sinon Borda). ──────────────────
    let adaptive = ConsensusTopology::new(Arc::new(CtrlGen), k, panel.clone())
        .with_selector(PatternSelector::default());
    println!("\n [1] ADAPTATIVE — pattern choisi selon l'accord des candidats :");
    println!(" {:<28} {:>9} {:>10} {:>9} {:>9}", "tâche", "accord", "pattern", "T-cons", "vote-seul");
    for &(label, n_correct) in PROFILES {
        let task = json!({ "expected": expected, "n_correct": n_correct });
        let out = adaptive.run(&task, &ctx).await;
        let t_cons = out.output.get("value").cloned().unwrap_or(Value::Null);

        let cands = rebuild(k, n_correct, &expected);
        let ag = agreement(&cands);
        let pattern = PatternSelector::default().select(&cands);
        let vote_val = MajorityVote
            .aggregate(&task, &cands, &[])
            .selected
            .get("value")
            .cloned()
            .unwrap_or(Value::Null);
        let pname = match pattern {
            Pattern::Vote => "vote",
            Pattern::Borda => "borda",
            Pattern::Condorcet => "condorcet",
        };
        let ok = |v: &Value| if *v == expected { "✓ tech" } else { "✗ leurre" };
        println!(" {:<28} {:>9.2} {:>10} {:>9} {:>9}", label, ag, pname, ok(&t_cons), ok(&vote_val));
    }

    // ── 2. ESCALADE par CONFIANCE (§5) : juges appelés SEULEMENT si le vote doute. ─
    // Seuil 0,9 = doctrine « vote seul si QUASI-unanime ». Confiance vote = (max−2ᵉ)/K.
    let escal = ConsensusTopology::new(Arc::new(CtrlGen), k, panel.clone())
        .with_escalation(0.9, Pattern::Borda);
    println!("\n [2] ESCALADE par confiance (seuil 0,9 → Borda) — juges = coût, économisés si sûr :");
    println!(" {:<28} {:>11} {:>8} {:>10} {:>9}", "tâche", "conf.vote", "juges", "coût", "T-cons");
    for &(label, n_correct) in PROFILES {
        let task = json!({ "expected": expected, "n_correct": n_correct });
        let cands = rebuild(k, n_correct, &expected);
        let conf = MajorityVote.aggregate(&task, &cands, &[]).confidence;

        let out = escal.run(&task, &ctx).await;
        let t_cons = out.output.get("value").cloned().unwrap_or(Value::Null);
        let escaladed = if out.quality.subtasks == 0 { "0 (vote)" } else { "5 (escal)" };
        let ok = if t_cons == expected { "✓ tech" } else { "✗ leurre" };
        println!(
            " {:<28} {:>11.2} {:>8} {:>10.0} {:>9}",
            label, conf, escaladed, out.cost_units, ok
        );
    }

    println!("\n──────────────────────────────────────────────────────────────────────────");
    println!(" → [1] Sur leurre majoritaire, T-Consensus (juges via le substrat) extrait le");
    println!("   correct ; le vote-seul échoue. [2] L'escalade n'appelle les juges QUE sur les");
    println!("   tâches douteuses : accord fort = coût K (juges économisés), sinon coût K+J.");
    println!(" → Panel monté PAR NOM depuis la flotte (§3) — zéro agent construit à la main.");
    println!("══════════════════════════════════════════════════════════════════════════");
}
