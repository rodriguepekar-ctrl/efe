// SPDX-License-Identifier: UNLICENSED
//! # `bench_scale` — benchmark de passage à l'échelle (« grande quantité »)
//!
//! Prouve EMPIRIQUEMENT que la brique « beaucoup d'agents en parallèle » tient : on pousse K
//! (générateurs) jusqu'à plusieurs centaines en **mode keyless (Mode A déterministe)** — le
//! véhicule idéal, car les nano-agents sont du CPU pur (pas de latence modèle), donc on mesure
//! réellement le **substrat + le parallélisme + la provenance + le budget**, pas un LLM.
//!
//! Pour chaque K : N runs, on mesure latence p50/p99, débit (agents/s), et on VÉRIFIE les
//! invariants — `verify` à 100 %, budget jamais dépassé (I13), 0 rupture de chaîne — plus les
//! métriques de substrat (écritures, conflits CAS, événements bus publiés/perdus). Enfin une
//! **rafale concurrente** (plusieurs runs en parallèle, métriques PARTAGÉES) stresse l'atomicité.
//!
//! Écrit `scripts/swarm/EFE_SWARM_SCALE_<date>.json`.
//! Run :  `cargo run --release --example bench_scale -p efe-swarm-sdk`

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use efe_swarm_sdk::{
    Budget, Pattern, ProvenanceMetrics, Swarm, SubstrateMetrics, TopologySpec,
};
use serde_json::{json, Value};

const K_VALUES: &[usize] = &[10, 25, 50, 100, 200];
const RUNS_PER_K: usize = 20;
const JUDGES: &[&str] = &["consistency", "length_band", "j_non_empty"];
const CONCURRENT_BURST: usize = 64; // runs lancés EN MÊME TEMPS (stress de l'état partagé)
const BURST_K: usize = 50;

fn consensus(k: usize) -> TopologySpec {
    TopologySpec::Consensus {
        generator: "echo_generator".into(),
        judges: JUDGES.iter().map(|s| s.to_string()).collect(),
        k: k.into(),
        pattern: efe_swarm_sdk::PatternSpec::Fixed(Pattern::Borda),
        escalate: false,
    }
}

/// Percentile (méthode du plus proche rang) sur des durées en microsecondes.
fn pct(sorted_us: &[u128], p: f64) -> f64 {
    if sorted_us.is_empty() {
        return 0.0;
    }
    let idx = (((sorted_us.len() as f64) * p).ceil() as usize).saturating_sub(1).min(sorted_us.len() - 1);
    sorted_us[idx] as f64 / 1000.0 // → ms
}

struct Row {
    k: usize,
    runs: usize,
    verify_ok: f64,
    budget_ok: bool,
    chain_breaks: u64,
    agents_total: usize,
    wall_ms: f64,
    p50_ms: f64,
    p99_ms: f64,
    agents_per_s: f64,
    bb_writes: u64,
    bb_conflicts: u64,
    ev_published: u64,
    ev_dropped: u64,
    traced_ratio: f64,
}

async fn measure_k(k: usize) -> Row {
    // Métriques FRAÎCHES par K (isolation des compteurs de substrat / provenance).
    let prov = ProvenanceMetrics::new();
    let sub = Arc::new(SubstrateMetrics::default());
    let swarm = Swarm::builder()
        .with_metrics(prov.clone())
        .with_substrate_metrics(sub.clone())
        .build()
        .expect("swarm keyless");

    let cap = ((k + JUDGES.len()) as u32) * 4; // budget large : pas de dégradation, on mesure le plein régime.
    let mut lats_us: Vec<u128> = Vec::with_capacity(RUNS_PER_K);
    let (mut verified, mut budget_ok, mut agents_total) = (0usize, true, 0usize);

    let t0 = Instant::now();
    for i in 0..RUNS_PER_K {
        let task = json!({ "text": format!("scale probe {i}"), "expected": "x" });
        let r0 = Instant::now();
        let outcome = swarm.run_traced(task, consensus(k), Budget::cents(cap)).await.expect("run keyless");
        lats_us.push(r0.elapsed().as_micros());
        if outcome.run.provenance.verified {
            verified += 1;
        }
        if outcome.run.budget.spent_cents > outcome.run.budget.max_cost_cents {
            budget_ok = false; // ne devrait JAMAIS arriver (I13)
        }
        agents_total += outcome.run.accounting.agents_used;
    }
    let wall = t0.elapsed();
    lats_us.sort_unstable();

    let sm = sub.snapshot();
    let pm = prov.snapshot();
    let wall_s = wall.as_secs_f64().max(1e-9);
    Row {
        k,
        runs: RUNS_PER_K,
        verify_ok: verified as f64 / RUNS_PER_K as f64,
        budget_ok,
        chain_breaks: pm.chain_breaks_total,
        agents_total,
        wall_ms: wall.as_millis() as f64,
        p50_ms: pct(&lats_us, 0.50),
        p99_ms: pct(&lats_us, 0.99),
        agents_per_s: agents_total as f64 / wall_s,
        bb_writes: sm.bb_writes_total,
        bb_conflicts: sm.bb_conflicts_total,
        ev_published: sm.ev_published_total,
        ev_dropped: sm.ev_dropped_total,
        traced_ratio: pm.traced_ratio,
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    println!("══════════════════════════════════════════════════════════════════════════════");
    println!(" bench_scale — « grande quantité » : K générateurs ∥ + {} juges (keyless Mode A)", JUDGES.len());
    println!("══════════════════════════════════════════════════════════════════════════════");

    let mut rows = Vec::new();
    for &k in K_VALUES {
        rows.push(measure_k(k).await);
    }

    // ── Table ────────────────────────────────────────────────────────────────────────────
    println!(
        "\n  {:>4} {:>5} {:>7} {:>5} {:>7} {:>8} {:>9} {:>10} {:>9} {:>8} {:>7}",
        "K", "runs", "verify", "I13", "breaks", "p50(ms)", "p99(ms)", "agents/s", "bb_write", "bb_conf", "ev_drop"
    );
    for r in &rows {
        println!(
            "  {:>4} {:>5} {:>6.0}% {:>5} {:>7} {:>8.2} {:>9.2} {:>10.0} {:>9} {:>8} {:>7}",
            r.k, r.runs, r.verify_ok * 100.0, yn(r.budget_ok), r.chain_breaks,
            r.p50_ms, r.p99_ms, r.agents_per_s, r.bb_writes, r.bb_conflicts, r.ev_dropped
        );
    }

    // ── Rafale CONCURRENTE : CONCURRENT_BURST runs en même temps, métriques PARTAGÉES. ─────
    println!("\n  Rafale concurrente : {CONCURRENT_BURST} runs ∥ à K={BURST_K} (métriques partagées) …");
    let prov = ProvenanceMetrics::new();
    let sub = Arc::new(SubstrateMetrics::default());
    let swarm = Arc::new(
        Swarm::builder().with_metrics(prov.clone()).with_substrate_metrics(sub.clone()).build().expect("swarm"),
    );
    let cap = ((BURST_K + JUDGES.len()) as u32) * 4;
    let t0 = Instant::now();
    let mut handles = Vec::with_capacity(CONCURRENT_BURST);
    for i in 0..CONCURRENT_BURST {
        let sw = swarm.clone();
        handles.push(tokio::spawn(async move {
            let task = json!({ "text": format!("burst {i}"), "expected": "x" });
            let o = sw.run_traced(task, consensus(BURST_K), Budget::cents(cap)).await.expect("burst run");
            o.run.provenance.verified
        }));
    }
    let mut burst_verified = 0usize;
    for h in handles {
        if h.await.unwrap_or(false) {
            burst_verified += 1;
        }
    }
    let burst_wall = t0.elapsed();
    let bpm = prov.snapshot();
    let burst_ok = burst_verified == CONCURRENT_BURST && bpm.chain_breaks_total == 0;
    println!(
        "    → {}/{} runs vérifiés · 0 rupture: {} · {:.0} runs/s · records {}",
        burst_verified, CONCURRENT_BURST, yn(bpm.chain_breaks_total == 0),
        CONCURRENT_BURST as f64 / burst_wall.as_secs_f64().max(1e-9), bpm.records_total
    );

    // ── Verdict ──────────────────────────────────────────────────────────────────────────
    let all_verified = rows.iter().all(|r| r.verify_ok >= 1.0);
    let all_budget = rows.iter().all(|r| r.budget_ok);
    let no_breaks = rows.iter().all(|r| r.chain_breaks == 0);
    let go = all_verified && all_budget && no_breaks && burst_ok;
    println!("\n  VERDICT bench_scale : {}", if go { "GO ✓ — invariants tenus à toutes les échelles" } else { "NO-GO ✗" });

    // ── Rapport JSON ───────────────────────────────────────────────────────────────────────
    let date = "2026-06-11";
    let report = json!({
        "bench": "scale",
        "date": date,
        "mode": "keyless (Mode A déterministe)",
        "judges": JUDGES,
        "runs_per_k": RUNS_PER_K,
        "verdict": if go { "GO" } else { "NO-GO" },
        "invariants": {
            "verify_ok_all_k": all_verified,
            "budget_never_exceeded_I13": all_budget,
            "zero_chain_breaks": no_breaks,
            "concurrent_burst_ok": burst_ok
        },
        "concurrent_burst": { "runs": CONCURRENT_BURST, "k": BURST_K, "verified": burst_verified,
                              "runs_per_s": (CONCURRENT_BURST as f64 / burst_wall.as_secs_f64().max(1e-9) * 100.0).round() / 100.0 },
        "by_k": rows.iter().map(row_json).collect::<Vec<_>>(),
    });

    if let Some(root) = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2) {
        let dir = root.join("scripts/swarm");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("EFE_SWARM_SCALE_{date}.json"));
        match std::fs::write(&path, format!("{}\n", serde_json::to_string_pretty(&report).unwrap())) {
            Ok(_) => println!("\n  rapport écrit : {}", path.display()),
            Err(e) => println!("\n  (écriture du rapport échouée : {e})"),
        }
    }
    println!("══════════════════════════════════════════════════════════════════════════════");
}

fn row_json(r: &Row) -> Value {
    json!({
        "k": r.k, "runs": r.runs, "verify_ok": r.verify_ok, "budget_within_cap": r.budget_ok,
        "chain_breaks": r.chain_breaks, "traced_ratio": r.traced_ratio, "agents_total": r.agents_total,
        "wall_ms": (r.wall_ms * 100.0).round() / 100.0,
        "p50_ms": (r.p50_ms * 1000.0).round() / 1000.0, "p99_ms": (r.p99_ms * 1000.0).round() / 1000.0,
        "agents_per_s": r.agents_per_s.round(),
        "bb_writes_total": r.bb_writes, "bb_conflicts_total": r.bb_conflicts,
        "ev_published_total": r.ev_published, "ev_dropped_total": r.ev_dropped
    })
}

fn yn(b: bool) -> &'static str {
    if b { "✓" } else { "✗" }
}
