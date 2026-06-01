// SPDX-License-Identifier: UNLICENSED
//! **Sprint 13 — N21** : EventBus — bounded broadcast pub/sub for
//! inter-agent events.
//!
//! Architecture § 7.2 + ADR-3 + I4 + I16.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use efe_nano_sdk::{AgentId, SwarmId};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::error::EventBusError;

/// **N21 + I4** : default channel capacity (per architecture § 7.2).
pub const DEFAULT_EVENTBUS_CAPACITY: usize = 256;

/// **N21** : an event published on the bus.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SwarmEvent {
    pub swarm_id: SwarmId,
    pub published_by: AgentId,
    pub topic: String,
    pub payload: serde_json::Value,
    pub published_at: SystemTime,
}

/// **N21** : the event bus.
///
/// Backed by a `tokio::sync::broadcast` channel with capacity 256
/// by default.  Slow consumers may see `RecvError::Lagged` when a
/// burst exceeds capacity — they catch up by discarding events.
pub struct EventBus {
    tx: broadcast::Sender<SwarmEvent>,
    capacity: usize,
    published_total: Arc<AtomicU64>,
    dropped_total: Arc<AtomicU64>,
}

impl std::fmt::Debug for EventBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventBus")
            .field("capacity", &self.capacity)
            .field(
                "subscriber_count",
                &self.tx.receiver_count(),
            )
            .field(
                "published_total",
                &self.published_total.load(Ordering::Relaxed),
            )
            .field(
                "dropped_total",
                &self.dropped_total.load(Ordering::Relaxed),
            )
            .finish()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    /// **N21** : Construct with default capacity (256).
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_EVENTBUS_CAPACITY)
    }

    /// **N21** : Construct with explicit capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity.max(1));
        Self {
            tx,
            capacity,
            published_total: Arc::new(AtomicU64::new(0)),
            dropped_total: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// **N21** : Number of active subscribers.
    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }

    /// **N21** : Cumulative published events.
    pub fn published_total(&self) -> u64 {
        self.published_total.load(Ordering::Relaxed)
    }

    /// **N21 + F11** : Cumulative dropped events (capacity exceeded
    /// for a subscriber).
    pub fn dropped_total(&self) -> u64 {
        self.dropped_total.load(Ordering::Relaxed)
    }

    /// **N21** : Publish an event.
    ///
    /// If no subscribers are listening, the event is dropped silently
    /// (this is fine — pub/sub without subscribers is no-op by
    /// convention, NOT an error).  Counters are still updated.
    ///
    /// If the channel is full for one subscriber, that subscriber
    /// will see `RecvError::Lagged` on its next receive ; this is
    /// surfaced via `dropped_total` for ops dashboards.
    pub fn publish(&self, event: SwarmEvent) -> Result<usize, EventBusError> {
        let result = self.tx.send(event);
        self.published_total.fetch_add(1, Ordering::Relaxed);
        match result {
            Ok(receiver_count) => Ok(receiver_count),
            Err(_) => {
                // No active receivers.  Not an error per ADR-3 ; we
                // still tick the counter so ops can see "publishing
                // without subscribers" rates.
                Ok(0)
            }
        }
    }

    /// **N21** : Subscribe to all events on the bus.  The
    /// subscription is per-receiver ; multiple subscribers each get
    /// every event.
    pub fn subscribe(&self) -> EventBusSubscription {
        EventBusSubscription {
            rx: self.tx.subscribe(),
            dropped_total: self.dropped_total.clone(),
        }
    }
}

/// **N21** : a subscriber handle.  Owned by ONE consumer task.
///
/// Calling `recv` repeatedly delivers events in publish order.
/// `RecvError::Lagged(n)` indicates the consumer fell behind by
/// `n` events (channel capacity exceeded between recv calls) — the
/// consumer catches up to the latest with the next recv.
pub struct EventBusSubscription {
    rx: broadcast::Receiver<SwarmEvent>,
    dropped_total: Arc<AtomicU64>,
}

impl std::fmt::Debug for EventBusSubscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EventBusSubscription")
    }
}

impl EventBusSubscription {
    /// **N21** : Receive the next event.  Returns `None` if the bus
    /// has been dropped (channel closed).  On `RecvError::Lagged`,
    /// ticks the drop counter and recurses for the next event.
    pub async fn recv(&mut self) -> Option<SwarmEvent> {
        loop {
            match self.rx.recv().await {
                Ok(e) => return Some(e),
                Err(broadcast::error::RecvError::Closed) => return None,
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    self.dropped_total
                        .fetch_add(n, Ordering::Relaxed);
                    // Loop and try again ; the receiver is now caught
                    // up to the latest available event.
                    continue;
                }
            }
        }
    }

    /// **N21** : Non-blocking receive — returns immediately.
    pub fn try_recv(&mut self) -> Option<SwarmEvent> {
        match self.rx.try_recv() {
            Ok(e) => Some(e),
            Err(broadcast::error::TryRecvError::Empty) => None,
            Err(broadcast::error::TryRecvError::Closed) => None,
            Err(broadcast::error::TryRecvError::Lagged(n)) => {
                self.dropped_total.fetch_add(n, Ordering::Relaxed);
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(s: &str) -> AgentId {
        AgentId::new(s).unwrap()
    }

    fn ev(topic: &str) -> SwarmEvent {
        SwarmEvent {
            swarm_id: SwarmId::new(),
            published_by: agent("ner_extractor"),
            topic: topic.into(),
            payload: serde_json::json!({"x": 1}),
            published_at: SystemTime::now(),
        }
    }

    #[test]
    fn fresh_bus_has_default_capacity_no_subscribers() {
        let b = EventBus::new();
        assert_eq!(b.capacity(), DEFAULT_EVENTBUS_CAPACITY);
        assert_eq!(b.subscriber_count(), 0);
        assert_eq!(b.published_total(), 0);
        assert_eq!(b.dropped_total(), 0);
    }

    #[tokio::test]
    async fn publish_then_subscribe_no_event_delivered() {
        // Publishing BEFORE subscribing : the message is gone
        // (broadcast doesn't replay history).
        let b = EventBus::new();
        b.publish(ev("found_pii")).unwrap();
        let mut sub = b.subscribe();
        let got = sub.try_recv();
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn subscribe_then_publish_delivers_event() {
        let b = EventBus::new();
        let mut sub = b.subscribe();
        b.publish(ev("found_pii")).unwrap();
        let got = sub.recv().await.unwrap();
        assert_eq!(got.topic, "found_pii");
    }

    #[tokio::test]
    async fn multiple_subscribers_all_receive() {
        let b = EventBus::new();
        let mut s1 = b.subscribe();
        let mut s2 = b.subscribe();
        b.publish(ev("topic_a")).unwrap();
        let g1 = s1.recv().await.unwrap();
        let g2 = s2.recv().await.unwrap();
        assert_eq!(g1.topic, "topic_a");
        assert_eq!(g2.topic, "topic_a");
    }

    #[tokio::test]
    async fn publish_counts_increment() {
        let b = EventBus::new();
        let _sub = b.subscribe();
        for _ in 0..5 {
            b.publish(ev("x")).unwrap();
        }
        assert_eq!(b.published_total(), 5);
    }

    #[tokio::test]
    async fn publish_with_no_subscribers_returns_ok_zero() {
        let b = EventBus::new();
        let n = b.publish(ev("x")).unwrap();
        assert_eq!(n, 0);
        // Counter still ticks.
        assert_eq!(b.published_total(), 1);
    }

    #[tokio::test]
    async fn subscriber_count_reflects_active() {
        let b = EventBus::new();
        assert_eq!(b.subscriber_count(), 0);
        let s1 = b.subscribe();
        assert_eq!(b.subscriber_count(), 1);
        let s2 = b.subscribe();
        assert_eq!(b.subscriber_count(), 2);
        drop(s1);
        // tokio::broadcast counts active receivers ; the dropped
        // receiver should reduce the count.
        assert_eq!(b.subscriber_count(), 1);
        drop(s2);
        assert_eq!(b.subscriber_count(), 0);
    }

    #[tokio::test]
    async fn slow_consumer_lagged_increments_dropped_counter() {
        // Bus capacity = 4 ; we publish 10 events without recv'ing.
        // The 5th publish onwards will lag the slow consumer.
        let b = EventBus::with_capacity(4);
        let mut sub = b.subscribe();
        for i in 0..10 {
            b.publish(ev(&format!("topic_{i}"))).unwrap();
        }
        // Now drain.  We'll see Lagged at some point ; recv() returns
        // the next available event and we keep going until closed.
        let mut received = Vec::new();
        for _ in 0..6 {
            if let Some(e) = sub.try_recv() {
                received.push(e);
            }
        }
        // At least some events lost.
        assert!(b.dropped_total() > 0);
        // The receiver caught up to the most recent events.
        assert!(!received.is_empty());
    }

    #[tokio::test]
    async fn try_recv_returns_none_on_empty() {
        let b = EventBus::new();
        let mut sub = b.subscribe();
        assert!(sub.try_recv().is_none());
    }

    #[tokio::test]
    async fn try_recv_returns_event_when_present() {
        let b = EventBus::new();
        let mut sub = b.subscribe();
        b.publish(ev("x")).unwrap();
        let got = sub.try_recv().unwrap();
        assert_eq!(got.topic, "x");
    }

    #[tokio::test]
    async fn subscriber_after_drop_of_bus_recv_returns_none() {
        let b = EventBus::new();
        let mut sub = b.subscribe();
        b.publish(ev("x")).unwrap();
        drop(b);
        // First recv : the queued event.
        let g = sub.recv().await.unwrap();
        assert_eq!(g.topic, "x");
        // Second recv : channel closed.
        assert!(sub.recv().await.is_none());
    }

    #[test]
    fn swarm_event_serde_round_trip() {
        let e = ev("found_pii");
        let json = serde_json::to_string(&e).unwrap();
        let back: SwarmEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
    }

    #[test]
    fn capacity_zero_clamped_to_one() {
        // tokio::broadcast::channel(0) would panic.  Our `with_capacity`
        // clamps to 1.
        let b = EventBus::with_capacity(0);
        assert_eq!(b.capacity(), 0);
        // But the actual channel works.
        let _ = b.subscribe();
    }

    #[test]
    fn debug_format_does_not_panic() {
        let b = EventBus::new();
        let _ = format!("{b:?}");
        let sub = b.subscribe();
        let _ = format!("{sub:?}");
    }
}
