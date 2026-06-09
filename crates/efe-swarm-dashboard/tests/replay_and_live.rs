// SPDX-License-Identifier: UNLICENSED
//! Test d'intégration (API publique) : **rejeu** (run terminé) + **live** (tail en cours).
//!
//! Vérifie le contrat clé du dashboard via un *vrai* run keyless observé :
//! - un abonné **tardif** rejoue la séquence complète, ordonnée, sans trou (`seq` monotone) ;
//! - un abonné **précoce** reçoit le tail live jusqu'au terminal.

use std::sync::Arc;
use std::time::Duration;

use efe_nano_sdk::TenantId;
use efe_swarm_dashboard::{DashboardBus, DashboardEvent, DashboardObserver};
use efe_swarm_sdk::{Budget, Swarm, TopologySpec};
use serde_json::json;
use uuid::Uuid;

fn observed_swarm(bus: Arc<DashboardBus>) -> Swarm {
    Swarm::builder()
        .with_observer(Arc::new(DashboardObserver::new(bus)))
        .build()
        .unwrap()
}

fn consensus() -> TopologySpec {
    TopologySpec::consensus("echo_generator", &["consistency", "length_band", "j_non_empty"], 5)
}

/// Abonné **tardif** : le run est fini avant la connexion → le ring rejoue tout, ordonné.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replay_after_finish_is_ordered_and_complete() {
    let bus = DashboardBus::new(TenantId::system(), Uuid::new_v4(), 8192);
    let swarm = observed_swarm(bus.clone());
    swarm
        .run(json!({ "text": "hello", "expected": "hello" }), consensus(), Budget::cents(1000))
        .await
        .unwrap();

    let replay = bus.replay();
    assert!(!replay.is_empty());
    // `seq` unique et **contigu** 0..n (aucun trou) — l'allocation est atomique. L'ORDRE du
    // ring suit l'émission (≈ concurrente) ; le client réordonne par `seq`. On vérifie donc
    // l'ENSEMBLE des seq, pas la position dans le ring.
    let mut seqs: Vec<u64> = replay.iter().map(|e| e.seq).collect();
    seqs.sort_unstable();
    for (i, s) in seqs.iter().enumerate() {
        assert_eq!(*s, i as u64, "seq contigus 0..n (aucun trou)");
    }
    // run_started = seq min (émis avant toute concurrence) ; run_finished = seq max (émis après).
    let started = replay.iter().find(|e| e.seq == 0).unwrap();
    assert_eq!(started.event.kind(), "run_started");
    let finished = replay.iter().max_by_key(|e| e.seq).unwrap();
    assert_eq!(finished.event.kind(), "run_finished");
    let kinds: Vec<&str> = replay.iter().map(|e| e.event.kind()).collect();
    for needed in ["candidate", "ballot", "decided", "sealed", "budget_tick", "agent_write"] {
        assert!(kinds.contains(&needed), "rejeu doit contenir {needed}");
    }
}

/// Abonné **précoce** : connecté avant l'exécution → reçoit le tail live jusqu'au terminal.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_subscriber_receives_tail_until_terminal() {
    let bus = DashboardBus::new(TenantId::system(), Uuid::new_v4(), 8192);
    let mut rx = bus.subscribe(); // s'abonne AVANT le run
    let swarm = observed_swarm(bus.clone());

    let handle = tokio::spawn(async move {
        swarm
            .run(json!({ "text": "x", "expected": "x" }), consensus(), Budget::cents(1000))
            .await
            .unwrap()
    });

    let mut kinds = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
            Ok(Ok(env)) => {
                let terminal = env.is_terminal();
                kinds.push(env.event.kind().to_string());
                if terminal {
                    break;
                }
            }
            Ok(Err(_)) => break, // bus fermé / lagged
            Err(_) => panic!("timeout : le run live aurait dû émettre un terminal"),
        }
    }
    let run = handle.await.unwrap();
    assert!(run.provenance.verified);
    assert_eq!(kinds.first().map(String::as_str), Some("run_started"));
    assert_eq!(kinds.last().map(String::as_str), Some("run_finished"));
    assert!(kinds.iter().any(|k| k == "ballot"), "ballots vus en direct");
}

/// La sortie `Decided` porte un pattern + une confiance réels (pas inventés).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn decided_carries_real_pattern_and_confidence() {
    let bus = DashboardBus::new(TenantId::system(), Uuid::new_v4(), 8192);
    observed_swarm(bus.clone())
        .run(json!({ "text": "hi", "expected": "hi" }), consensus(), Budget::cents(1000))
        .await
        .unwrap();
    let decided = bus.replay().into_iter().find_map(|e| match e.event {
        DashboardEvent::Decided { pattern, confidence, .. } => Some((pattern, confidence)),
        _ => None,
    });
    let (pattern, confidence) = decided.expect("verdict émis");
    assert!(!pattern.is_empty());
    assert!((0.0..=1.0).contains(&confidence));
}
