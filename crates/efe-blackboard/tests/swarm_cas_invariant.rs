// SPDX-License-Identifier: UNLICENSED
//! # Invariant de coordination de l'essaim (Phase 0 — Vérité & socle).
//!
//! Gèle la propriété FONDAMENTALE du work-stealing lock-free démontré par les
//! exemples `swarm_relay` / `swarm_slm` : quand R consommateurs se disputent N
//! items via `Blackboard::write_if` (CAS) sous diffusion `EventBus`,
//!
//!   * chaque item est gagné par **exactement un** consommateur ;
//!   * `total_gagnés == N` ;
//!   * `total_gagnés + total_perdus == N × R` (chaque consommateur tente chaque item) ;
//!   * chaque cellule `/claim/<i>` finit en **version 1** (un seul vainqueur).
//!
//! C'est la signature mathématique d'une coordination stigmergique correcte,
//! sans verrou global ni scheduler. Si ce test casse, l'essaim n'est plus sûr.

use std::sync::Arc;
use std::time::SystemTime;

use efe_blackboard::{Blackboard, BlackboardError, EventBus, SwarmEvent};
use efe_nano_sdk::{AgentId, SwarmId};
use serde_json::json;

fn ev(swarm_id: SwarmId, by: &AgentId, topic: &str, payload: serde_json::Value) -> SwarmEvent {
    SwarmEvent {
        swarm_id,
        published_by: by.clone(),
        topic: topic.to_string(),
        payload,
        published_at: SystemTime::now(),
    }
}

/// Lance un essaim work-stealing (N items, R consommateurs) et renvoie
/// `(total_gagnés, total_perdus, vainqueurs_distincts)`.
async fn run_swarm(n_items: usize, n_consumers: usize) -> (u64, u64, usize) {
    let swarm_id = SwarmId::new();
    let bb = Arc::new(Blackboard::new(swarm_id));
    // Capacité large : aucun consommateur ne doit rater un event (pas de lag).
    let bus = Arc::new(EventBus::with_capacity((n_items * 2).max(256)));

    // Tous les consommateurs s'abonnent AVANT toute publication (le broadcast
    // ne rejoue pas l'historique).
    let mut consumers = Vec::new();
    for c in 0..n_consumers {
        let mut sub = bus.subscribe();
        let bb = bb.clone();
        let cid = AgentId::new(format!("consumer_{c}")).unwrap();
        consumers.push(tokio::spawn(async move {
            let mut won = 0u64;
            let mut lost = 0u64;
            let mut seen = 0usize;
            // Chaque consommateur reçoit exactement N events "item" puis s'arrête.
            while seen < n_items {
                let Some(e) = sub.recv().await else { break };
                if e.topic != "item" {
                    continue;
                }
                seen += 1;
                let i = e.payload.get("i").and_then(|v| v.as_u64()).unwrap_or(0);
                // CAS : premier arrivé gagne /claim/<i> ; les autres → Conflict.
                match bb.write_if(&format!("/claim/{i}"), 0, json!(cid.as_str()), cid.clone()) {
                    Ok(_) => won += 1,
                    Err(BlackboardError::Conflict { .. }) => lost += 1,
                    Err(other) => panic!("erreur CAS inattendue : {other:?}"),
                }
            }
            (won, lost)
        }));
    }

    // Producteur : publie N events "item".
    let producer = AgentId::new("producer").unwrap();
    for i in 0..n_items {
        let _ = bus.publish(ev(swarm_id, &producer, "item", json!({ "i": i })));
    }

    let mut total_won = 0u64;
    let mut total_lost = 0u64;
    for c in consumers {
        let (w, l) = c.await.unwrap();
        total_won += w;
        total_lost += l;
    }

    // Vainqueurs distincts : nombre de cellules /claim/* effectivement écrites.
    let winners = bb
        .snapshot()
        .fields
        .keys()
        .filter(|k| k.starts_with("/claim/"))
        .count();

    // Chaque cellule revendiquée doit être en version 1 (un seul vainqueur).
    for i in 0..n_items {
        if let Some(v) = bb.read(&format!("/claim/{i}")) {
            assert_eq!(v.version, 1, "/claim/{i} devrait être en version 1 (un seul vainqueur)");
        }
    }

    (total_won, total_lost, winners)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn cas_invariant_holds_under_parallel_contention() {
    let n_items = 200usize;
    let n_consumers = 8usize;
    let (won, lost, winners) = run_swarm(n_items, n_consumers).await;

    // 1. Chaque item gagné exactement une fois.
    assert_eq!(won, n_items as u64, "chaque item doit avoir exactement un vainqueur");
    // 2. Vainqueurs distincts == items (aucun item non revendiqué, aucun double).
    assert_eq!(winners, n_items, "il doit y avoir N cellules /claim revendiquées");
    // 3. L'INVARIANT central : gagnés + perdus == N × R.
    assert_eq!(
        won + lost,
        (n_items * n_consumers) as u64,
        "gagnés + perdus doit égaler N×R (chaque consommateur tente chaque item)"
    );
    // 4. Corollaire : perdus == N × (R-1).
    assert_eq!(lost, (n_items * (n_consumers - 1)) as u64);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cas_invariant_scales_across_sizes() {
    // L'invariant tient quelle que soit l'échelle (petit, moyen, déséquilibré).
    for (n, r) in [(1usize, 4usize), (10, 2), (64, 16), (300, 6)] {
        let (won, lost, winners) = run_swarm(n, r).await;
        assert_eq!(won, n as u64, "N={n} R={r} : gagnés==N");
        assert_eq!(winners, n, "N={n} R={r} : vainqueurs distincts==N");
        assert_eq!(
            won + lost,
            (n * r) as u64,
            "N={n} R={r} : gagnés+perdus==N×R"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn single_consumer_wins_everything_zero_contention() {
    // Cas dégénéré : R=1 → aucune course perdue, tout gagné.
    let (won, lost, winners) = run_swarm(50, 1).await;
    assert_eq!(won, 50);
    assert_eq!(lost, 0, "R=1 : aucune contention, zéro perte CAS");
    assert_eq!(winners, 50);
}
