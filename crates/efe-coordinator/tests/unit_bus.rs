// SPDX-License-Identifier: UNLICENSED
//! Tests d'intégration `InMemoryBus` + `StreamEvent`.

use chrono::Utc;
use efe_coordinator::{EventBus, InMemoryBus, MissionTag, StreamEvent, SubscriberToken};
use efe_nano_app_coordinator::types::router::TaskClass;
use uuid::Uuid;

fn ev(swarm: Uuid) -> StreamEvent {
    StreamEvent::SwarmStarted {
        swarm_id: swarm,
        mission: MissionTag::MissionA,
        user_text: "test".into(),
        task_class: TaskClass::FactualQa,
        ts: Utc::now(),
    }
}

#[tokio::test]
async fn publish_then_subscribe_misses_past_events() {
    let bus = InMemoryBus::new();
    let swarm = Uuid::now_v7();
    bus.publish(ev(swarm)).await.unwrap();
    let (_t, mut rx) = bus.subscribe(Some("late"));
    // Pas de message à recevoir car publish a eu lieu avant le sub.
    let res = tokio::time::timeout(std::time::Duration::from_millis(20), rx.recv()).await;
    assert!(res.is_err(), "expected timeout, got something");
}

#[tokio::test]
async fn subscribe_then_publish_delivers() {
    let bus = InMemoryBus::new();
    let (_t, mut rx) = bus.subscribe(Some("early"));
    let swarm = Uuid::now_v7();
    bus.publish(ev(swarm)).await.unwrap();
    let got = rx.recv().await.unwrap();
    assert_eq!(got.swarm_id(), Some(swarm));
}

#[tokio::test]
async fn token_uniqueness_invariant() {
    let bus = InMemoryBus::new();
    let mut tokens = std::collections::HashSet::new();
    for _ in 0..100 {
        let (t, _rx) = bus.subscribe(None);
        assert!(tokens.insert(t));
    }
    assert_eq!(bus.subscriber_count(), 100);
}

#[tokio::test]
async fn named_subscriber_overwrites_previous_with_same_name() {
    let bus = InMemoryBus::new();
    let (t1, _rx1) = bus.subscribe(Some("ui"));
    let (t2, _rx2) = bus.subscribe(Some("ui"));
    assert_eq!(t1, t2);
    assert_eq!(bus.subscriber_count(), 1);
}

#[tokio::test]
async fn sse_frame_includes_event_field() {
    let ev = StreamEvent::Heartbeat { ts: Utc::now() };
    let frame = ev.to_sse_frame();
    assert!(frame.contains("event: HEARTBEAT"));
    assert!(frame.contains("\"type\":\"HEARTBEAT\""));
}

#[tokio::test]
async fn dropped_count_increases_under_backpressure() {
    let bus = InMemoryBus::new();
    let (token, _rx_slow) = bus.subscribe(Some("slow"));
    let swarm = Uuid::now_v7();
    for _ in 0..3000 {
        bus.publish(ev(swarm)).await.unwrap();
    }
    let drops = bus.dropped_count(&token);
    assert!(drops > 0, "no drops detected: {drops}");
}

#[tokio::test]
async fn unsubscribe_unknown_token_is_safe() {
    let bus = InMemoryBus::new();
    bus.unsubscribe(&SubscriberToken::named("ghost"));
    assert_eq!(bus.subscriber_count(), 0);
}
