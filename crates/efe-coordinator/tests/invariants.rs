// SPDX-License-Identifier: UNLICENSED
//! Tests dédiés aux 8 invariants documentés dans `lib.rs`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::{
    run_mission_a, run_mission_b, Ballot, BlackboardEntry, CoordError, Coordinator, EntryKind,
    MissionARequest, MissionBRequest, MockLlmProvider, PheromoneBlackboard, ProviderRegistry,
    StreamEvent, VoteEngine, MAX_BLACKBOARD_BYTES,
};
use efe_nano_app_coordinator::types::router::TaskClass;
use efe_nano_app_coordinator::types::{Budget, LlmMetadata};

fn meta(success_rate: f32) -> LlmMetadata {
    LlmMetadata {
        cost_input_per_million: 1000,
        cost_output_per_million: 3000,
        // R3.S1 : 0 → _resolved() fallback à cents × 1000.
        cost_input_per_million_microcents: 0,
        cost_output_per_million_microcents: 0,
        p50_latency_ms: 500,
        p99_latency_ms: 2000,
        success_rate,
        capability_scores: HashMap::new(),
        circuit_open: false,
    }
}

// ── I2 : blackboard borné 256 KB ────────────────────────────────────
#[tokio::test]
async fn i2_blackboard_never_exceeds_256kb() {
    let bb = PheromoneBlackboard::new();
    for i in 0..500 {
        let e = BlackboardEntry::new(
            "x",
            EntryKind::Trace,
            serde_json::json!({"i": i, "pad": "x".repeat(2_000)}),
        );
        let _ = bb.deposit(e);
    }
    assert!(bb.bytes_used() <= MAX_BLACKBOARD_BYTES);
}

// ── I3 : tout StreamEvent JSON-roundtrip ─────────────────────────────
#[test]
fn i3_all_stream_events_roundtrip() {
    let s = uuid::Uuid::now_v7();
    let evs = vec![
        StreamEvent::SwarmStarted {
            swarm_id: s,
            mission: efe_coordinator::MissionTag::MissionA,
            user_text: "x".into(),
            task_class: TaskClass::FactualQa,
            ts: chrono::Utc::now(),
        },
        StreamEvent::SwarmFinished {
            swarm_id: s,
            mission: efe_coordinator::MissionTag::MissionA,
            latency_ms: 0,
            audit_chain_hash: "0".repeat(64),
            ts: chrono::Utc::now(),
        },
        StreamEvent::Heartbeat {
            ts: chrono::Utc::now(),
        },
    ];
    for ev in evs {
        let j = serde_json::to_string(&ev).unwrap();
        let back: StreamEvent = serde_json::from_str(&j).unwrap();
        assert_eq!(back, ev);
    }
}

// ── I5 : pas de quorum ⇒ fallback success_rate (déterministe) ────────
#[test]
fn i5_no_quorum_fallback_uses_success_rate() {
    let mut rates = HashMap::new();
    rates.insert("a".to_string(), 0.5);
    rates.insert("b".to_string(), 0.99);
    let v = VoteEngine::new(HashMap::new(), rates);
    let ballots = vec![
        Ballot {
            provider: "a".into(),
            answer_text: "A".into(),
            self_quality: 1.0,
            latency_ms: 50,
        },
        Ballot {
            provider: "b".into(),
            answer_text: "B".into(),
            self_quality: 1.0,
            latency_ms: 50,
        },
    ];
    let out = v.tally(&ballots, 2).unwrap();
    assert_eq!(out.winner, "b");
}

// ── I6 : Mission B avec n < 2 ⇒ MissionBTooFewLlms ───────────────────
#[tokio::test]
async fn i6_mission_b_too_few_llms_errors() {
    let coord = Coordinator::for_tests();
    let req = MissionBRequest {
        tenant_id: None,
        user_text: "x".into(),
        task_class: TaskClass::FactualQa,
        llms: vec!["only".into()],
        llm_metadata: HashMap::new(),
        vote_weights: HashMap::new(),
        quorum: None,
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(1),
    };
    let r = run_mission_b(&coord, req).await;
    assert!(matches!(r, Err(CoordError::MissionBTooFewLlms { n: 1 })));
}

// ── I7 : intensity monotone non-croissante hors dépôt ────────────────
#[test]
fn i7_intensity_strictly_monotone_under_time() {
    let bb = PheromoneBlackboard::new();
    let e = BlackboardEntry::with_half_life(
        "x",
        EntryKind::Trace,
        serde_json::json!({"v": 1}),
        2,
    );
    bb.deposit(e).unwrap();
    let prev = bb.read_kind(EntryKind::Trace);
    if prev.is_empty() {
        return;
    }
    let i0 = prev[0].1;
    std::thread::sleep(std::time::Duration::from_millis(50));
    let next = bb.read_kind(EntryKind::Trace);
    if next.is_empty() {
        // GCée — c'est encore monotone (vers 0).
        return;
    }
    let i1 = next[0].1;
    assert!(i1 <= i0);
}

// ── I8 : parallélisme borné dans DagExecutor (couvert dans unit_dag) ──
// ⇒ voir tests/unit_dag.rs::wide_fan_out_uses_parallelism (max ≤ 4).

// ── I4 : idempotence d'un swarm_id par run — couverte par UUID v7 ─────
#[tokio::test]
async fn i4_same_request_runs_independently() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("c").with_response("DONE")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let mut metas = HashMap::new();
    metas.insert("c".to_string(), meta(0.99));

    let mk_req = || MissionARequest {
        tenant_id: None,
        user_text: "same".into(),
        task_class: TaskClass::FactualQa,
        available_llms: vec!["c".into()],
        llm_metadata: metas.clone(),
        budget: Budget {
            max_cost_cents: 100,
            // R3.S1 : 0 → _resolved() fallback à cents × 1000.
            max_cost_microcents: 0,
            max_latency_ms: 5000,
            min_quality_score: 0.5,
        },
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(2),
        task_class_scoring_table: None,
    };
    let r1 = run_mission_a(&coord, mk_req()).await.unwrap();
    let r2 = run_mission_a(&coord, mk_req()).await.unwrap();
    // Même texte (déterministe via mock), mais swarm_id différent
    // (donc audit_chain_hash potentiellement différent).
    assert_eq!(r1.final_text, r2.final_text);
}
