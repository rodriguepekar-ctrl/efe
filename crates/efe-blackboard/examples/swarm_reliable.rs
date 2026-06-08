// SPDX-License-Identifier: UNLICENSED
//! # `swarm_reliable` — WS1 ferme le ❌ de la baseline (perte best-effort).
//!
//! La baseline `swarm_load` a montré : sous charge, le bus best-effort perd
//! ~99 % des événements (p99 25 ms). Ici, on met la lane FIABLE ([`ReliableTopic`]
//! + [`TopicRouter`]) dans les MÊMES conditions hostiles (capacité de bus
//! minuscule ⇒ notifs massivement perdues) et on démontre **0 % de perte** :
//! le payload est dans le Blackboard (vérité) ; le lecteur réconcilie.
//!
//! Run :
//! ```text
//! cargo run --release --example swarm_reliable -p efe-blackboard
//! cargo run --release --example swarm_reliable -p efe-blackboard -- 100,500,1000 100000
//! ```

use std::sync::Arc;
use std::time::Instant;

use efe_blackboard::{Blackboard, ReliableTopic, SubstrateMetrics, TopicRouter};
use efe_nano_sdk::{AgentId, SwarmId};
use serde_json::json;

const ROUTER_CAP: usize = 64; // VOLONTAIREMENT minuscule → notifs massivement perdues
const RING_CAP: usize = 1024;

struct Row {
    n: usize,
    per: usize,
    produced: u64,
    notif_published: u64,
    notif_dropped: u64,
    delivered: u64,
    events_per_s: f64,
    p99: f64,
}

async fn trial(n: usize, budget: usize) -> Row {
    let per = (budget / n).max(1);
    let produced = (per * n) as u64;

    let swarm_id = SwarmId::new();
    let metrics = SubstrateMetrics::new();
    let bb = Arc::new(Blackboard::with_recent_cap(swarm_id, 256_000_000, RING_CAP));
    bb.set_metrics(metrics.clone());
    let router = Arc::new(TopicRouter::with_metrics(ROUTER_CAP, metrics.clone()));
    let topic = ReliableTopic::new("/rel/work", "work");

    // Lecteur fiable : s'abonne AVANT les producteurs, draine en flux.
    let reader_handle = {
        let mut reader = topic.reader(&router);
        let bb = bb.clone();
        tokio::spawn(async move {
            let mut delivered = 0u64;
            while let Some(batch) = reader.recv_batch(&bb).await {
                delivered += batch.len() as u64;
            }
            delivered
        })
    };

    // Producteurs : N en parallèle, chacun publie `per` items de façon fiable.
    let start = Instant::now();
    let mut producers = Vec::new();
    for p in 0..n {
        let bb = bb.clone();
        let router = router.clone();
        let topic = topic.clone();
        let id = AgentId::new(format!("prod_{p:05}")).unwrap();
        producers.push(tokio::spawn(async move {
            for i in 0..per {
                let _ = topic.publish(&router, &bb, &id, json!({ "p": p, "i": i }));
            }
        }));
    }
    for p in producers {
        let _ = p.await;
    }
    let wall = start.elapsed();

    // Ferme le routeur → le lecteur fait un dernier drain (queue) puis termine.
    drop(router);
    let delivered = reader_handle.await.unwrap_or(0);

    let s = metrics.snapshot();
    let secs = wall.as_secs_f64().max(1e-9);
    Row {
        n,
        per,
        produced,
        notif_published: s.ev_published_total,
        notif_dropped: s.ev_dropped_total,
        delivered,
        events_per_s: produced as f64 / secs,
        p99: s.propagation_p99_ms,
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let sweep: Vec<usize> = args
        .get(1)
        .map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect())
        .unwrap_or_else(|| vec![100, 500, 1000]);
    let budget: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(100_000);

    println!("══════════════════════════════════════════════════════════════════════════════════════");
    println!(" LANE FIABLE (WS1) — sweep N={sweep:?} · budget≈{budget} · router_cap={ROUTER_CAP} (minuscule, hostile)");
    if cfg!(debug_assertions) {
        println!(" ⚠ build DEBUG — relancez avec --release.");
    }
    println!("══════════════════════════════════════════════════════════════════════════════════════");
    println!(
        " {:>5} {:>6} {:>10} {:>11} {:>11} {:>11} {:>8} {:>10} {:>7}",
        "N", "per", "produced", "notif_pub", "notif_drop", "delivered", "loss%", "events/s", "p99ms"
    );
    println!(" {}", "─".repeat(96));

    let mut all_zero_loss = true;
    for &n in &sweep {
        let r = trial(n, budget).await;
        let loss = if r.produced > 0 {
            100.0 * (r.produced.saturating_sub(r.delivered)) as f64 / r.produced as f64
        } else {
            0.0
        };
        if r.delivered != r.produced {
            all_zero_loss = false;
        }
        println!(
            " {:>5} {:>6} {:>10} {:>11} {:>11} {:>11} {:>7.2}% {:>10.0} {:>7.2}",
            r.n, r.per, r.produced, r.notif_published, r.notif_dropped, r.delivered, loss, r.events_per_s, r.p99
        );
    }
    println!(" {}", "─".repeat(96));

    println!("\n────────────────────────── VERDICT WS1 ──────────────────────────");
    println!(" notifs MASSIVEMENT perdues (router_cap={ROUTER_CAP}) — pourtant :");
    if all_zero_loss {
        println!("   livraison fiable : 0 % de perte ✅  (la baseline best-effort perdait ~99 %)");
        println!("   → keystone validé : bus = indice (perdu), Blackboard = vérité (réconciliée).");
        println!("   → le SLO « perte best-effort » de la Phase 1 est RÉSOLU par la lane reliable.");
    } else {
        println!("   ❌ perte détectée — revoir la réconciliation.");
    }
    println!("══════════════════════════════════════════════════════════════════");
}
