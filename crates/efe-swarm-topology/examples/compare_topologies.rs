// SPDX-License-Identifier: UNLICENSED
//! # `compare_topologies` — la couche d'abstraction §3 en action.
//!
//! Démontre que T1/T2/T3 sont **assemblées depuis l'`AgentFleet` par NOM**, puis
//! exécutées **uniformément** via `Box<dyn Topology>` (une seule boucle), chacune
//! renvoyant le même `TopologyOutput` (sortie, coût, latence, agents, qualité).
//!
//! Run :  cargo run --release --example compare_topologies -p efe-swarm-topology

use std::sync::Arc;

use async_trait::async_trait;
use efe_swarm_topology::{
    AgentFleet, Decomposer, GenerateVerify, Generator, GvSolver, GvTopology, HierTopology,
    Hierarchical, Pipeline, PipelineTopology, SwarmContext, Topology,
};
use serde_json::{json, Value};

const N: usize = 20;
const TOPICS: &[&str] = &["technology", "finance", "health", "legal"];

fn unit(mut z: u64) -> f64 {
    z = z.wrapping_add(0x9E3779B97F4A7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

/// Générateur faillible simulé (agent custom enregistré dans la flotte).
struct SimGen {
    skill: f64,
    gamma: f64,
}
#[async_trait]
impl Generator for SimGen {
    async fn generate(&self, task: &Value, variant: usize) -> Value {
        let diff = task.get("difficulty").and_then(|v| v.as_f64()).unwrap_or(0.5);
        let id = task.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
        let expected = task.get("expected").cloned().unwrap_or(Value::Null);
        let p = (self.skill - self.gamma * diff).clamp(0.0, 1.0);
        if unit(id ^ (variant as u64).wrapping_mul(0x9E37)) < p {
            json!({ "value": expected })
        } else {
            json!({ "value": "__wrong__" })
        }
    }
}

/// Décomposeur custom : tâche → 5 sous-tâches faciles (même topic attendu).
struct DocDecomposer;
#[async_trait]
impl Decomposer for DocDecomposer {
    async fn decompose(&self, task: &Value) -> Vec<Value> {
        let id = task.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
        let expected = task.get("expected").cloned().unwrap_or(Value::Null);
        (0..5)
            .map(|i| json!({ "id": id * 100 + i, "expected": expected, "difficulty": 0.2 }))
            .collect()
    }
}

fn correct(out: &Value, expected: &Value) -> bool {
    out.get("value").map(|v| v == expected).unwrap_or(false)
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    // ── Flotte = catalogue standard + 2 agents custom (générateur + décomposeur). ──
    let mut fleet = AgentFleet::standard();
    fleet.register_generator("sim_classifier", Arc::new(SimGen { skill: 0.55, gamma: 0.6 }));
    fleet.register_decomposer("doc_split", Arc::new(DocDecomposer));
    let ctx = SwarmContext::new(Arc::new(fleet));

    println!("══════════════════════════════════════════════════════════════════════════");
    println!(" COMPARE TOPOLOGIES — assemblées depuis l'AgentFleet ({} agents) par nom,", ctx.fleet.len());
    println!(" exécutées uniformément via Box<dyn Topology>.");
    println!("══════════════════════════════════════════════════════════════════════════");

    // ── Assemblage PAR NOM depuis la flotte ──────────────────────────────────────
    let gen = ctx.fleet.generator("sim_classifier").unwrap();
    let ver = ctx.fleet.verifier("equals_expected").unwrap();
    let merger = ctx.fleet.merger("majority_value").unwrap();
    let decomposer = ctx.fleet.decomposer("doc_split").unwrap();

    // T1 = Generate-Verify.
    let t1: Box<dyn Topology> = Box::new(GvTopology {
        gv: GenerateVerify::new(7, 4),
        gen: gen.clone(),
        ver: ver.clone(),
    });
    // T3 = Hiérarchique (decompose → T1 par sous-tâche → majorité).
    let h = Arc::new(Hierarchical::new(
        decomposer,
        Arc::new(GvSolver { gv: GenerateVerify::new(3, 3), gen: gen.clone(), ver: ver.clone() }),
        merger,
    ));
    let t3: Box<dyn Topology> = Box::new(HierTopology::new(h, 3.0));

    // ── Boucle UNIFORME sur les topologies (classification : T1 vs T3) ──────────
    let topos: Vec<Box<dyn Topology>> = vec![t1, t3];
    println!("\n Classification ({N} tâches, difficulté 0.7) :");
    println!(" {:<20} {:>8} {:>10} {:>10}", "topologie", "acc", "coût/moy", "agents/moy");
    println!(" {}", "─".repeat(52));
    for topo in &topos {
        let (mut ok, mut cost, mut agents) = (0usize, 0.0f64, 0usize);
        for i in 0..N {
            let expected = json!(TOPICS[i % TOPICS.len()]);
            let task = json!({ "id": i as u64, "expected": expected, "difficulty": 0.7 });
            let out = topo.run(&task, &ctx).await; // ← interface UNIFORME
            if correct(&out.output, &expected) {
                ok += 1;
            }
            cost += out.cost_units;
            agents += out.agents_used;
        }
        println!(
            " {:<20} {:>7.0}% {:>10.1} {:>10.1}",
            topo.name(),
            100.0 * ok as f64 / N as f64,
            cost / N as f64,
            agents as f64 / N as f64
        );
    }

    // ── T2 (transform pipeline) sous la MÊME interface ──────────────────────────
    let pipe = Pipeline::new()
        .stage("normalize", ctx.fleet.stage("normalize_whitespace").unwrap())
        .stage("upper", ctx.fleet.stage("uppercase").unwrap());
    let t2 = PipelineTopology::new(pipe);
    let out = t2.run(&json!({ "text": "  hello   swarm  " }), &ctx).await;
    println!("\n {} (transform) : {:?} · coût {} · agents {}", t2.name(), out.output["text"], out.cost_units, out.agents_used);

    println!("\n──────────────────────────────────────────────────────────────────────────");
    println!(" → 3 topologies hétérogènes, ASSEMBLÉES depuis la flotte par nom, exécutées");
    println!("   via UNE interface (`Topology`) — la couche d'abstraction §3, en action.");
    println!("══════════════════════════════════════════════════════════════════════════");
}
