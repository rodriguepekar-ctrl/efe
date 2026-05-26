// SPDX-License-Identifier: UNLICENSED
//! Tests dédiés au protocole SSE (`StreamEvent`).

use chrono::Utc;
use efe_coordinator::{MissionTag, StreamEvent, EVENT_SCHEMA_VERSION};
use uuid::Uuid;

#[test]
fn schema_version_is_stable_v1() {
    // Cas de canari : si quelqu'un bump par accident, faire failer.
    assert_eq!(EVENT_SCHEMA_VERSION, "1.0.0");
}

#[test]
fn all_swarm_events_carry_swarm_id() {
    let s = Uuid::now_v7();
    let evs = vec![
        StreamEvent::SwarmStarted {
            swarm_id: s,
            mission: MissionTag::MissionA,
            user_text: "x".into(),
            task_class: efe_nano_app_coordinator::types::router::TaskClass::FactualQa,
            ts: Utc::now(),
        },
        StreamEvent::RobotStarted {
            swarm_id: s,
            robot_id: "a".into(),
            ts: Utc::now(),
        },
        StreamEvent::SwarmFinished {
            swarm_id: s,
            mission: MissionTag::MissionA,
            latency_ms: 0,
            audit_chain_hash: "0".repeat(64),
            ts: Utc::now(),
        },
    ];
    for ev in evs {
        assert_eq!(ev.swarm_id(), Some(s));
    }
    let hb = StreamEvent::Heartbeat { ts: Utc::now() };
    assert_eq!(hb.swarm_id(), None);
}

#[test]
fn sse_frame_format_is_text_event_stream_compliant() {
    let ev = StreamEvent::Heartbeat { ts: Utc::now() };
    let frame = ev.to_sse_frame();
    // Sépare strictement par "\n\n" comme requis par spec SSE.
    assert!(frame.ends_with("\n\n"));
    // Possède exactement 1 ligne "event:" et 1 ligne "data:" par évènement.
    let lines: Vec<_> = frame.split('\n').collect();
    let nb_event = lines.iter().filter(|l| l.starts_with("event:")).count();
    let nb_data = lines.iter().filter(|l| l.starts_with("data:")).count();
    assert_eq!(nb_event, 1);
    assert_eq!(nb_data, 1);
}

#[test]
fn tag_string_is_kebab_uppercase() {
    let ev = StreamEvent::Heartbeat { ts: Utc::now() };
    assert_eq!(ev.tag(), "HEARTBEAT");
    assert!(ev.tag().chars().all(|c| c.is_ascii_uppercase() || c == '_'));
}

#[test]
fn mission_tag_serializes_screaming() {
    let s = serde_json::to_string(&MissionTag::Speculative).unwrap();
    assert_eq!(s, "\"SPECULATIVE\"");
}

#[test]
fn vote_resolved_carries_per_provider_scores() {
    let mut m = std::collections::HashMap::new();
    m.insert("claude".to_string(), 0.7);
    m.insert("gpt4".to_string(), 0.3);
    let ev = StreamEvent::VoteResolved {
        swarm_id: Uuid::now_v7(),
        winner: "claude".into(),
        winner_score: 0.7,
        per_llm_scores: m.clone(),
        ts: Utc::now(),
    };
    let s = serde_json::to_string(&ev).unwrap();
    let back: StreamEvent = serde_json::from_str(&s).unwrap();
    match back {
        StreamEvent::VoteResolved {
            per_llm_scores, ..
        } => {
            assert_eq!(per_llm_scores.len(), 2);
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn verdict_severity_string_is_uppercase() {
    let ev = StreamEvent::VerdictReady {
        swarm_id: Uuid::now_v7(),
        claim: "x".into(),
        verdict: "SUPPORTED".into(),
        severity: "WARN".into(),
        best_source_id: Some("src-1".into()),
        similarity: 0.8,
        ts: Utc::now(),
    };
    let s = ev.to_sse_frame();
    assert!(s.contains("\"verdict\":\"SUPPORTED\""));
    assert!(s.contains("\"severity\":\"WARN\""));
}

#[test]
fn streamed_token_serialization_compact() {
    let ev = StreamEvent::LlmTokenStreamed {
        swarm_id: Uuid::now_v7(),
        provider: "claude".into(),
        token: "Hello".into(),
        index: 0,
        ts: Utc::now(),
    };
    let json = serde_json::to_string(&ev).unwrap();
    // SSE doit rester compact (≤ 500 chars pour un seul token).
    assert!(json.len() < 500, "json too long: {}", json.len());
}
