// SPDX-License-Identifier: UNLICENSED
//! # Lot S2-bis — le gate du Gossip (ADR-SWARM-003), exécutable.
//!
//! **Question** : sur l'estimation continue bruitée, le raffinement itératif
//! (Gossip, r tours) bat-il le **meilleur autre usage du même budget** ?
//!
//! Bras à ISO-BUDGET (B = 12 appels par tâche) :
//! - `single`       : 1 estimateur (le plancher, coût 1 — référence, pas un rival) ;
//! - `mean@B`       : moyenne de B estimations one-shot — **le bras-témoin** : la
//!   réduction de variance SANS itération (loi des grands nombres) ;
//! - `gossip 6×2`   : la VRAIE `GossipTopology` (mean_pull), K=6, r=2  → 12 appels ;
//! - `gossip 4×3`   : idem, K=4, r=3 → 12 appels.
//!
//! **GO** : un bras gossip bat `mean@B` robustement (borne sup gossip < borne inf
//! témoin, bootstrap ±σ). **Prior (ADR-003) : NO** — l'itération sur des moyennes
//! converge vers la moyenne de SES K initiales : à budget égal, moyenner B
//! échantillons indépendants devrait gagner. Ce harnais existe pour que le verdict
//! soit MESURÉ, pas présumé. Artefact : `scripts/swarm/EFE_SWARM_S2BIS_<date>.json`.
//!
//! Run : `cargo run --release --example lot_s2bis -p efe-swarm-sdk`

use std::path::Path;

use efe_swarm_topology::{builtin_refiner, ConvergenceMetric, GossipTopology, SwarmContext, Topology};
use serde_json::{json, Value};

const TRIALS: usize = 400;
const BOOT: usize = 200;
const BUDGET_CALLS: usize = 12;
const DATE: &str = "2026-06-11";

/// splitmix64 — l'aléa DÉTERMINISTE du harnais (reproductible, sans dép `rand`).
fn sm64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E3779B97F4A7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}
/// uniforme [0,1) déterministe depuis une graine composée.
fn u01(seed: u64) -> f64 {
    (sm64(seed) >> 11) as f64 / (1u64 << 53) as f64
}
/// ~N(0,1) par somme de 12 uniformes (Irwin-Hall, suffisant pour le harnais).
fn gauss(seed: u64) -> f64 {
    (0..12).map(|i| u01(seed.wrapping_mul(31).wrapping_add(i))).sum::<f64>() - 6.0
}

/// L'estimation initiale d'un agent : µ + biais(agent) + bruit(tâche, agent).
/// Biais par agent ~N(0,2) (les erreurs CORRÉLÉES par agent — le monde réel),
/// bruit ~N(0,4) par (tâche, agent).
fn initial(trial: usize, agent: usize, mu: f64) -> f64 {
    let bias = 2.0 * gauss(0xB1A5 + agent as u64);
    let noise = 4.0 * gauss((trial as u64) << 16 | agent as u64);
    mu + bias + noise
}

fn mean(xs: &[f64]) -> f64 {
    xs.iter().sum::<f64>() / xs.len().max(1) as f64
}

/// Bootstrap de la moyenne : (moyenne, σ) sur BOOT rééchantillonnages.
fn boot(errors: &[f64], seed: u64) -> (f64, f64) {
    let n = errors.len();
    let mut means = Vec::with_capacity(BOOT);
    for b in 0..BOOT {
        let m: f64 = (0..n).map(|i| errors[(sm64(seed + (b * n + i) as u64) as usize) % n]).sum::<f64>() / n as f64;
        means.push(m);
    }
    let mu = mean(&means);
    let var = means.iter().map(|m| (m - mu) * (m - mu)).sum::<f64>() / BOOT as f64;
    (mu, var.sqrt())
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    println!("══════════════════════════════════════════════════════════════════════");
    println!(" Lot S2-bis — Gossip vs le meilleur usage du même budget (B = {BUDGET_CALLS})");
    println!("══════════════════════════════════════════════════════════════════════");

    let ctx = SwarmContext::standard();
    let arms = ["single", "mean@12", "gossip 6x2", "gossip 4x3"];
    let mut errs: Vec<Vec<f64>> = vec![Vec::with_capacity(TRIALS); arms.len()];

    for t in 0..TRIALS {
        let mu = 50.0 + 100.0 * u01(0xCAFE + t as u64); // µ ∈ [50, 150)
        let inits: Vec<f64> = (0..BUDGET_CALLS).map(|a| initial(t, a, mu)).collect();

        // ① single — le plancher.
        errs[0].push((inits[0] - mu).abs());
        // ② mean@B — le témoin sans itération.
        errs[1].push((mean(&inits) - mu).abs());
        // ③/④ gossip — la VRAIE topologie sur les K premières initiales.
        for (slot, (k, r)) in [(6usize, 2usize), (4, 3)].iter().enumerate() {
            let task = json!({ "initial": inits[..*k].to_vec() });
            let topo = GossipTopology::new(
                builtin_refiner("mean_pull").expect("refiner intégré"),
                *k,
                *r,
                0.0, // ε=0 : on FORCE r tours pleins (iso-coût strict)
                ConvergenceMetric::Variance,
            );
            let out = topo.run(&task, &ctx).await;
            let est = out.output.pointer("/value").and_then(Value::as_f64).unwrap_or(f64::NAN);
            errs[2 + slot].push((est - mu).abs());
        }
    }

    let stats: Vec<(f64, f64)> = errs.iter().enumerate().map(|(i, e)| boot(e, 0x5EED + i as u64)).collect();
    println!("\n  {:<12} {:>10} {:>8} {:>8}", "bras", "|err| moy", "±σ", "coût");
    let costs = [1, BUDGET_CALLS, 12, 12];
    for (i, a) in arms.iter().enumerate() {
        println!("  {:<12} {:>10.3} {:>8.3} {:>8}", a, stats[i].0, stats[i].1, costs[i]);
    }

    // GO ssi un gossip bat le TÉMOIN mean@B robustement (2σ).
    let witness_low = stats[1].0 - 2.0 * stats[1].1;
    let best_gossip = if stats[2].0 < stats[3].0 { 2 } else { 3 };
    let gossip_high = stats[best_gossip].0 + 2.0 * stats[best_gossip].1;
    let go = gossip_high < witness_low;
    let verdict = if go { "GO" } else { "NO-GO" };
    println!(
        "\n  VERDICT S2-bis : {verdict} — meilleur gossip ({}) : {:.3}±{:.3} vs témoin mean@{BUDGET_CALLS} : {:.3}±{:.3}",
        arms[best_gossip], stats[best_gossip].0, stats[best_gossip].1, stats[1].0, stats[1].1
    );
    if !go {
        println!("  (prior ADR-003 confirmé : à budget égal, l'itération n'apporte rien que la moyenne n'ait déjà)");
    }

    let report = json!({
        "lot": "S2-bis", "date": DATE, "_real": "harnais déterministe Mode A (vraie GossipTopology ; estimation simulée biais+bruit)",
        "question": "Gossip(r tours) bat-il le meilleur usage du même budget (moyenne one-shot) ?",
        "budget_calls": BUDGET_CALLS, "trials": TRIALS, "boot": BOOT,
        "arms": arms.iter().enumerate().map(|(i, a)| json!({
            "arm": a, "mean_abs_err": stats[i].0, "sigma": stats[i].1, "cost_calls": costs[i]
        })).collect::<Vec<_>>(),
        "criterion": "GO ssi borne_sup(meilleur gossip, 2σ) < borne_inf(mean@B, 2σ)",
        "verdict": verdict,
        "prior": "NO (ADR-003 : Lot 1 itération Δ=-0,4%±3,1%)",
    });
    if let Some(root) = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2) {
        let dir = root.join("scripts/swarm");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("EFE_SWARM_S2BIS_{DATE}.json"));
        match std::fs::write(&path, format!("{}\n", serde_json::to_string_pretty(&report).unwrap())) {
            Ok(_) => println!("\n  artefact écrit : {}", path.display()),
            Err(e) => println!("\n  (écriture artefact échouée : {e})"),
        }
    }
    println!("══════════════════════════════════════════════════════════════════════");
}
