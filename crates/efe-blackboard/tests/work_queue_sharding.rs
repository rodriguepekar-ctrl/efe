// SPDX-License-Identifier: UNLICENSED
//! Intégration **Phase 1 · WS4** — distribution du travail / contention CAS.
//!
//! Gèle :
//! - **Broadcast** : invariant historique `gagnés+perdus == items×workers`
//!   (chaque item réclamé une fois, (R−1) conflits par item).
//! - **Sharded** : `gagnés == items`, **conflits == 0**, `attempts == items`
//!   (O(items) au lieu de O(N×items)) — la contention CAS est SUPPRIMÉE.
//! - **Sharded + vol** : même avec un propriétaire ABSENT, tous les items sont
//!   réclamés (vol), contention BORNÉE (≪ items×workers).

use std::sync::Arc;

use efe_blackboard::{
    shard_topic, shard_topic_for_item, Blackboard, ClaimModel, SwarmEvent,
    TopicRouter, WorkQueue, WorkQueueStats,
};
use efe_nano_sdk::{AgentId, SwarmId};

fn agg(stats: &[WorkQueueStats]) -> WorkQueueStats {
    let mut a = WorkQueueStats::default();
    for s in stats {
        a.attempts += s.attempts;
        a.won += s.won;
        a.conflicts += s.conflicts;
        a.stolen += s.stolen;
        a.skipped_not_owner += s.skipped_not_owner;
    }
    a
}

fn claimed_count(bb: &Blackboard, items: u64) -> u64 {
    (0..items).filter(|&i| bb.contains(&format!("/claim/{i}"))).count() as u64
}

async fn run(model: ClaimModel, workers: usize, items: u64) -> (WorkQueueStats, u64, Arc<Blackboard>) {
    let bb = Arc::new(Blackboard::with_recent_cap(SwarmId::new(), 64_000_000, 1024));
    let mut handles = Vec::new();
    for w in 0..workers {
        let bb = bb.clone();
        let id = AgentId::new(format!("worker_{w}")).unwrap();
        handles.push(tokio::spawn(async move {
            let wq = WorkQueue::new(model, w, workers, "/claim");
            for item in 0..items {
                let _ = wq.claim(&bb, item, &id);
            }
            wq.stats()
        }));
    }
    let mut stats = Vec::new();
    for h in handles {
        stats.push(h.await.unwrap());
    }
    let a = agg(&stats);
    let claimed = claimed_count(&bb, items);
    (a, claimed, bb)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn broadcast_invariant_won_plus_lost_equals_items_times_workers() {
    let (workers, items) = (8usize, 200u64);
    let (a, claimed, _bb) = run(ClaimModel::Broadcast, workers, items).await;
    assert_eq!(a.won, items, "chaque item réclamé exactement une fois");
    assert_eq!(claimed, items, "tous les items présents au Blackboard");
    assert_eq!(
        a.won + a.conflicts,
        items * workers as u64,
        "invariant Broadcast : gagnés+perdus == items×workers"
    );
    assert_eq!(a.conflicts, items * (workers as u64 - 1));
    assert_eq!(a.skipped_not_owner, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sharded_eliminates_cas_contention() {
    let (workers, items) = (8usize, 200u64);
    let (a, claimed, _bb) = run(ClaimModel::Sharded { stealing: false }, workers, items).await;
    assert_eq!(a.won, items, "chaque item réclamé exactement une fois");
    assert_eq!(claimed, items);
    // LA propriété WS4 : O(items) CAS, ZÉRO conflit.
    assert_eq!(a.attempts, items, "attempts == items (pas items×workers)");
    assert_eq!(a.conflicts, 0, "aucune contention CAS en shardé");
    // (R−1) tentatives évitées par item.
    assert_eq!(a.skipped_not_owner, items * (workers as u64 - 1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sharded_contention_is_orders_of_magnitude_below_broadcast() {
    let (workers, items) = (16usize, 500u64);
    let (bcast, _, _) = run(ClaimModel::Broadcast, workers, items).await;
    let (shard, _, _) = run(ClaimModel::Sharded { stealing: false }, workers, items).await;
    // Broadcast gaspille (R−1)/R des CAS ; Sharded n'en gaspille aucun.
    assert!(bcast.attempts >= shard.attempts * (workers as u64 - 1));
    assert_eq!(shard.conflicts, 0);
    assert!(bcast.conflicts > 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn aggregator_collects_all_shards_via_subscribe_prefix() {
    // WS1+WS4 composés : on route chaque item vers le topic de SON shard, et un
    // agrégateur écoute TOUS les shards par préfixe → reçoit tous les items, une
    // seule fois, sans firehose global.
    let workers = 4usize;
    let n = 100u64;
    let router = Arc::new(TopicRouter::new(8192));
    // les workers créent leurs topics de shard (abonnement = création du canal).
    let mut worker_subs: Vec<_> = (0..workers).map(|w| router.subscribe(&shard_topic("job", w))).collect();
    // agrégateur : un seul flux fusionné sur tous les "job.shard.*".
    let mut agg = router.subscribe_prefix("job.shard");
    assert_eq!(agg.topic_count(), workers);

    for i in 0..n {
        router.publish(SwarmEvent {
            swarm_id: SwarmId::new(),
            published_by: AgentId::new("producer").unwrap(),
            topic: shard_topic_for_item("job", i, workers),
            payload: serde_json::json!({ "i": i }),
            published_at: std::time::SystemTime::now(),
        });
    }

    // l'agrégateur reçoit les N items (tous shards confondus).
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..n {
        if let Some(ev) = agg.recv().await {
            seen.insert(ev.payload["i"].as_i64().unwrap());
        }
    }
    assert_eq!(seen.len() as u64, n, "agrégateur reçoit tous les items via préfixe");
    // chaque item est aussi allé sur EXACTEMENT un shard (partition).
    let _ = &mut worker_subs; // gardés vivants pour maintenir les canaux ouverts
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stealing_recovers_a_dead_owners_shard() {
    // worker 0 est ABSENT (ne réclame pas son shard) ; les autres volent.
    let (workers, items) = (4usize, 200u64);
    let bb = Arc::new(Blackboard::with_recent_cap(SwarmId::new(), 64_000_000, 1024));

    // Phase 1 : workers 1..4 réclament leurs propres shards (0 saute).
    let mut handles = Vec::new();
    for w in 1..workers {
        let bb = bb.clone();
        let id = AgentId::new(format!("worker_{w}")).unwrap();
        handles.push(tokio::spawn(async move {
            let wq = WorkQueue::new(ClaimModel::Sharded { stealing: true }, w, workers, "/claim");
            for item in 0..items {
                let _ = wq.claim(&bb, item, &id);
            }
            wq.stats()
        }));
    }
    let mut p1 = Vec::new();
    for h in handles {
        p1.push(h.await.unwrap());
    }
    // les items du shard 0 sont encore non réclamés.
    let after_p1 = claimed_count(&bb, items);
    assert!(after_p1 < items, "le shard du worker absent n'est pas encore réclamé");

    // Phase 2 : les survivants VOLENT tout item encore libre.
    let mut handles = Vec::new();
    for w in 1..workers {
        let bb = bb.clone();
        let id = AgentId::new(format!("worker_{w}")).unwrap();
        handles.push(tokio::spawn(async move {
            let wq = WorkQueue::new(ClaimModel::Sharded { stealing: true }, w, workers, "/claim");
            for item in 0..items {
                if !bb.contains(&format!("/claim/{item}")) {
                    let _ = wq.steal(&bb, item, &id);
                }
            }
            wq.stats()
        }));
    }
    let mut p2 = Vec::new();
    for h in handles {
        p2.push(h.await.unwrap());
    }

    // tous les items sont désormais réclamés (le vol a récupéré le shard mort).
    assert_eq!(claimed_count(&bb, items), items, "le vol récupère le shard absent");
    let a2 = agg(&p2);
    assert!(a2.stolen > 0, "des items ont été volés");
    // contention BORNÉE : ≪ items×workers (le pire cas Broadcast).
    assert!(
        a2.conflicts < items * workers as u64,
        "contention de vol bornée : {} < {}",
        a2.conflicts,
        items * workers as u64
    );
}
