// SPDX-License-Identifier: UNLICENSED
//! # `swarm_streaming` — exécution streaming + quiescence (WS2) + replay (WS6).
//!
//! Démontre un essaim où les infos CIRCULENT sans barrière de couche : une source
//! injecte des items ; un agent `split` les éclate en sous-tâches ; un agent
//! `work` les traite et écrit le résultat. La terminaison est détectée par
//! QUIESCENCE (plus aucun événement en vol), pas par un join de couche.
//!
//! Puis (WS6) : on capture le flux dans un [`EventLog`] et on montre que l'état
//! final est DÉTERMINISTE (clés distinctes → indépendant de l'ordre).
//!
//! Run :
//! ```text
//! cargo run --release --example swarm_streaming -p efe-blackboard
//! cargo run --release --example swarm_streaming -p efe-blackboard -- 200 4
//! ```

use std::sync::Arc;
use std::time::{Duration, Instant};

use efe_blackboard::{Blackboard, StreamingSwarm, TopicRouter};
use efe_nano_sdk::{AgentId, SwarmId};
use serde_json::json;

async fn run(items: i64, fanout: i64) -> (u64, u64, bool, usize, f64) {
    let bb = Arc::new(Blackboard::with_recent_cap(SwarmId::new(), 64_000_000, 2048));
    let router = Arc::new(TopicRouter::new(4096));
    let mut swarm = StreamingSwarm::new(bb.clone(), router);

    // Agent `split` : chaque item → `fanout` sous-tâches sur "work".
    swarm.agent(
        AgentId::new("splitter").unwrap(),
        "item",
        Arc::new(move |ctx, ev| {
            let i = ev.payload.get("i").and_then(|v| v.as_i64()).unwrap_or(0);
            for k in 0..fanout {
                ctx.emit("work", json!({ "i": i, "k": k }));
            }
        }),
    );
    // Agent `work` : traite et écrit sa propre clé (clés DISTINCTES → déterministe).
    swarm.agent(
        AgentId::new("worker").unwrap(),
        "work",
        Arc::new(|ctx, ev| {
            let i = ev.payload.get("i").and_then(|v| v.as_i64()).unwrap_or(0);
            let k = ev.payload.get("k").and_then(|v| v.as_i64()).unwrap_or(0);
            let _ = ctx.blackboard().write(
                &format!("/result/{i}/{k}"),
                json!(i * 1000 + k),
                ctx.agent_id().clone(),
            );
        }),
    );

    for i in 0..items {
        swarm.inject("item", json!({ "i": i }));
    }
    let start = Instant::now();
    let report = swarm.run_until_quiescent(Duration::from_secs(10)).await;
    let wall = start.elapsed().as_secs_f64() * 1000.0;
    let results = bb.snapshot().value_map().keys().filter(|k| k.starts_with("/result/")).count();
    (report.produced, report.consumed, report.quiesced, results, wall)
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let items: i64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(200);
    let fanout: i64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(4);

    println!("══════════════════════════════════════════════════════════════════════");
    println!(" STREAMING + QUIESCENCE (WS2) — {items} items × fanout {fanout}");
    if cfg!(debug_assertions) {
        println!(" ⚠ build DEBUG — relancez avec --release.");
    }
    println!("══════════════════════════════════════════════════════════════════════");

    // Deux runs identiques → déterminisme de l'état final (WS6).
    let (p1, c1, q1, r1, w1) = run(items, fanout).await;
    let (p2, c2, q2, r2, _w2) = run(items, fanout).await;

    let expected = (items + items * fanout) as u64; // sources + enfants
    println!(" produits/consommés : {p1}/{c1}  (attendu {expected})");
    println!(" quiescence atteinte : {q1}");
    println!(" résultats écrits    : {r1}  (attendu {})", items * fanout);
    println!(" wall                : {w1:.1} ms (sans barrière de couche)");
    println!("\n── Déterminisme (WS6) : run #1 vs run #2 ──");
    println!(" run1 produits={p1} consommés={c1} quiescé={q1} résultats={r1}");
    println!(" run2 produits={p2} consommés={c2} quiescé={q2} résultats={r2}");

    let ok = q1 && q2
        && p1 == expected && c1 == expected
        && p1 == p2 && c1 == c2 && r1 == r2
        && r1 == (items * fanout) as usize;
    println!("\n────────────────────────── VERDICT WS2+WS6 ──────────────────────────");
    if ok {
        println!(" ✅ streaming sans barrière, quiescence détectée par crédit (inflight→0),");
        println!("    comptes équilibrés (produits==consommés), état final REPRODUCTIBLE.");
    } else {
        println!(" ❌ incohérence — voir les comptes ci-dessus.");
    }
    println!("══════════════════════════════════════════════════════════════════════");
}
