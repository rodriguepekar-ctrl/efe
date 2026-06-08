// SPDX-License-Identifier: UNLICENSED
//! Intégration **Phase 1 · WS1** — routage par topic + lane fiable at-least-once.
//!
//! Gèle la propriété que la baseline `swarm_load` avait montrée VIOLÉE par le bus
//! best-effort (~99 % de perte) : sous flot concurrent et notifs massivement
//! perdues, la lane fiable livre **100 %** (aucune perte), exactement une fois.

use std::sync::Arc;

use efe_blackboard::{Blackboard, ReliableTopic, SubstrateMetrics, TopicRouter};
use efe_nano_sdk::{AgentId, SwarmId};
use serde_json::json;

// ── WS1 : routage par topic (fan-out isolé) ──────────────────────────────────

#[tokio::test]
async fn topic_router_isolates_fanout() {
    let r = TopicRouter::new(32);
    let mut a = r.subscribe("alpha");
    let mut b = r.subscribe("beta");
    for _ in 0..5 {
        r.publish(efe_blackboard::SwarmEvent {
            swarm_id: SwarmId::new(),
            published_by: AgentId::new("prod").unwrap(),
            topic: "alpha".into(),
            payload: json!({}),
            published_at: std::time::SystemTime::now(),
        });
    }
    let mut got_a = 0;
    while a.try_recv().is_some() {
        got_a += 1;
    }
    assert_eq!(got_a, 5, "alpha reçoit ses 5 événements");
    assert!(b.try_recv().is_none(), "beta ne reçoit RIEN (pas le firehose)");
}

// ── WS1 : at-least-once sous flot concurrent malgré perte massive de notifs ───

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reliable_lane_zero_loss_under_hostile_drops() {
    let bb = Arc::new(Blackboard::with_recent_cap(SwarmId::new(), 64_000_000, 512));
    let metrics = SubstrateMetrics::new();
    bb.set_metrics(metrics.clone());
    // capacité minuscule → notifs massivement perdues (conditions hostiles).
    let router = Arc::new(TopicRouter::with_metrics(8, metrics.clone()));
    let topic = ReliableTopic::new("/rel/it", "it");

    // lecteur fiable concurrent.
    let reader_handle = {
        let mut reader = topic.reader(&router);
        let bb = bb.clone();
        tokio::spawn(async move {
            let mut seqs = Vec::new();
            while let Some(batch) = reader.recv_batch(&bb).await {
                for it in batch {
                    seqs.push(it.seq);
                }
            }
            seqs
        })
    };

    let n = 12u64;
    let per = 250u64;
    let mut producers = Vec::new();
    for p in 0..n {
        let bb = bb.clone();
        let router = router.clone();
        let topic = topic.clone();
        let id = AgentId::new(format!("prod_{p}")).unwrap();
        producers.push(tokio::spawn(async move {
            for i in 0..per {
                topic.publish(&router, &bb, &id, json!({ "i": i })).unwrap();
            }
        }));
    }
    for p in producers {
        p.await.unwrap();
    }
    drop(router); // ferme → le lecteur draine la queue puis termine.

    let mut seqs = reader_handle.await.unwrap();
    let produced = n * per;

    // 1. aucune perte : tous les items livrés.
    assert_eq!(seqs.len() as u64, produced, "at-least-once : 0 perte");
    // 2. exactly-once : seqs uniques et contigus 1..=produced.
    seqs.sort_unstable();
    seqs.dedup();
    assert_eq!(seqs.len() as u64, produced, "aucun doublon");
    assert_eq!(*seqs.first().unwrap(), 1);
    assert_eq!(*seqs.last().unwrap(), produced);

    // 3. preuve que les conditions étaient bien hostiles : des notifs ONT été perdues.
    let s = metrics.snapshot();
    assert!(
        s.ev_dropped_total > 0,
        "le test doit être hostile (notifs perdues) ; drops={}",
        s.ev_dropped_total
    );
}
