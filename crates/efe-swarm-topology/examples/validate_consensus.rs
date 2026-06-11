// SPDX-License-Identifier: UNLICENSED
//! # `validate_consensus` — validation Phase 3 : Δqualité + cost/gain multi-familles.
//!
//! EXÉCUTE la VRAIE `ConsensusTopology` (substrat : candidats au Blackboard, juges-
//! agents écrivent leurs ballots, agrégation Borda) sur 3 familles différant par la
//! compétence des juges v. Mesure Δqualité(consensus) vs best-single ET vs vote, et
//! le coût/gain. Bootstrap ±σ.
//!
//! ⚠ Candidats + juges SIMULÉS (juge de compétence v) ; TOPOLOGIE + math RÉELLES.
//! La compétence v RÉELLE d'un juge LLM se mesure avec llama (cf. `llm_judge`).
//!
//! Run :  cargo run --release --example validate_consensus -p efe-swarm-topology

use std::fs;
use std::sync::Arc;

use async_trait::async_trait;
use efe_swarm_topology::{
    Aggregator, ConsensusTopology, Generator, Judge, MajorityVote, Pattern, SwarmContext, Topology,
};
use serde_json::{json, Value};

const K: usize = 7;
const JN: usize = 5;
const N: usize = 300;
const B: usize = 150;
const GAMMA: f64 = 0.5;

fn unit(mut z: u64) -> f64 {
    z = z.wrapping_add(0x9E3779B97F4A7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

fn boot(arr: &[bool], seed: u64) -> (f64, f64) {
    let n = arr.len().max(1);
    let mut m = Vec::with_capacity(B);
    for k in 0..B {
        let mut s = 0.0;
        for j in 0..n {
            let idx = (unit(seed ^ (k as u64).wrapping_mul(0x1234) ^ j as u64) * n as f64) as usize % n;
            if arr[idx] {
                s += 1.0;
            }
        }
        m.push(s / n as f64);
    }
    let mu = m.iter().sum::<f64>() / B as f64;
    let var = m.iter().map(|x| (x - mu).powi(2)).sum::<f64>() / B as f64;
    (mu, var.sqrt())
}

/// Générateur DÉTERMINISTE : générateur fort en 0 (= best-single) ; leurres corrélés.
struct SimGen;
#[async_trait]
impl Generator for SimGen {
    async fn generate(&self, task: &Value, variant: usize) -> Value {
        let id = task.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
        let d = task.get("difficulty").and_then(|v| v.as_f64()).unwrap_or(0.5);
        let skill = 0.60 - 0.02 * variant as f64;
        let p = (skill - GAMMA * d).clamp(0.0, 1.0);
        let correct = unit(0x6E ^ id.wrapping_mul(0x100000001B3) ^ (variant as u64).wrapping_mul(0x9E37)) < p;
        if correct {
            json!({ "value": task.get("expected").cloned().unwrap_or(Value::Null) })
        } else {
            json!({ "value": "L" }) // leurre CORRÉLÉ → le vote échoue
        }
    }
}

/// Juge SIMULÉ de compétence v : détecte la correction avec prob v, décorrélé entre juges.
struct SimJudge {
    v: f64,
    idx: usize,
}
#[async_trait]
impl Judge for SimJudge {
    async fn assess(&self, task: &Value, candidates: &[Value]) -> Vec<f64> {
        let id = task.get("id").and_then(|x| x.as_u64()).unwrap_or(0);
        candidates
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let correct = c.get("value") == task.get("expected");
                let flip = unit(0x5E ^ id.wrapping_mul(131) ^ ((self.idx as u64) << 8) ^ i as u64) >= self.v;
                let detected = correct ^ flip;
                let noise = unit(0xC0 ^ id ^ ((self.idx as u64) << 16) ^ ((i as u64) << 24)) * 1e-3;
                (if detected { 1.0 } else { 0.0 }) + noise
            })
            .collect()
    }
    fn name(&self) -> &str {
        "sim_judge"
    }
}

struct Fam {
    v: f64,
    consensus: (f64, f64),
    best_single: (f64, f64),
    vote: (f64, f64),
    oracle: (f64, f64),
}

async fn run_family(v: f64) -> Fam {
    let gen: Arc<dyn Generator> = Arc::new(SimGen);
    let ctx = SwarmContext::standard();
    let (mut cons, mut bs, mut vo, mut or) = (vec![], vec![], vec![], vec![]);

    for i in 0..N {
        let expected = json!("T");
        let task = json!({ "id": i as u64, "expected": expected, "difficulty": 0.3 + 0.4 * unit(0xD1 ^ i as u64) });

        // reproduit les candidats (déterministe) → best-single / vote / oracle.
        let mut cands = Vec::with_capacity(K);
        for variant in 0..K {
            cands.push(gen.generate(&task, variant).await);
        }
        bs.push(cands[0].get("value") == Some(&expected)); // variant 0 = générateur le plus fort
        let vote = MajorityVote.aggregate(&task, &cands, &[]).selected;
        vo.push(vote.get("value") == Some(&expected));
        or.push(cands.iter().any(|c| c.get("value") == Some(&expected)));

        // exécute la VRAIE ConsensusTopology (juges de compétence v, Borda).
        let judges: Vec<Arc<dyn Judge>> = (0..JN).map(|idx| Arc::new(SimJudge { v, idx }) as Arc<dyn Judge>).collect();
        let topo = ConsensusTopology::new(gen.clone(), K, judges).with_pattern(Pattern::Borda);
        let out = topo.run(&task, &ctx).await;
        cons.push(out.output.get("value") == Some(&expected));
    }

    Fam {
        v,
        consensus: boot(&cons, 1),
        best_single: boot(&bs, 2),
        vote: boot(&vo, 3),
        oracle: boot(&or, 4),
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let sweep = [0.55, 0.70, 0.85];
    let mut fams = Vec::new();
    for &v in &sweep {
        fams.push(run_family(v).await);
    }

    println!("══════════════════════════════════════════════════════════════════════════════");
    println!(" VALIDATION CONSENSUS (vraie ConsensusTopology) — Δqualité + cost/gain · K={K} J={JN}");
    println!("══════════════════════════════════════════════════════════════════════════════");
    let pp = |t: (f64, f64)| format!("{:.3}±{:.3}", t.0, t.1);
    println!(" {:>9} {:>13} {:>13} {:>13} {:>13} {:>11} {:>11}", "juges v", "consensus", "best-single", "vote", "oracle", "Δvs-best", "Δvs-vote");
    println!(" {}", "─".repeat(90));
    let mut go_families = 0;
    for f in &fams {
        let d_bs = f.consensus.0 - f.best_single.0;
        let d_vo = f.consensus.0 - f.vote.0;
        let robust = (f.consensus.0 - f.consensus.1) > f.best_single.0 && (f.consensus.0 - f.consensus.1) > f.vote.0;
        if robust {
            go_families += 1;
        }
        println!(
            " {:>9.2} {:>13} {:>13} {:>13} {:>13} {:>+11.3} {:>+11.3}{}",
            f.v, pp(f.consensus), pp(f.best_single), pp(f.vote), pp(f.oracle), d_bs, d_vo,
            if robust { " ✓" } else { "" }
        );
    }
    println!(" {}", "─".repeat(90));
    // cost/gain au point de référence v=0,70.
    let mid = fams.iter().find(|f| (f.v - 0.70).abs() < 1e-9).unwrap();
    let gain = mid.consensus.0 - mid.best_single.0;
    println!("\n────────────────────────── SLO PHASE 3 (réf. v=0,70) ──────────────────────────");
    println!(" Δqualité consensus − best-single : {:+.3}  ·  − vote : {:+.3}", gain, mid.consensus.0 - mid.vote.0);
    println!(" Coût/gain : consensus = {K} générations (+ juges) vs best-single = 1 → +{gain:.3} qualité pour {K}× coût.");
    println!(" Familles GO (consensus bat best-single ET vote, robuste) : {go_families}/{}", fams.len());
    let verdict = if go_families >= 1 {
        "GO — le consensus apporte un Δqualité positif robuste dès que les juges ont du signal (v≥~0,7)"
    } else {
        "EN-DEÇÀ — juges trop faibles ; revoir v / la diversité"
    };
    println!(" VERDICT : {verdict}");
    println!(" Portée : candidats + juges SIMULÉS ; topologie + math RÉELLES. v réel = juges LLM + llama.");
    println!("══════════════════════════════════════════════════════════════════════════════");

    let payload = json!({
        "k": K, "j": JN, "n": N,
        "families": fams.iter().map(|f| json!({
            "judge_v": f.v,
            "consensus": [f.consensus.0, f.consensus.1],
            "best_single": [f.best_single.0, f.best_single.1],
            "vote": [f.vote.0, f.vote.1],
            "oracle": [f.oracle.0, f.oracle.1],
            "delta_vs_best": f.consensus.0 - f.best_single.0,
            "delta_vs_vote": f.consensus.0 - f.vote.0,
        })).collect::<Vec<_>>(),
        "go_families": go_families, "verdict": verdict,
        "_note": "ConsensusTopology RÉELLE ; candidats+juges simulés (compétence v). v réel = LLM+llama.",
    });
    let _ = fs::create_dir_all("scripts/swarm");
    let _ = fs::write("scripts/swarm/EFE_SWARM_VALCONS_2026-06-09.json", serde_json::to_string_pretty(&payload).unwrap());
    println!("\n✅ Résultats : scripts/swarm/EFE_SWARM_VALCONS_2026-06-09.json");
}
