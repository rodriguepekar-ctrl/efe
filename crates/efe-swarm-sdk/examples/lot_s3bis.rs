// SPDX-License-Identifier: UNLICENSED
//! # Lot S3-bis — le gate du Debate (ADR-SWARM-005), exécutable.
//!
//! **Question** : le débat (r tours d'arguments PRO/CONTRA avant le jury) bat-il le
//! **meilleur autre usage du même budget** — c.-à-d. payer PLUS DE JUGES ?
//!
//! Modèle (le protocole S3, étendu) : K = 7 candidats, 1 correct ; un juge de
//! compétence v score le correct plus haut avec probabilité v. Le débat est modélisé
//! comme un **gain de compétence effectif** : v_eff = v + δ — δ est précisément ce
//! qu'une mesure RÉELLE (Mode C) devra établir ; ce harnais calcule le **seuil de
//! rentabilité δ\*** : en-dessous, le témoin « plus de juges » gagne à coût égal.
//!
//! Bras à ISO-BUDGET (coût en appels : K gén. + 2K×r arguments + J juges) :
//! - `borda J=5` (baseline S3, coût 12) ;
//! - `debate r=1, J=5, v+δ` (coût 26)  vs  témoin `borda J=19` (coût 26) ;
//! - `debate r=2, J=5, v+δ` (coût 40)  vs  témoin `borda J=33` (coût 40).
//!
//! **Verdict** : conditionnel par construction — « NO-GO tant que δ_mesuré < δ\* ».
//! Le prior S3 (le levier est v, pas les tours) est ENCODÉ dans le témoin.
//! Artefact : `scripts/swarm/EFE_SWARM_S3BIS_<date>.json`.
//!
//! Run : `cargo run --release --example lot_s3bis -p efe-swarm-sdk`

use std::path::Path;

use efe_swarm_topology::{Ballot, BordaConsensus, Aggregator};
use serde_json::json;

const TRIALS: usize = 2000;
const K: usize = 7;
const V: f64 = 0.70;
const DELTAS: [f64; 4] = [0.0, 0.05, 0.10, 0.15];
const DATE: &str = "2026-06-11";

fn sm64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E3779B97F4A7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}
fn u01(seed: u64) -> f64 {
    (sm64(seed) >> 11) as f64 / (1u64 << 53) as f64
}

/// Un ballot de juge à compétence v : le juge « voit juste » avec probabilité v —
/// alors le correct reçoit le meilleur score ; sinon les scores sont du bruit pur.
fn ballot(trial: usize, judge: usize, salt: u64, correct: usize, v: f64) -> Ballot {
    let seed = ((trial as u64) << 24) | ((judge as u64) << 8) | salt;
    let sees_right = u01(seed) < v;
    let scores: Vec<f64> = (0..K)
        .map(|c| {
            let noise = u01(seed.wrapping_mul(K as u64 + 1).wrapping_add(c as u64));
            if sees_right && c == correct {
                0.8 + 0.2 * noise // nettement au-dessus
            } else {
                0.6 * noise
            }
        })
        .collect();
    Ballot::new(format!("j{judge}"), scores)
}

/// Exactitude du Borda sur TRIALS tâches, avec J juges de compétence v.
fn accuracy(j: usize, v: f64, salt: u64) -> f64 {
    let cands: Vec<serde_json::Value> = (0..K).map(|c| json!({ "cand": c })).collect();
    let mut ok = 0usize;
    for t in 0..TRIALS {
        let correct = (sm64(0xC0FFEE + t as u64) as usize) % K;
        let ballots: Vec<Ballot> = (0..j).map(|jj| ballot(t, jj, salt, correct, v)).collect();
        let out = BordaConsensus.aggregate(&json!({}), &cands, &ballots);
        if out.selected_index == correct {
            ok += 1;
        }
    }
    ok as f64 / TRIALS as f64
}

fn main() {
    println!("══════════════════════════════════════════════════════════════════════");
    println!(" Lot S3-bis — Debate (v+δ) vs « plus de juges » au même budget (v={V})");
    println!("══════════════════════════════════════════════════════════════════════");

    let base = accuracy(5, V, 1);
    println!("\n  baseline  borda J=5  v={V}        acc={base:.3}  (coût 12)");

    let mut rows = Vec::new();
    let mut break_even: Option<f64> = None;
    for (r, witness_j, cost) in [(1usize, 19usize, 26usize), (2, 33, 40)] {
        let witness = accuracy(witness_j, V, 2 + r as u64);
        println!("\n  — iso-budget {cost} appels (debate r={r}) —");
        println!("  témoin    borda J={witness_j} v={V}      acc={witness:.3}");
        for d in DELTAS {
            let debate = accuracy(5, (V + d).min(0.95), 7 + (d * 100.0) as u64);
            let wins = debate > witness;
            println!(
                "  debate    r={r} J=5  v_eff={:.2}   acc={debate:.3}  {}",
                V + d,
                if wins { "≥ témoin ✓" } else { "< témoin" }
            );
            if wins && break_even.is_none() {
                break_even = Some(d);
            }
            rows.push(json!({ "r": r, "cost": cost, "delta": d, "acc_debate": debate,
                              "witness_j": witness_j, "acc_witness": witness, "beats_witness": wins }));
        }
    }

    let be = break_even.unwrap_or(f64::INFINITY);
    println!(
        "\n  VERDICT S3-bis : CONDITIONNEL — seuil de rentabilité δ* ≈ {be:.2} ;\n  \
         NO-GO tant que le δ RÉEL (à mesurer en Mode C : juges avec vs sans dossier)\n  \
         n'atteint pas ce seuil. Le prior S3 (« le levier est v ») est encodé dans le témoin."
    );

    let report = json!({
        "lot": "S3-bis", "date": DATE,
        "_note": "harnais déterministe : math Borda RÉELLE ; juges simulés à compétence v ; le débat modélisé en v_eff = v + δ — δ réel à mesurer en Mode C",
        "protocol": { "k": K, "v": V, "trials": TRIALS, "baseline_acc": base },
        "rows": rows,
        "break_even_delta": if be.is_finite() { json!(be) } else { json!(null) },
        "criterion": "GO ssi δ_mesuré(Mode C) ≥ δ* ; sinon investir dans v (plus/meilleurs juges)",
        "verdict": "CONDITIONNEL (NO-GO par défaut, prior ADR-005)",
    });
    if let Some(root) = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2) {
        let dir = root.join("scripts/swarm");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("EFE_SWARM_S3BIS_{DATE}.json"));
        match std::fs::write(&path, format!("{}\n", serde_json::to_string_pretty(&report).unwrap())) {
            Ok(_) => println!("\n  artefact écrit : {}", path.display()),
            Err(e) => println!("\n  (écriture artefact échouée : {e})"),
        }
    }
    println!("══════════════════════════════════════════════════════════════════════");
}
