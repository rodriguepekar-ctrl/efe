// SPDX-License-Identifier: UNLICENSED
//! Intégration **Phase 1 · WS3+WS5** — durcissement du substrat.
//!
//! Couvre : historique borné mais compteurs EXACTS, sink reçoit tout, observers
//! câblés, caps par namespace + éviction/epoch, métriques (écritures, conflits,
//! rejets, octets) et latence de propagation inter-agent du bus.

use std::sync::Arc;
use std::time::SystemTime;

use efe_blackboard::observer::CapturingObserver;
use efe_blackboard::{
    Blackboard, CountingSink, EventBus, SubstrateMetrics, SwarmEvent,
};
use efe_nano_sdk::{AgentId, SwarmId};
use serde_json::json;

fn agent(s: &str) -> AgentId {
    AgentId::new(s).unwrap()
}

// ── WS3 : historique borné, compteurs exacts ─────────────────────────────────

#[test]
fn recent_history_bounded_but_counters_exact() {
    // ring borné à 16, mais 100 écritures → compteurs exacts, ring borné.
    let bb = Blackboard::with_recent_cap(SwarmId::new(), 1_000_000, 16);
    for i in 0..100 {
        bb.write(&format!("/k{i}"), json!(i), agent("writer")).unwrap();
    }
    assert_eq!(bb.write_count(), 100, "compteur global EXACT, jamais borné");
    assert_eq!(bb.writes_by_agent(&agent("writer")), 100, "par-agent EXACT");
    assert!(bb.recent_len() <= 16, "ring borné");
    assert!(bb.write_history().len() <= 16, "historique détaillé borné");
}

#[test]
fn writes_by_agent_is_exact_per_agent() {
    let bb = Blackboard::new(SwarmId::new());
    bb.write("/a", json!(1), agent("alpha")).unwrap();
    bb.write("/b", json!(2), agent("alpha")).unwrap();
    bb.write("/c", json!(3), agent("beta")).unwrap();
    assert_eq!(bb.writes_by_agent(&agent("alpha")), 2);
    assert_eq!(bb.writes_by_agent(&agent("beta")), 1);
    assert_eq!(bb.writes_by_agent(&agent("gamma")), 0);
}

// ── WS3 : sink reçoit TOUTES les écritures (même au-delà du ring) ────────────

#[test]
fn sink_receives_every_write_even_beyond_ring() {
    let bb = Blackboard::with_recent_cap(SwarmId::new(), 1_000_000, 8);
    let sink = Arc::new(CountingSink::new());
    bb.set_write_sink(sink.clone());
    for i in 0..50 {
        bb.write(&format!("/k{i}"), json!(i), agent("www")).unwrap();
    }
    assert_eq!(sink.count(), 50, "le sink voit TOUTES les écritures");
    assert!(sink.bytes() > 0);
    assert!(bb.recent_len() <= 8);
}

// ── WS5 : le hook observer est désormais CÂBLÉ ───────────────────────────────

#[test]
fn observer_fires_on_each_committed_write() {
    let bb = Blackboard::new(SwarmId::new());
    let obs = Arc::new(CapturingObserver::new());
    bb.register_observer(obs.clone());
    bb.write("/x", json!(1), agent("aaa")).unwrap();
    bb.write("/y", json!(2), agent("aaa")).unwrap();
    bb.write_if("/z", 0, json!(3), agent("aaa")).unwrap(); // câblé aussi côté CAS
    assert_eq!(obs.len(), 3, "observer appelé pour write ET write_if");
}

// ── WS5 : métriques (écritures, conflits, rejets, octets) ────────────────────

#[test]
fn metrics_capture_writes_conflicts_rejections() {
    let metrics = SubstrateMetrics::new();
    // cap global petit pour provoquer un rejet.
    let bb = Blackboard::with_cap(SwarmId::new(), 200);
    bb.set_metrics(metrics.clone());

    bb.write("/x", json!(1), agent("aaa")).unwrap(); // write ok
    // conflit CAS : /x est en version 1, on tente expected=0.
    assert!(bb.write_if("/x", 0, json!(2), agent("aaa")).is_err());
    // rejet de taille : valeur trop grosse pour le cap 200.
    assert!(bb.write("/big", json!("z".repeat(500)), agent("aaa")).is_err());

    let s = metrics.snapshot();
    assert_eq!(s.bb_writes_total, 1, "1 écriture committée");
    assert_eq!(s.bb_conflicts_total, 1, "1 conflit CAS");
    assert_eq!(s.bb_rejections_total, 1, "1 rejet de taille");
    assert!(s.bb_bytes > 0, "jauge octets peuplée");
    assert_eq!(bb.rejections(), 1);
}

// ── WS3 : caps par namespace + éviction / epoch ──────────────────────────────

#[test]
fn namespace_cap_enforced_then_eviction_frees_space() {
    // cap global large ; cap /cand/ = 100 octets.
    let bb = Blackboard::with_namespaces(
        SwarmId::new(),
        1_000_000,
        vec![("/cand/".to_string(), 100)],
    );
    // 1re écriture /cand/ ~62 octets : OK.
    bb.write("/cand/a", json!("x".repeat(60)), agent("aaa")).unwrap();
    assert!(bb.namespace_bytes("/cand/") > 0);
    // 2e écriture /cand/ : cumul > 100 → REJET (cap namespace).
    let err = bb.write("/cand/b", json!("y".repeat(60)), agent("aaa"));
    assert!(err.is_err(), "le cap namespace doit rejeter");
    // hors namespace : pas affecté par le cap /cand/.
    bb.write("/other", json!("z".repeat(60)), agent("aaa")).unwrap();

    // éviction du namespace transient → libère la place.
    bb.evict_namespace("/cand/");
    assert_eq!(bb.namespace_bytes("/cand/"), 0);
    assert!(!bb.contains("/cand/a"), "cellules /cand/ supprimées");
    assert!(bb.contains("/other"), "les autres namespaces survivent");
    // après éviction on peut réécrire dans /cand/.
    bb.write("/cand/c", json!("w".repeat(60)), agent("aaa")).unwrap();
}

#[test]
fn new_epoch_purges_transient_namespaces() {
    let bb = Blackboard::new(SwarmId::new());
    bb.write("/cand/1", json!(1), agent("aaa")).unwrap();
    bb.write("/claim/1", json!("x"), agent("aaa")).unwrap();
    bb.write("/keep/1", json!(1), agent("aaa")).unwrap();
    bb.new_epoch(&["/cand/", "/claim/"]);
    assert!(!bb.contains("/cand/1"));
    assert!(!bb.contains("/claim/1"));
    assert!(bb.contains("/keep/1"), "namespace non transient préservé");
}

// ── WS5 : latence de propagation inter-agent du bus ──────────────────────────

#[tokio::test]
async fn eventbus_records_propagation_and_publish_metrics() {
    let metrics = SubstrateMetrics::new();
    let bus = EventBus::with_metrics(64, metrics.clone());
    let mut sub = bus.subscribe();
    let ev = SwarmEvent {
        swarm_id: SwarmId::new(),
        published_by: agent("producer"),
        topic: "candidate".into(),
        payload: json!({"v": 1}),
        published_at: SystemTime::now(),
    };
    bus.publish(ev).unwrap();
    let got = sub.recv().await;
    assert!(got.is_some());

    let s = metrics.snapshot();
    assert!(s.ev_published_total >= 1, "publish compté");
    assert_eq!(s.propagation_count, 1, "latence de propagation enregistrée");
    assert!(s.propagation_p99_ms >= 0.0);
}

// ── WS3+WS5 : combiné sous parallélisme (substrat complet) ────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_substrate_under_parallel_load() {
    let metrics = SubstrateMetrics::new();
    let bb = Arc::new(Blackboard::with_recent_cap(SwarmId::new(), 4_000_000, 64));
    bb.set_metrics(metrics.clone());
    let sink = Arc::new(CountingSink::new());
    bb.set_write_sink(sink.clone());

    let n_agents = 16usize;
    let per = 50usize;
    let mut handles = Vec::new();
    for a in 0..n_agents {
        let bb = bb.clone();
        let id = agent(&format!("agent-{a}"));
        handles.push(tokio::spawn(async move {
            for i in 0..per {
                bb.write(&format!("/a{a}/i{i}"), json!(i), id.clone()).unwrap();
            }
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    let total = n_agents * per;
    assert_eq!(bb.write_count(), total, "compteur exact sous parallélisme");
    assert_eq!(sink.count(), total as u64, "sink exhaustif sous parallélisme");
    assert!(bb.recent_len() <= 64, "ring borné malgré la charge");
    assert_eq!(metrics.snapshot().bb_writes_total, total as u64);
}
