// SPDX-License-Identifier: UNLICENSED
//! # `swarm_load` — baseline de charge du substrat (Phase 1, gate 85 %→95 %).
//!
//! Balaie N = 10 → 1000 agents et met le substrat durci (WS3+WS5) sous charge :
//! chaque agent écrit au Blackboard + publie sur l'EventBus (le « passage d'infos
//! en parallèle »), pendant que quelques collecteurs consomment (→ latence de
//! propagation mesurée). Une sonde de contention CAS (claim disputé) révèle F6.
//!
//! Sortie : la **table SLO réelle** par N + un verdict vs les cibles Phase 1 :
//! - throughput ≥ 100 k events/s @ N=1000
//! - latence de propagation p99 < 5 ms @ N=1000
//! - perte best-effort **bornée et observée** (la lane *reliable* = WS1, pas encore)
//! - mémoire **bornée** (ring constant quel que soit N)
//! - scaling quasi-linéaire ; sinon on PLAFONNE honnêtement le claim « grande quantité ».
//!
//! Run :
//! ```text
//! cargo run --release --example swarm_load -p efe-blackboard
//! cargo run --release --example swarm_load -p efe-blackboard -- 10,100,1000 200000
//! ```
//! (⚠ utilisez `--release` : une baseline en debug sous-estime le throughput.)

use std::sync::Arc;
use std::time::{Instant, SystemTime};

use efe_blackboard::{Blackboard, EventBus, SubstrateMetrics, SwarmEvent};
use efe_nano_sdk::{AgentId, SwarmId};
use serde_json::json;

const RING_CAP: usize = 1024; // historique récent borné — constant ⇒ mémoire bornée
const BUS_CAP: usize = 8192; // capacité broadcast (best-effort)
const COLLECTORS: usize = 4; // abonnés réalistes (≠ full-mesh O(N²), qui est l'enjeu WS1)
const CONTENDED: usize = 16; // items disputés pour la sonde de contention CAS

struct Trial {
    n: usize,
    per: usize,
    total_events: u64,
    wall_ms: f64,
    events_per_s: f64,
    writes_per_s: f64,
    p50: f64,
    p90: f64,
    p99: f64,
    drops: u64,
    cas_attempts: u64,
    cas_conflicts: u64,
    ring_len: usize,
}

async fn run_trial(n: usize, budget: usize) -> Trial {
    let per = (budget / n).max(1);
    let total_events = (per * n) as u64;
    let cas_attempts = (CONTENDED * n) as u64;

    let swarm_id = SwarmId::new();
    let metrics = SubstrateMetrics::new();
    let bb = Arc::new(Blackboard::with_recent_cap(swarm_id, 256_000_000, RING_CAP));
    bb.set_metrics(metrics.clone());
    let bus = Arc::new(EventBus::with_metrics(BUS_CAP, metrics.clone()));

    // Collecteurs : s'abonnent AVANT les producteurs ; ne détiennent PAS d'Arc<bus>
    // → quand `bus` est droppé après la prod, le canal se ferme et ils terminent.
    let mut collectors = Vec::new();
    for _ in 0..COLLECTORS {
        let mut sub = bus.subscribe();
        collectors.push(tokio::spawn(async move {
            let mut seen = 0u64;
            while sub.recv().await.is_some() {
                seen += 1; // recv() enregistre la latence de propagation (WS5)
            }
            seen
        }));
    }

    // ── Production sous charge : N agents en parallèle ──────────────────────────
    let start = Instant::now();
    let mut agents = Vec::new();
    for a in 0..n {
        let bb = bb.clone();
        let bus = bus.clone();
        let id = AgentId::new(format!("agent_{a:05}")).unwrap();
        agents.push(tokio::spawn(async move {
            // écriture + publication (le cœur : passage d'infos en parallèle).
            for i in 0..per {
                let _ = bb.write(&format!("/w/{a}/{i}"), json!(i), id.clone());
                let _ = bus.publish(SwarmEvent {
                    swarm_id,
                    published_by: id.clone(),
                    topic: "work".into(),
                    payload: json!({ "a": a, "i": i }),
                    published_at: SystemTime::now(),
                });
            }
            // sonde de contention CAS : tous les agents se disputent CONTENDED items.
            for j in 0..CONTENDED {
                let _ = bb.write_if(&format!("/probe/{j}"), 0, json!(a), id.clone());
            }
        }));
    }
    for a in agents {
        let _ = a.await;
    }
    let wall = start.elapsed();
    let wall_ms = wall.as_secs_f64() * 1000.0;

    // Ferme le bus → les collecteurs drainent le reste puis terminent (None).
    drop(bus);
    let mut consumed = 0u64;
    for c in collectors {
        consumed += c.await.unwrap_or(0);
    }
    let _ = consumed;

    let s = metrics.snapshot();
    let secs = wall.as_secs_f64().max(1e-9);
    Trial {
        n,
        per,
        total_events,
        wall_ms,
        events_per_s: total_events as f64 / secs,
        writes_per_s: s.bb_writes_total as f64 / secs,
        p50: s.propagation_p50_ms,
        p90: s.propagation_p90_ms,
        p99: s.propagation_p99_ms,
        drops: s.ev_dropped_total,
        cas_attempts,
        cas_conflicts: s.bb_conflicts_total,
        ring_len: bb.recent_len(),
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let sweep: Vec<usize> = args
        .get(1)
        .map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect())
        .unwrap_or_else(|| vec![10, 50, 100, 250, 500, 1000]);
    let budget: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(100_000);

    println!("══════════════════════════════════════════════════════════════════════════════════════");
    println!(" BASELINE DE CHARGE DU SUBSTRAT — sweep N={sweep:?} · budget≈{budget} events/essai");
    println!(" ring={RING_CAP} · bus_cap={BUS_CAP} · collecteurs={COLLECTORS} · CAS disputés={CONTENDED}");
    if cfg!(debug_assertions) {
        println!(" ⚠ build DEBUG — relancez avec --release pour des chiffres de throughput valides.");
    }
    println!("══════════════════════════════════════════════════════════════════════════════════════");
    println!(
        " {:>5} {:>6} {:>10} {:>9} {:>12} {:>12} {:>7} {:>7} {:>7} {:>8} {:>9} {:>6}",
        "N", "per", "events", "wall_ms", "events/s", "writes/s", "p50ms", "p90ms", "p99ms", "drops", "CAS_conf%", "ring"
    );
    println!(" {}", "─".repeat(118));

    let mut trials = Vec::new();
    for &n in &sweep {
        let t = run_trial(n, budget).await;
        let cas_pct = if t.cas_attempts > 0 {
            100.0 * t.cas_conflicts as f64 / t.cas_attempts as f64
        } else {
            0.0
        };
        println!(
            " {:>5} {:>6} {:>10} {:>9.1} {:>12.0} {:>12.0} {:>7.2} {:>7.2} {:>7.2} {:>8} {:>8.1}% {:>6}",
            t.n, t.per, t.total_events, t.wall_ms, t.events_per_s, t.writes_per_s,
            t.p50, t.p90, t.p99, t.drops, cas_pct, t.ring_len
        );
        trials.push(t);
    }
    println!(" {}", "─".repeat(118));

    // ── Verdict SLO (sur le plus grand N du sweep) ─────────────────────────────
    let big = trials.last().expect("au moins un essai");
    let small = &trials[0];
    let thr_ok = big.events_per_s >= 100_000.0;
    let p99_ok = big.p99 < 5.0;
    let mem_ok = trials.iter().all(|t| t.ring_len <= RING_CAP);
    let scaling = big.events_per_s / small.events_per_s.max(1.0);

    println!("\n────────────────────────── VERDICT SLO (Phase 1) ──────────────────────────");
    println!(" @ N={} :", big.n);
    println!("   throughput ≥ 100k events/s : {:.0}/s  → {}", big.events_per_s, ok(thr_ok));
    println!("   latence propagation p99 < 5ms : {:.2} ms  → {}", big.p99, ok(p99_ok));
    println!("   mémoire bornée (ring const)   : {} ≤ {RING_CAP}  → {}", big.ring_len, ok(mem_ok));
    println!("   perte best-effort (observée)  : {} drops  (lane reliable = WS1, à venir)", big.drops);
    println!("   contention CAS (broadcast)    : {:.1}% conflits  → motive le sharded-claim (WS4)",
             100.0 * big.cas_conflicts as f64 / big.cas_attempts.max(1) as f64);
    println!("   scaling events/s (N{}→N{})   : ×{:.2}", small.n, big.n, scaling);
    println!("────────────────────────────────────────────────────────────────────────────");
    let verdict = if thr_ok && p99_ok && mem_ok {
        "GO — le substrat tient les SLO jusqu'à N=".to_string() + &big.n.to_string()
            + " ; reste WS1 (perte) + WS4 (contention) pour le 95 %."
    } else {
        format!(
            "PLAFONNER — un SLO casse à N={} (thr={}, p99={}, mem={}) → caper honnêtement le claim « grande quantité ».",
            big.n, ok(thr_ok), ok(p99_ok), ok(mem_ok)
        )
    };
    println!(" {verdict}");
    println!("════════════════════════════════════════════════════════════════════════════");
}

fn ok(b: bool) -> &'static str {
    if b {
        "✅"
    } else {
        "❌"
    }
}
