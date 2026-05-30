// SPDX-License-Identifier: UNLICENSED
//! **P4.S1** — Tests du flux d'événements live `run_mission_*_streamed`.
//!
//! Valident le contrat du bridge SSE (Phase 4) :
//! - **abonnement SANS COURSE** : `SwarmStarted` est TOUJOURS le 1er event capté
//!   (le helper s'abonne avant de spawn la mission) ;
//! - **corrélation** : le `swarm_id` retourné == celui de `SwarmStarted`, et
//!   tous les events du flux portent ce `swarm_id` (filtre `filter_by_swarm`) ;
//! - **injection** : `run_mission_a_with_swarm_id` honore l'`id` fourni ;
//! - **cleanup RAII** : dropper le `MissionStream` retire le subscriber du bus
//!   (pas de fuite — `InMemoryBus` ne prune pas les receivers fermés au publish) ;
//! - **backward-compat** : `run_mission_a` (wrapper) marche toujours.
//!
//! Offline : `MockLlmProvider` (aucune clé, aucun réseau).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::{
    filter_by_swarm, run_mission_a, run_mission_a_with_swarm_id, Coordinator, MissionARequest,
    MockLlmProvider, ProviderRegistry, StreamEvent,
};
use efe_nano_app_coordinator::types::router::TaskClass;
use efe_nano_app_coordinator::types::{Budget, LlmMetadata};
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────

fn meta() -> LlmMetadata {
    LlmMetadata {
        cost_input_per_million: 0,
        cost_output_per_million: 0,
        cost_input_per_million_microcents: 1_000,
        cost_output_per_million_microcents: 1_000,
        p50_latency_ms: 50,
        p99_latency_ms: 100,
        success_rate: 1.0,
        capability_scores: HashMap::new(),
        circuit_open: false,
    }
}

fn req(text: &str) -> MissionARequest {
    let mut metas = HashMap::new();
    metas.insert("mock".to_string(), meta());
    MissionARequest {
        user_text: text.to_string(),
        task_class: TaskClass::FactualQa,
        available_llms: vec!["mock".into()],
        llm_metadata: metas,
        budget: Budget {
            max_cost_cents: 0,
            max_cost_microcents: 1_000_000,
            max_latency_ms: 10_000,
            min_quality_score: 0.0,
        },
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(5),
        task_class_scoring_table: None,
        tenant_id: None,
    }
}

/// Coordinator naked (0 SLM) + 1 provider mock qui répond `"Paris"`.
fn coord() -> Arc<Coordinator> {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("mock").with_response("Paris")));
    Arc::new(Coordinator::builder().registry(reg).build().unwrap())
}

/// Draine le flux jusqu'à `SwarmFinished`/`SwarmFailed` (timeout borné anti-hang).
async fn drain(rx: &mut tokio::sync::mpsc::Receiver<StreamEvent>) -> Vec<StreamEvent> {
    let mut out = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
            Ok(Some(ev)) => {
                let done = matches!(
                    ev,
                    StreamEvent::SwarmFinished { .. } | StreamEvent::SwarmFailed { .. }
                );
                out.push(ev);
                if done {
                    break;
                }
            }
            Ok(None) => break, // canal fermé
            Err(_) => break,   // timeout
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────
// T2 — abonnement sans course + corrélation
// ─────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn streamed_swarm_started_is_always_first_no_race() {
    let c = coord();
    let mut stream = c.run_mission_a_streamed(req("Quelle est la capitale de la France ?"));
    let swarm_id = stream.swarm_id;

    let events = drain(&mut stream.events).await;
    assert!(!events.is_empty(), "le flux doit livrer des events");

    // H (zéro course) : le PREMIER event est SwarmStarted avec le swarm_id retourné.
    match &events[0] {
        StreamEvent::SwarmStarted { swarm_id: s, .. } => {
            assert_eq!(*s, swarm_id, "SwarmStarted doit porter le swarm_id retourné");
        }
        other => panic!("1er event doit être SwarmStarted, got {}", other.tag()),
    }

    // H (corrélation) : tous les events portent CE swarm_id (filtre effectif).
    for ev in &events {
        assert_eq!(
            ev.swarm_id(),
            Some(swarm_id),
            "event hors swarm dans un flux filtré: {}",
            ev.tag()
        );
    }

    // H : la mission se termine proprement.
    assert!(
        events
            .iter()
            .any(|e| matches!(e, StreamEvent::SwarmFinished { .. })),
        "SwarmFinished attendu"
    );

    // H : la réponse typée finale est disponible via le JoinHandle.
    let resp = stream.join.await.expect("join ok").expect("mission ok");
    assert_eq!(resp.final_text, "Paris");
}

// ─────────────────────────────────────────────────────────────────────────
// Injection : `_with_swarm_id` honore l'id fourni
// ─────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn with_swarm_id_injects_exact_id() {
    let c = coord();
    let injected = Uuid::now_v7();

    // On s'abonne AVANT de lancer, filtré sur l'id injecté.
    let (token, mut rx) = c.bus.subscribe_filtered(None, filter_by_swarm(injected));

    let resp = run_mission_a_with_swarm_id(&c, req("x"), injected)
        .await
        .expect("mission ok");
    assert_eq!(resp.final_text, "Paris");

    // Le 1er event sur ce canal filtré est SwarmStarted avec l'id injecté exact.
    let first = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("pas de timeout")
        .expect("au moins un event");
    assert!(
        matches!(first, StreamEvent::SwarmStarted { swarm_id, .. } if swarm_id == injected),
        "le swarm_id injecté doit être propagé à SwarmStarted"
    );

    c.bus.unsubscribe(&token);
}

// ─────────────────────────────────────────────────────────────────────────
// Cleanup RAII — pas de fuite de subscriber
// ─────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn dropping_mission_stream_unsubscribes_no_leak() {
    let c = coord();
    assert_eq!(c.bus.subscriber_count(), 0, "bus vide au départ");

    {
        let mut stream = c.run_mission_a_streamed(req("x"));
        assert_eq!(
            c.bus.subscriber_count(),
            1,
            "1 subscriber actif pendant la mission"
        );
        let _ = drain(&mut stream.events).await;
        let _ = stream.join.await;
    } // `stream` droppé ici → Drop → bus.unsubscribe

    assert_eq!(
        c.bus.subscriber_count(),
        0,
        "RAII: le subscriber doit être retiré au Drop (sinon fuite)"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Backward-compat — le wrapper `run_mission_a` marche toujours
// ─────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn wrapper_run_mission_a_still_works() {
    let c = coord();
    let resp = run_mission_a(&c, req("x")).await.expect("mission ok");
    assert_eq!(resp.final_text, "Paris");
}

// ─────────────────────────────────────────────────────────────────────────
// Deux missions concurrentes : tokens uniques, flux non croisés
// ─────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn concurrent_streams_do_not_cross_contaminate() {
    let c = coord();
    let mut s1 = c.run_mission_a_streamed(req("un"));
    let mut s2 = c.run_mission_a_streamed(req("deux"));
    assert_ne!(s1.swarm_id, s2.swarm_id, "deux swarm_id distincts");
    assert_eq!(c.bus.subscriber_count(), 2, "2 subscribers (tokens uniques)");

    let e1 = drain(&mut s1.events).await;
    let e2 = drain(&mut s2.events).await;

    for ev in &e1 {
        assert_eq!(ev.swarm_id(), Some(s1.swarm_id), "flux 1 pollué");
    }
    for ev in &e2 {
        assert_eq!(ev.swarm_id(), Some(s2.swarm_id), "flux 2 pollué");
    }

    let _ = s1.join.await;
    let _ = s2.join.await;
}

// ─────────────────────────────────────────────────────────────────────────
// T4 — consumer lent / non-consommé : ne bloque JAMAIS la mission
// ─────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn mission_completes_without_consuming_stream() {
    let c = coord();
    let mut stream = c.run_mission_a_streamed(req("x"));

    // On NE draine PAS le receiver. `publish` est non-bloquant (`try_send`) :
    // la mission doit se terminer quand même (pas de deadlock par back-pressure).
    let resp = stream
        .join
        .await
        .expect("join ok")
        .expect("mission ok malgré stream non-consommé");
    assert_eq!(resp.final_text, "Paris");

    // Les events ont été bufferisés (< cap canal) → récupérables a posteriori,
    // et aucun n'a été droppé sur une mission aussi courte.
    let mut count = 0;
    while stream.events.try_recv().is_ok() {
        count += 1;
    }
    assert!(count > 0, "events bufferisés malgré non-consommation");
}
