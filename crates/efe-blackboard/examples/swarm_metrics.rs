// SPDX-License-Identifier: UNLICENSED
//! # `swarm_metrics` — observabilité live du substrat (Phase 1 · WS5).
//!
//! Démontre, sur un mini-essaim réel, le câblage WS3+WS5 :
//! - `Blackboard::set_metrics` + `EventBus::with_metrics` → un seul
//!   [`SubstrateMetrics`] partagé ;
//! - les agents écrivent au Blackboard (compteurs O(1) + ring borné + sink) et
//!   publient sur le bus ; un collecteur consomme → la **latence de propagation
//!   inter-agent** (publish → consume) est mesurée ;
//! - sortie finale : **format Prometheus** + p50/p90/p99 de propagation.
//!
//! Run :
//! ```text
//! cargo run --example swarm_metrics -p efe-blackboard            # 16 agents × 50
//! cargo run --example swarm_metrics -p efe-blackboard -- 64 100
//! ```

use std::sync::Arc;
use std::time::SystemTime;

use efe_blackboard::{Blackboard, CountingSink, EventBus, SubstrateMetrics, SwarmEvent};
use efe_nano_sdk::{AgentId, SwarmId};
use serde_json::json;

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(16);
    let per: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(50);

    let swarm_id = SwarmId::new();
    let metrics = SubstrateMetrics::new();

    // WS3+WS5 : un seul registre partagé entre Blackboard et EventBus.
    let bb = Arc::new(Blackboard::with_recent_cap(swarm_id, 8_000_000, 256));
    bb.set_metrics(metrics.clone());
    let sink = Arc::new(CountingSink::new());
    bb.set_write_sink(sink.clone());
    let bus = Arc::new(EventBus::with_metrics((n * per + 16).max(256), metrics.clone()));

    println!("══════════════════════════════════════════════════════════════");
    println!(" SUBSTRAT OBSERVABLE — {n} agents × {per} écritures · swarm_id={swarm_id}");
    println!("══════════════════════════════════════════════════════════════");

    // Collecteur : s'abonne AVANT les agents ; consomme → mesure la propagation.
    let total_events = n * per;
    let collector = {
        let mut sub = bus.subscribe();
        let metrics = metrics.clone();
        tokio::spawn(async move {
            let mut seen = 0usize;
            loop {
                match sub.recv().await {
                    Some(ev) if ev.topic == "done" => break,
                    Some(_) => {
                        seen += 1;
                        metrics.inflight_add(-1);
                    }
                    None => break,
                }
            }
            seen
        })
    };

    // Agents : écrivent au Blackboard + publient une notification sur le bus.
    let mut agents = Vec::new();
    for a in 0..n {
        let bb = bb.clone();
        let bus = bus.clone();
        let metrics = metrics.clone();
        let id = AgentId::new(format!("agent_{a:03}")).unwrap();
        agents.push(tokio::spawn(async move {
            metrics.agents_inc();
            for i in 0..per {
                let _ = bb.write(&format!("/work/{a}/{i}"), json!(i), id.clone());
                metrics.inflight_add(1);
                let _ = bus.publish(SwarmEvent {
                    swarm_id,
                    published_by: id.clone(),
                    topic: "work".into(),
                    payload: json!({ "a": a, "i": i }),
                    published_at: SystemTime::now(),
                });
            }
            metrics.agents_dec();
        }));
    }
    for a in agents {
        let _ = a.await;
    }

    // Sentinelle de fin pour le collecteur.
    let _ = bus.publish(SwarmEvent {
        swarm_id,
        published_by: AgentId::new("coordinator").unwrap(),
        topic: "done".into(),
        payload: json!({}),
        published_at: SystemTime::now(),
    });
    let consumed = collector.await.unwrap_or(0);

    // ── Rapport ────────────────────────────────────────────────────────────────
    let s = metrics.snapshot();
    println!("\n── Vérité de coordination ──");
    println!(" écritures Blackboard   : {} (exact)", s.bb_writes_total);
    println!(" sink (historique complet): {} · ring borné à {}", sink.count(), bb.recent_len());
    println!(" événements publiés     : {} · consommés : {consumed}", s.ev_published_total);
    println!(" événements perdus (lag): {}", s.ev_dropped_total);
    println!("\n── Latence de propagation inter-agent (publish → consume) ──");
    println!(" échantillons : {}", s.propagation_count);
    println!(" moyenne : {:.3} ms · p50 : {:.3} ms · p90 : {:.3} ms · p99 : {:.3} ms",
             s.propagation_mean_ms, s.propagation_p50_ms, s.propagation_p90_ms, s.propagation_p99_ms);
    println!("\n── Exposition Prometheus ──");
    print!("{}", metrics.render_prometheus());
    println!("══════════════════════════════════════════════════════════════");
    println!("(total attendu d'événements de travail : {total_events})");
}
