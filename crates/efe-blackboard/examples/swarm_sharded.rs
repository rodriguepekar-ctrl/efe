// SPDX-License-Identifier: UNLICENSED
//! # `swarm_sharded` — WS4 ferme le ❌ CAS de la baseline.
//!
//! La baseline `swarm_load` a mesuré 90 %→99,9 % de CAS gaspillés (tout le monde
//! tente chaque item, O(N×items)). Ici on compare, à charge égale :
//! - **Broadcast** : O(N×items) tentatives, ~(R−1)/R de conflits ;
//! - **Sharded**   : O(items) tentatives, **0 conflit**, chaque item réclamé une fois.
//!
//! Run :
//! ```text
//! cargo run --release --example swarm_sharded -p efe-blackboard
//! cargo run --release --example swarm_sharded -p efe-blackboard -- 10,100,500 4000
//! ```

use std::sync::Arc;
use std::time::Instant;

use efe_blackboard::{Blackboard, ClaimModel, WorkQueue, WorkQueueStats};
use efe_nano_sdk::{AgentId, SwarmId};

async fn run(model: ClaimModel, workers: usize, items: u64) -> (WorkQueueStats, u64, f64) {
    let bb = Arc::new(Blackboard::with_recent_cap(SwarmId::new(), 256_000_000, 1024));
    let start = Instant::now();
    let mut handles = Vec::new();
    for w in 0..workers {
        let bb = bb.clone();
        let id = AgentId::new(format!("worker_{w:05}")).unwrap();
        handles.push(tokio::spawn(async move {
            let wq = WorkQueue::new(model, w, workers, "/claim");
            for item in 0..items {
                let _ = wq.claim(&bb, item, &id);
            }
            wq.stats()
        }));
    }
    let mut agg = WorkQueueStats::default();
    for h in handles {
        let s = h.await.unwrap();
        agg.attempts += s.attempts;
        agg.won += s.won;
        agg.conflicts += s.conflicts;
        agg.skipped_not_owner += s.skipped_not_owner;
    }
    let wall_ms = start.elapsed().as_secs_f64() * 1000.0;
    let claimed = (0..items).filter(|&i| bb.contains(&format!("/claim/{i}"))).count() as u64;
    (agg, claimed, wall_ms)
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let sweep: Vec<usize> = args
        .get(1)
        .map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect())
        .unwrap_or_else(|| vec![10, 100, 500]);
    let items: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(4000);

    println!("══════════════════════════════════════════════════════════════════════════════════════");
    println!(" WS4 — Broadcast vs Sharded · items={items} · sweep workers={sweep:?}");
    if cfg!(debug_assertions) {
        println!(" ⚠ build DEBUG — relancez avec --release.");
    }
    println!("══════════════════════════════════════════════════════════════════════════════════════");
    println!(
        " {:>6} {:>9} {:>11} {:>8} {:>9} {:>8} {:>9}",
        "model", "workers", "attempts", "won", "conflits", "conf%", "wall_ms"
    );
    println!(" {}", "─".repeat(78));

    let mut all_sharded_clean = true;
    for &w in &sweep {
        for model in [ClaimModel::Broadcast, ClaimModel::Sharded { stealing: false }] {
            let (a, claimed, wall) = run(model, w, items).await;
            assert_eq!(a.won, items, "chaque item réclamé une fois");
            assert_eq!(claimed, items);
            let pct = 100.0 * a.conflicts as f64 / a.attempts.max(1) as f64;
            let name = match model {
                ClaimModel::Broadcast => "bcast",
                ClaimModel::Sharded { .. } => "shard",
            };
            if matches!(model, ClaimModel::Sharded { .. }) && a.conflicts != 0 {
                all_sharded_clean = false;
            }
            println!(
                " {:>6} {:>9} {:>11} {:>8} {:>9} {:>7.1}% {:>9.1}",
                name, w, a.attempts, a.won, a.conflicts, pct, wall
            );
        }
        println!(" {}", "·".repeat(78));
    }

    println!("\n────────────────────────── VERDICT WS4 ──────────────────────────");
    println!(" Broadcast : attempts = items×workers, conflits → ~(R−1)/R (gaspillage).");
    println!(" Sharded   : attempts = items, conflits = 0  → {}", if all_sharded_clean { "✅" } else { "❌" });
    println!(" → la tempête CAS de la baseline (99,9 %) est SUPPRIMÉE par l'ownership shardé.");
    println!(" → composé à WS1 (route l'item vers le topic de son propriétaire), un worker");
    println!("   ne REÇOIT que ses items ET ne fait le CAS que sur eux : O(items) bout-en-bout.");
    println!("══════════════════════════════════════════════════════════════════");
}
