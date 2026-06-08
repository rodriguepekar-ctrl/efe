// SPDX-License-Identifier: UNLICENSED
//! Intégration **Phase 1 · WS2+WS6** — streaming + quiescence + déterminisme.
//!
//! Gèle :
//! - **WS2** : un pipeline multi-étages s'exécute SANS barrière et termine par
//!   QUIESCENCE (inflight→0), comptes équilibrés (produits==consommés) ;
//! - **WS6** : l'état final (clés distinctes) est REPRODUCTIBLE entre deux runs
//!   (`value_map` identique) et l'`EventLog` rejoue la séquence à l'identique.

use std::sync::Arc;
use std::time::Duration;

use efe_blackboard::{Blackboard, EventLog, StreamingSwarm, TopicRouter};
use efe_nano_sdk::{AgentId, SwarmId};
use serde_json::json;

async fn run_pipeline(items: i64, fanout: i64) -> std::collections::BTreeMap<String, serde_json::Value> {
    let bb = Arc::new(Blackboard::with_recent_cap(SwarmId::new(), 64_000_000, 4096));
    let router = Arc::new(TopicRouter::new(8192));
    let mut swarm = StreamingSwarm::new(bb.clone(), router);

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
    let report = swarm.run_until_quiescent(Duration::from_secs(10)).await;
    let expected = (items + items * fanout) as u64;
    assert!(report.quiesced, "quiescence non atteinte");
    assert_eq!(report.produced, expected, "produits == sources + enfants");
    assert_eq!(report.consumed, expected, "consommés == produits (équilibre crédit)");
    bb.snapshot().value_map()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn streaming_pipeline_reaches_quiescence() {
    let vm = run_pipeline(100, 4).await;
    let results = vm.keys().filter(|k| k.starts_with("/result/")).count();
    assert_eq!(results, 400, "100×4 résultats écrits");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn final_state_is_deterministic_across_runs() {
    // clés distinctes → l'état final ne dépend pas de l'ordre d'exécution.
    let a = run_pipeline(80, 5).await;
    let b = run_pipeline(80, 5).await;
    assert_eq!(a, b, "value_map identique entre deux runs → déterminisme WS6");
}

#[tokio::test]
async fn event_log_replays_in_order() {
    let log = EventLog::new();
    let router = Arc::new(TopicRouter::new(256));

    // abonné de journalisation sur "stream".
    let cap = {
        let sub = router.subscribe("stream");
        let log = log.clone();
        tokio::spawn(async move { log.capture(sub).await })
    };
    for i in 0..20 {
        router.publish(efe_blackboard::SwarmEvent {
            swarm_id: SwarmId::new(),
            published_by: AgentId::new("producer").unwrap(),
            topic: "stream".into(),
            payload: json!({ "i": i }),
            published_at: std::time::SystemTime::now(),
        });
    }
    drop(router);
    cap.await.unwrap();
    assert_eq!(log.len(), 20);

    // replay sur un nouveau routeur → même ordre.
    let router2 = TopicRouter::new(256);
    let mut sub2 = router2.subscribe("stream");
    assert_eq!(log.replay(&router2), 20);
    let mut got = Vec::new();
    while let Some(e) = sub2.try_recv() {
        got.push(e.payload["i"].as_i64().unwrap());
    }
    assert_eq!(got, (0..20).collect::<Vec<_>>(), "replay ordonné");
}
