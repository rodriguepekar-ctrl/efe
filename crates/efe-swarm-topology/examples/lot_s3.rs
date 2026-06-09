// SPDX-License-Identifier: UNLICENSED
//! # `lot_s3` — gate Phase 3 : le consensus crack-t-il le NON-VÉRIFIABLE (F3 de S2) ?
//!
//! EXÉCUTE la VRAIE math consensus (`efe_swarm_topology::consensus`) — Vote, Borda,
//! Condorcet — sur des candidats + ballots de juges SIMULÉS honnêtement.
//!
//! Modèle (hypothèse §0 de l'architecture Phase 3) :
//! - K candidats : générateur k correct avec P = clip(skill_k − γ·difficulté) ; sinon
//!   LEURRE corrélé (→ le vote majoritaire échoue, S1).
//! - J juges : chacun DÉTECTE la correction d'un candidat avec une compétence v
//!   (signal INDÉPENDANT des producteurs), erreurs décorrélées entre juges.
//! - Borda/Condorcet agrègent les ballots des juges → sélectionnent un candidat.
//!
//! Mesure (bootstrap ±σ) : accuracy de best-single / vote / Borda / Condorcet / oracle.
//! VERDICT : Borda/Condorcet battent-ils **vote ET best-single** robustement ?
//! Balayage de la compétence des juges v → montre le seuil (analogue du v* d'AGG).
//!
//! ⚠ Candidats + juges SIMULÉS (modèle honnête). La MATH est réelle (prouvée par
//! oracles). Run pleinement réel = juges Mode C (LlmClient) + llama (ADR-SWARM-004).
//!
//! Run :  cargo run --release --example lot_s3 -p efe-swarm-topology

use std::fs;

use efe_swarm_topology::{Aggregator, Ballot, BordaConsensus, CondorcetConsensus, MajorityVote};
use serde_json::json;

const K: usize = 7; // candidats
const J: usize = 5; // juges
const N: usize = 3000; // tâches
const B: usize = 200; // bootstraps
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
    let mut means = Vec::with_capacity(B);
    for k in 0..B {
        let mut s = 0.0;
        for j in 0..n {
            let idx = (unit(seed ^ (k as u64).wrapping_mul(0x1234) ^ j as u64) * n as f64) as usize % n;
            if arr[idx] {
                s += 1.0;
            }
        }
        means.push(s / n as f64);
    }
    let mu = means.iter().sum::<f64>() / B as f64;
    let var = means.iter().map(|m| (m - mu).powi(2)).sum::<f64>() / B as f64;
    (mu, var.sqrt())
}

struct Row {
    v: f64,
    best_single: (f64, f64),
    vote: (f64, f64),
    borda: (f64, f64),
    condorcet: (f64, f64),
    oracle: (f64, f64),
}

fn run_v(v: f64) -> Row {
    // générateur 0 = le plus fort (= best-single, R0) ; skills décroissants.
    let skills: Vec<f64> = (0..K).map(|k| 0.60 - 0.02 * k as f64).collect();

    let (mut bs, mut vo, mut bo, mut co, mut or) = (
        Vec::with_capacity(N),
        Vec::with_capacity(N),
        Vec::with_capacity(N),
        Vec::with_capacity(N),
        Vec::with_capacity(N),
    );

    for i in 0..N {
        let d = 0.3 + 0.4 * unit(0xD1FF ^ i as u64);
        // candidats + vérité cachée.
        let mut correct = vec![false; K];
        let mut candidates = Vec::with_capacity(K);
        for k in 0..K {
            let p = (skills[k] - GAMMA * d).clamp(0.0, 1.0);
            let ok = unit(0x6E ^ (i as u64).wrapping_mul(0x100000001B3) ^ (k as u64).wrapping_mul(0x9E37)) < p;
            correct[k] = ok;
            // valeur : "T" si correct ; sinon leurre CORRÉLÉ "L" (60 %) ou bruit "R{n}".
            let label = if ok {
                "T".to_string()
            } else if unit(0xAA ^ i as u64 ^ (k as u64).wrapping_mul(7)) < 0.6 {
                "L".to_string()
            } else {
                format!("R{}", k % 3)
            };
            candidates.push(json!(label));
        }

        // ballots des juges : détecteurs de correction de compétence v, décorrélés.
        let mut ballots = Vec::with_capacity(J);
        for j in 0..J {
            let mut scores = vec![0.0f64; K];
            for k in 0..K {
                let flip = unit(0xB0 ^ (i as u64).wrapping_mul(131) ^ (j as u64).wrapping_mul(17) ^ k as u64) >= v;
                let detected = correct[k] ^ flip; // détecte correct[k] avec prob v
                // petit bruit pour départager les ex-æquo (signal dominant).
                let noise = unit(0xC0 ^ i as u64 ^ (j as u64) << 8 ^ (k as u64) << 16) * 1e-3;
                scores[k] = if detected { 1.0 } else { 0.0 } + noise;
            }
            ballots.push(Ballot::new(format!("judge_{j}"), scores));
        }

        let task = json!({});
        let sel_v = MajorityVote.aggregate(&task, &candidates, &[]).selected_index;
        let sel_b = BordaConsensus.aggregate(&task, &candidates, &ballots).selected_index;
        let sel_c = CondorcetConsensus.aggregate(&task, &candidates, &ballots).selected_index;

        bs.push(correct[0]); // best-single = générateur le plus fort
        vo.push(correct[sel_v]);
        bo.push(correct[sel_b]);
        co.push(correct[sel_c]);
        or.push(correct.iter().any(|&c| c)); // oracle = un correct existe
    }

    Row {
        v,
        best_single: boot(&bs, 1),
        vote: boot(&vo, 2),
        borda: boot(&bo, 3),
        condorcet: boot(&co, 4),
        oracle: boot(&or, 5),
    }
}

fn main() {
    let sweep = [0.55, 0.70, 0.85];
    let rows: Vec<Row> = sweep.iter().map(|&v| run_v(v)).collect();

    println!("══════════════════════════════════════════════════════════════════════════════");
    println!(" LOT S3 — consensus vs vote vs best-single, NON-VÉRIFIABLE · K={K} J={J} · {N} tâches");
    println!(" (Vote/Borda/Condorcet = VRAIE math ; candidats + juges simulés)");
    println!("══════════════════════════════════════════════════════════════════════════════");
    let pp = |t: (f64, f64)| format!("{:.3}±{:.3}", t.0, t.1);
    println!(" {:>10} {:>13} {:>13} {:>13} {:>13} {:>13}", "juges v", "best-single", "vote", "Borda", "Condorcet", "oracle");
    println!(" {}", "─".repeat(80));
    for r in &rows {
        println!(
            " {:>10.2} {:>13} {:>13} {:>13} {:>13} {:>13}",
            r.v, pp(r.best_single), pp(r.vote), pp(r.borda), pp(r.condorcet), pp(r.oracle)
        );
    }
    println!(" {}", "─".repeat(80));

    // verdict au point principal (v = 0,70).
    let main = rows.iter().find(|r| (r.v - 0.70).abs() < 1e-9).unwrap();
    let beats = |a: (f64, f64), b: (f64, f64)| (a.0 - a.1) > b.0; // borne basse > moyenne baseline
    let best_consensus = if main.borda.0 >= main.condorcet.0 { ("Borda", main.borda) } else { ("Condorcet", main.condorcet) };
    let (cname, cval) = best_consensus;
    let go = beats(cval, main.best_single) && beats(cval, main.vote);

    let verdict = if go {
        format!(
            "GO — {cname} ({:.3}) bat best-single ({:.3}) ET vote ({:.3}) robustement → le consensus N-juges crack le F3",
            cval.0, main.best_single.0, main.vote.0
        )
    } else if main.oracle.0 > cval.0 + 0.1 {
        "INCONCLUS — la capacité existe (oracle haut) mais les juges ne l'extraient pas (v trop faible)".to_string()
    } else {
        "NO-GO — le consensus ne bat pas (vote, best-single) : la non-vérifiabilité reste un mur".to_string()
    };

    println!("\n────────────────────────── VERDICT (juges v=0,70) ──────────────────────────");
    println!(" Δqualité {cname} − best-single : {:+.3}  ·  {cname} − vote : {:+.3}", cval.0 - main.best_single.0, cval.0 - main.vote.0);
    println!(" {verdict}");
    println!(" Coût : best-single = 1 génération ; consensus = {K} générations + juges Mode A GRATUITS.");
    println!(" Seuil : Borda/Condorcet ne battent le vote QUE si les juges ajoutent du signal (v ↑).");
    println!(" Portée : candidats + juges SIMULÉS ; math RÉELLE. Run réel = juges Mode C + llama (ADR-SWARM-004).");
    println!("══════════════════════════════════════════════════════════════════════════════");

    // écrit les résultats.
    let payload = json!({
        "k": K, "j": J, "n": N, "boot": B,
        "rows": rows.iter().map(|r| json!({
            "judge_skill_v": r.v,
            "best_single": [r.best_single.0, r.best_single.1],
            "vote": [r.vote.0, r.vote.1],
            "borda": [r.borda.0, r.borda.1],
            "condorcet": [r.condorcet.0, r.condorcet.1],
            "oracle": [r.oracle.0, r.oracle.1],
        })).collect::<Vec<_>>(),
        "verdict": verdict,
        "_note": "Candidats + juges simulés ; math consensus réelle. Run réel = juges Mode C + llama.",
    });
    let _ = fs::create_dir_all("scripts/swarm");
    let _ = fs::write("scripts/swarm/EFE_SWARM_S3_2026-06-09.json", serde_json::to_string_pretty(&payload).unwrap());
    let md = format!(
        "# EFE Lot S3 — consensus vs vote vs best-single (non-vérifiable)\n\n> Gate Phase 3 (ADR-SWARM-004). Vote/Borda/Condorcet = math RÉELLE ; candidats + juges simulés. K={K}, J={J}, {N} tâches, {B} bootstraps.\n\n**Verdict (juges v=0,70) : {verdict}**\n\n| juges v | best-single | vote | Borda | Condorcet | oracle |\n|---|---|---|---|---|---|\n{}\n\n## Lecture\n- **best-single** = meilleur générateur seul (R0). **vote** = mode des valeurs (plancher S1).\n- **Borda/Condorcet** = consensus sur les ballots de J juges (signal indépendant).\n- Le consensus crack le non-vérifiable SSI les juges ajoutent du signal (v au-dessus d'un seuil).\n- Run réel = juges Mode C (LlmClient) + llama-server.\n",
        rows.iter().map(|r| format!("| {:.2} | {} | {} | {} | {} | {} |", r.v, pp(r.best_single), pp(r.vote), pp(r.borda), pp(r.condorcet), pp(r.oracle))).collect::<Vec<_>>().join("\n")
    );
    let _ = fs::write("scripts/swarm/EFE_SWARM_S3_2026-06-09.md", md);
    println!("\n✅ Résultats : scripts/swarm/EFE_SWARM_S3_2026-06-09.{{md,json}}");
}
