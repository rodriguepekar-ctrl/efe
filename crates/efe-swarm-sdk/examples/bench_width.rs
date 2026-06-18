// SPDX-License-Identifier: UNLICENSED
//! # `bench_width` — la **largeur d'exécution** mesurée, et la sémantique assertée (ADR-014).
//!
//! Trois questions chiffrées (architecture §8.2), sur la machine de référence :
//! 1. le plateau intra-run (~22 k agents/s à largeur 8, bench_scale 2026-06-11)
//!    bouge-t-il à largeur = cœurs (11 ici) ? — l'hypothèse écrite AVANT mesure est
//!    +20-35 % ; si rien ne bouge, le goulot est ailleurs (sérialisation du recorder) ;
//! 2. au-delà des cœurs (16, 32) : churn ou stagnation ? — fixe le plafond serveur ;
//! 3. largeur 1 : le coût exact du séquentiel (la borne basse qui rend tout lisible).
//!
//! **Et le gardien** : à chaque case, un run-témoin (tâche fixe) doit produire la MÊME
//! empreinte sémantique (agents, variants, hashes, cellules) que la case largeur 1 —
//! I-CP1 vérifié sur toute la grille, à chaque exécution du bench, pour toujours.
//!
//! Écrit `scripts/swarm/EFE_SWARM_WIDTH_<date>.json`.
//! Run : `cargo run --release --example bench_width -p efe-swarm-sdk`

use std::path::Path;
use std::time::Instant;

use efe_swarm_sdk::{Budget, RunOutcome, Swarm, TopologySpec, WidthSpec};
use serde_json::{json, Value};

const DATE: &str = "2026-06-12";
const WIDTHS: &[usize] = &[1, 2, 4, 8, 11, 16, 32];
const K_VALUES: &[usize] = &[50, 200];
const RUNS: usize = 20;
const JUDGES: &[&str] = &["consistency", "length_band", "j_non_empty"];

fn consensus(k: usize) -> TopologySpec {
    TopologySpec::Consensus {
        generator: "echo_generator".into(),
        judges: JUDGES.iter().map(|s| s.to_string()).collect(),
        k: k.into(),
        pattern: efe_swarm_sdk::PatternSpec::Fixed(efe_swarm_sdk::Pattern::Borda),
        escalate: false,
    }
}

fn pct(sorted_us: &[u128], p: f64) -> f64 {
    if sorted_us.is_empty() {
        return 0.0;
    }
    let idx = (((sorted_us.len() as f64) * p).ceil() as usize).saturating_sub(1).min(sorted_us.len() - 1);
    sorted_us[idx] as f64 / 1000.0
}

/// Empreinte SÉMANTIQUE d'un run (tout ce que la largeur n'a pas le droit de changer).
fn fingerprint(out: &RunOutcome) -> Vec<String> {
    let mut fp: Vec<String> = out
        .sealed
        .as_ref()
        .expect("scellé")
        .records
        .iter()
        .map(|r| {
            let writes: Vec<String> =
                r.writes.iter().map(|c| format!("{}@{}", c.pointer, c.version)).collect();
            format!(
                "{}|{:?}|{}|{}|w={}",
                r.agent_id.as_str(),
                r.variant,
                r.input_hash.to_hex(),
                r.output_hash.to_hex(),
                writes.join(",")
            )
        })
        .collect();
    fp.sort();
    fp
}

struct Row {
    width: String,
    k: usize,
    p50_ms: f64,
    p99_ms: f64,
    agents_per_s: f64,
    verify_ok: bool,
    budget_ok: bool,
}

async fn measure(width: Option<usize>, k: usize) -> (Row, RunOutcome) {
    let mut builder = Swarm::builder();
    if let Some(w) = width {
        builder = builder.width(WidthSpec::Fixed(w));
    }
    let swarm = builder.build().expect("swarm keyless");
    let cap = ((k + JUDGES.len()) as u32) * 4;
    let mut lats: Vec<u128> = Vec::with_capacity(RUNS);
    let (mut verified, mut budget_ok, mut agents_total) = (0usize, true, 0usize);
    let t0 = Instant::now();
    let mut witness: Option<RunOutcome> = None;
    for i in 0..RUNS {
        // i = 0 : la tâche-TÉMOIN (fixe) — son empreinte est comparée entre largeurs.
        let task = if i == 0 {
            json!({ "text": "width witness", "expected": "width witness" })
        } else {
            json!({ "text": format!("width probe {i}"), "expected": "x" })
        };
        let r0 = Instant::now();
        let out = swarm.run_traced(task, consensus(k), Budget::cents(cap)).await.expect("run");
        lats.push(r0.elapsed().as_micros());
        if out.run.provenance.verified {
            verified += 1;
        }
        if out.run.budget.spent_cents > out.run.budget.max_cost_cents {
            budget_ok = false;
        }
        agents_total += out.run.accounting.agents_used;
        if i == 0 {
            witness = Some(out);
        }
    }
    let wall_s = t0.elapsed().as_secs_f64().max(1e-9);
    lats.sort_unstable();
    let row = Row {
        width: width.map(|w| w.to_string()).unwrap_or_else(|| "auto".into()),
        k,
        p50_ms: pct(&lats, 0.50),
        p99_ms: pct(&lats, 0.99),
        agents_per_s: agents_total as f64 / wall_s,
        verify_ok: verified == RUNS,
        budget_ok,
    };
    (row, witness.expect("témoin"))
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0);
    println!("══════════════════════════════════════════════════════════════════════════════");
    println!(" bench_width — largeur mesurée + sémantique assertée (ADR-014) · {cores} cœurs · {RUNS} runs/case");
    println!("══════════════════════════════════════════════════════════════════════════════");

    let mut rows: Vec<Row> = Vec::new();
    let mut semantics_ok = true;
    let mut by_k_json: Vec<Value> = Vec::new();

    for &k in K_VALUES {
        // référence : largeur 1 (séquentiel intégral) — l'empreinte-témoin.
        let (row1, witness1) = measure(Some(1), k).await;
        let reference = fingerprint(&witness1);
        rows.push(row1);
        for &w in &WIDTHS[1..] {
            let (row, witness) = measure(Some(w), k).await;
            let same = fingerprint(&witness) == reference;
            if !same {
                semantics_ok = false;
                println!("  ✗ I-CP1 VIOLÉ : K={k}, largeur {w} ≠ largeur 1 (empreinte sémantique)");
            }
            rows.push(row);
        }
        // ligne Auto (résolution cœurs) — la politique par défaut, mesurée elle aussi.
        let (row_auto, witness_auto) = measure(None, k).await;
        if fingerprint(&witness_auto) != reference {
            semantics_ok = false;
            println!("  ✗ I-CP1 VIOLÉ : K={k}, Auto ≠ largeur 1");
        }
        rows.push(row_auto);
    }

    println!("\n  {:>6} {:>5} {:>9} {:>9} {:>11} {:>7} {:>5}", "width", "K", "p50(ms)", "p99(ms)", "agents/s", "verify", "I13");
    for r in &rows {
        println!(
            "  {:>6} {:>5} {:>9.3} {:>9.3} {:>11.0} {:>7} {:>5}",
            r.width, r.k, r.p50_ms, r.p99_ms, r.agents_per_s,
            if r.verify_ok { "✓" } else { "✗" },
            if r.budget_ok { "✓" } else { "✗" }
        );
        by_k_json.push(json!({
            "width": r.width, "k": r.k,
            "p50_ms": (r.p50_ms * 1000.0).round() / 1000.0,
            "p99_ms": (r.p99_ms * 1000.0).round() / 1000.0,
            "agents_per_s": r.agents_per_s.round(),
            "verify_ok": r.verify_ok, "budget_within_cap": r.budget_ok,
        }));
    }

    // lecture chiffrée : la voie 8 (le « 8 » câblé historique) vs les cœurs.
    let tput = |w: &str, k: usize| rows.iter().find(|r| r.width == w && r.k == k).map(|r| r.agents_per_s);
    if let (Some(t8), Some(tc)) = (tput("8", 200), tput(&cores.to_string(), 200)) {
        let gain = (tc / t8 - 1.0) * 100.0;
        println!("\n  largeur 8 → {cores} (K=200) : {t8:.0} → {tc:.0} agents/s ({gain:+.1} %) — hypothèse écrite : +20-35 %");
    }

    let all_verified = rows.iter().all(|r| r.verify_ok);
    let all_budget = rows.iter().all(|r| r.budget_ok);
    let go = all_verified && all_budget && semantics_ok;
    println!("\n  I-CP1 (empreinte sémantique identique sur toute la grille) : {}", if semantics_ok { "✓" } else { "✗" });
    println!("  VERDICT bench_width : {}", if go { "GO ✓ — la largeur est une politique d'exécution, jamais une sémantique" } else { "NO-GO ✗" });

    let report = json!({
        "bench": "width",
        "date": DATE,
        "adr": "ADR-SWARM-014",
        "mode": "keyless (Mode A déterministe)",
        "machine_cores": cores,
        "runs_per_cell": RUNS,
        "verdict": if go { "GO" } else { "NO-GO" },
        "invariants": {
            "semantic_fingerprint_invariant_under_width_ICP1": semantics_ok,
            "verify_ok_all_cells": all_verified,
            "budget_never_exceeded_I13": all_budget,
        },
        "grid": by_k_json,
    });
    if let Some(root) = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2) {
        let dir = root.join("scripts/swarm");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("EFE_SWARM_WIDTH_{DATE}.json"));
        match std::fs::write(&path, format!("{}\n", serde_json::to_string_pretty(&report).unwrap())) {
            Ok(_) => println!("\n  rapport écrit : {}", path.display()),
            Err(e) => println!("\n  (écriture du rapport échouée : {e})"),
        }
    }
    println!("══════════════════════════════════════════════════════════════════════════════");
}
