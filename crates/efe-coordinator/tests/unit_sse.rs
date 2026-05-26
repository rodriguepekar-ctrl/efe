// SPDX-License-Identifier: UNLICENSED
//! Tests d'intégration `sse::*`.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use efe_coordinator::{
    parse_sse_blob, to_sse_blob, EventBus, InMemoryBus, MissionTag, SseByteStream,
    StreamEvent, CONTENT_TYPE_SSE, SSE_PREFACE,
};
use efe_nano_app_coordinator::types::router::TaskClass;
use futures::StreamExt;
use uuid::Uuid;

#[test]
fn constants_are_documented() {
    assert_eq!(CONTENT_TYPE_SSE, "text/event-stream");
    assert!(SSE_PREFACE.contains("efe-coordinator"));
}

#[test]
fn to_sse_blob_includes_preface_and_frames() {
    let s = Uuid::now_v7();
    let evs = vec![
        StreamEvent::SwarmStarted {
            swarm_id: s,
            mission: MissionTag::MissionA,
            user_text: "x".into(),
            task_class: TaskClass::FactualQa,
            ts: Utc::now(),
        },
        StreamEvent::Heartbeat { ts: Utc::now() },
    ];
    let blob = to_sse_blob(&evs);
    assert!(blob.starts_with(SSE_PREFACE));
    assert!(blob.contains("event: SWARM_STARTED"));
    assert!(blob.contains("event: HEARTBEAT"));
}

#[test]
fn parse_roundtrips_swarm_event() {
    let s = Uuid::now_v7();
    let ev = StreamEvent::SwarmStarted {
        swarm_id: s,
        mission: MissionTag::MissionA,
        user_text: "hi".into(),
        task_class: TaskClass::FactualQa,
        ts: Utc::now(),
    };
    let blob = to_sse_blob(&[ev.clone()]);
    let parsed = parse_sse_blob(&blob);
    assert_eq!(parsed.len(), 1);
    let back = parsed.into_iter().next().unwrap().unwrap();
    assert_eq!(back.swarm_id(), Some(s));
}

#[tokio::test]
async fn byte_stream_first_chunk_is_preface() {
    let bus = InMemoryBus::new();
    let (_t, rx) = bus.subscribe(Some("test"));
    let mut stream = SseByteStream::new(rx);
    let first = stream.next().await.unwrap().unwrap();
    let s = std::str::from_utf8(&first).unwrap();
    assert!(s.starts_with(":efe-coordinator SSE"));
}

#[tokio::test]
async fn byte_stream_forwards_subsequent_events() {
    let bus = Arc::new(InMemoryBus::new());
    let (_t, rx) = bus.subscribe(Some("test"));
    let mut stream = SseByteStream::new(rx);
    let _ = stream.next().await.unwrap().unwrap(); // préface
    let ev = StreamEvent::Heartbeat { ts: Utc::now() };
    bus.publish(ev).await.unwrap();
    let next = stream.next().await.unwrap().unwrap();
    let s = std::str::from_utf8(&next).unwrap();
    assert!(s.contains("event: HEARTBEAT"));
}

#[tokio::test]
async fn byte_stream_closes_when_bus_dropped() {
    let bus = Arc::new(InMemoryBus::new());
    let (_t, rx) = bus.subscribe(Some("test"));
    let mut stream = SseByteStream::new(rx);
    let _ = stream.next().await; // préface
    drop(bus);
    let end = tokio::time::timeout(Duration::from_millis(200), stream.next()).await;
    assert!(matches!(end, Ok(None)));
}

#[test]
fn parse_handles_blob_with_multiple_events() {
    let evs = vec![
        StreamEvent::Heartbeat { ts: Utc::now() },
        StreamEvent::Heartbeat { ts: Utc::now() },
        StreamEvent::Heartbeat { ts: Utc::now() },
    ];
    let blob = to_sse_blob(&evs);
    let parsed = parse_sse_blob(&blob);
    assert_eq!(parsed.len(), 3);
    for p in parsed {
        assert!(p.is_ok());
    }
}

#[test]
fn parse_skips_comment_and_retry_lines_gracefully() {
    let blob = ":this is a comment\nretry: 5000\n\ndata: {\"type\":\"HEARTBEAT\",\"ts\":\"2026-05-25T00:00:00Z\"}\n\n";
    let parsed = parse_sse_blob(blob);
    assert_eq!(parsed.len(), 1);
    assert!(parsed[0].is_ok());
}
