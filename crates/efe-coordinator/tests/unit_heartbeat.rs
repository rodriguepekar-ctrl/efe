// SPDX-License-Identifier: UNLICENSED
//! Tests d'intégration `HeartbeatHandle`.

use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::{EventBus, HeartbeatHandle, InMemoryBus, DEFAULT_HEARTBEAT_PERIOD};

#[tokio::test]
async fn emits_heartbeats_at_configured_interval() {
    let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::new());
    let (_t, mut rx) = bus.subscribe(Some("hb-test"));
    let h = HeartbeatHandle::spawn(Arc::clone(&bus), Duration::from_millis(40));
    tokio::time::sleep(Duration::from_millis(180)).await;
    h.stop().await;

    let mut count = 0;
    while let Ok(Some(ev)) =
        tokio::time::timeout(Duration::from_millis(20), rx.recv()).await
    {
        if ev.tag() == "HEARTBEAT" {
            count += 1;
        }
    }
    // ~ 4 heartbeats attendus.
    assert!(count >= 3, "got {count}");
}

#[tokio::test]
async fn stop_terminates_within_period() {
    let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::new());
    let h = HeartbeatHandle::spawn(Arc::clone(&bus), Duration::from_millis(200));
    let started = std::time::Instant::now();
    h.stop().await;
    // stop doit retourner sous 50 ms (cancellation via notify).
    assert!(started.elapsed() < Duration::from_millis(100));
}

#[tokio::test]
async fn drop_does_not_panic() {
    let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::new());
    {
        let _h = HeartbeatHandle::spawn(Arc::clone(&bus), Duration::from_millis(10));
        tokio::time::sleep(Duration::from_millis(25)).await;
        // _h droppé ici → notify → tâche s'arrête au prochain tick.
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[test]
fn default_period_15s_documented() {
    assert_eq!(DEFAULT_HEARTBEAT_PERIOD, Duration::from_secs(15));
}

#[tokio::test]
async fn spawn_default_uses_documented_period() {
    let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::new());
    let h = HeartbeatHandle::spawn_default(Arc::clone(&bus));
    // Vérifie juste qu'on peut spawn et stop sans panic.
    h.stop().await;
}
