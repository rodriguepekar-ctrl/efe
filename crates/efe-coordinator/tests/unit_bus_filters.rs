// SPDX-License-Identifier: UNLICENSED
//! Tests d'intégration : `subscribe_filtered` + helpers `filter_*`.

use std::time::Duration;

use chrono::Utc;
use efe_coordinator::{
    filter_and, filter_by_swarm, filter_by_tag, filter_or, EventBus, InMemoryBus,
    MissionTag, StreamEvent,
};
use efe_nano_app_coordinator::types::router::TaskClass;
use uuid::Uuid;

fn swarm_event(s: Uuid) -> StreamEvent {
    StreamEvent::SwarmStarted {
        swarm_id: s,
        mission: MissionTag::MissionA,
        user_text: "x".into(),
        task_class: TaskClass::FactualQa,
        ts: Utc::now(),
    }
}

#[tokio::test]
async fn filter_by_tag_admits_only_matching() {
    let bus = InMemoryBus::new();
    let (_t, mut rx) = bus.subscribe_filtered(Some("only-hb"), filter_by_tag("HEARTBEAT"));
    bus.publish(StreamEvent::Heartbeat { ts: Utc::now() }).await.unwrap();
    bus.publish(swarm_event(Uuid::now_v7())).await.unwrap();
    bus.publish(StreamEvent::Heartbeat { ts: Utc::now() }).await.unwrap();
    let mut got = 0;
    while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await {
        assert_eq!(ev.tag(), "HEARTBEAT");
        got += 1;
    }
    assert_eq!(got, 2);
}

#[tokio::test]
async fn filter_by_swarm_id_admits_only_matching() {
    let bus = InMemoryBus::new();
    let s1 = Uuid::now_v7();
    let (_t, mut rx) = bus.subscribe_filtered(Some("s1"), filter_by_swarm(s1));
    bus.publish(swarm_event(s1)).await.unwrap();
    bus.publish(swarm_event(Uuid::now_v7())).await.unwrap();
    let got = rx.recv().await.unwrap();
    assert_eq!(got.swarm_id(), Some(s1));
    // Pas d'autre.
    let timed = tokio::time::timeout(Duration::from_millis(30), rx.recv()).await;
    assert!(timed.is_err());
}

#[tokio::test]
async fn filter_and_combines_constraints() {
    let bus = InMemoryBus::new();
    let s = Uuid::now_v7();
    let f = filter_and(filter_by_swarm(s), filter_by_tag("ROBOT_STARTED"));
    let (_t, mut rx) = bus.subscribe_filtered(Some("strict"), f);

    // Match.
    bus.publish(StreamEvent::RobotStarted {
        swarm_id: s,
        robot_id: "a".into(),
        ts: Utc::now(),
    })
    .await
    .unwrap();
    // Tag match mais swarm no-match.
    bus.publish(StreamEvent::RobotStarted {
        swarm_id: Uuid::now_v7(),
        robot_id: "a".into(),
        ts: Utc::now(),
    })
    .await
    .unwrap();
    // Swarm match mais tag no-match.
    bus.publish(swarm_event(s)).await.unwrap();

    let got = rx.recv().await.unwrap();
    assert_eq!(got.tag(), "ROBOT_STARTED");
    assert_eq!(got.swarm_id(), Some(s));
    let timed = tokio::time::timeout(Duration::from_millis(30), rx.recv()).await;
    assert!(timed.is_err());
}

#[tokio::test]
async fn filter_or_admits_either_constraint() {
    let bus = InMemoryBus::new();
    let f = filter_or(filter_by_tag("HEARTBEAT"), filter_by_tag("SWARM_STARTED"));
    let (_t, mut rx) = bus.subscribe_filtered(Some("lax"), f);
    bus.publish(StreamEvent::Heartbeat { ts: Utc::now() }).await.unwrap();
    bus.publish(swarm_event(Uuid::now_v7())).await.unwrap();
    bus.publish(StreamEvent::RobotStarted {
        swarm_id: Uuid::now_v7(),
        robot_id: "a".into(),
        ts: Utc::now(),
    })
    .await
    .unwrap();
    let mut got = std::collections::HashSet::new();
    while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await {
        got.insert(ev.tag().to_string());
    }
    assert!(got.contains("HEARTBEAT"));
    assert!(got.contains("SWARM_STARTED"));
    assert!(!got.contains("ROBOT_STARTED"));
}

#[tokio::test]
async fn flush_completes_quickly_when_drained() {
    let bus = InMemoryBus::new();
    let started = std::time::Instant::now();
    bus.flush().await.unwrap();
    assert!(started.elapsed() < Duration::from_millis(50));
}

#[tokio::test]
async fn flush_after_burst_drains_all() {
    let bus = InMemoryBus::new();
    let (_t, mut rx) = bus.subscribe(Some("reader"));
    let reader = tokio::spawn(async move {
        let mut n = 0;
        while let Some(_e) = rx.recv().await {
            n += 1;
        }
        n
    });
    for _ in 0..100 {
        bus.publish(StreamEvent::Heartbeat { ts: Utc::now() }).await.unwrap();
    }
    bus.flush().await.unwrap();
    assert_eq!(bus.delivered_count(), bus.publish_count());
    drop(bus);
    let n = reader.await.unwrap();
    assert_eq!(n, 100);
}

#[tokio::test]
async fn unfiltered_and_filtered_subscribers_coexist() {
    let bus = InMemoryBus::new();
    let (_t_all, mut rx_all) = bus.subscribe(Some("all"));
    let (_t_hb, mut rx_hb) =
        bus.subscribe_filtered(Some("hb"), filter_by_tag("HEARTBEAT"));
    bus.publish(StreamEvent::Heartbeat { ts: Utc::now() }).await.unwrap();
    bus.publish(swarm_event(Uuid::now_v7())).await.unwrap();
    let mut all_count = 0;
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(30), rx_all.recv()).await {
        all_count += 1;
    }
    let mut hb_count = 0;
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(30), rx_hb.recv()).await {
        hb_count += 1;
    }
    assert_eq!(all_count, 2);
    assert_eq!(hb_count, 1);
}
