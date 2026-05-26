// SPDX-License-Identifier: UNLICENSED
//! `InMemoryBus` — implémentation pub/sub in-process via `tokio::mpsc`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use dashmap::DashMap;
use parking_lot::Mutex;
use tokio::sync::mpsc;

use crate::error::CoordResult;

use super::bus::{EventBus, EventFilter, SubscriberToken};
use super::stream::StreamEvent;

/// Capacité du canal par subscriber (au-delà → drop + compteur).
const PER_SUBSCRIBER_CAP: usize = 1024;

struct Subscriber {
    tx: mpsc::Sender<StreamEvent>,
    dropped: Arc<Mutex<u64>>,
    /// Filtre optionnel : si Some et qu'il retourne false, l'évènement
    /// est skippé silencieusement (pas de drop_count).
    filter: Option<EventFilter>,
}

/// Bus in-memory.  Pas de garantie de delivery si subscriber lent (drop + compte).
pub struct InMemoryBus {
    subs: DashMap<SubscriberToken, Subscriber>,
    /// Compteur monotone : nombre total de `publish` jamais lancés.
    publish_count: AtomicU64,
    /// Compteur monotone : nombre total de `publish` ayant fini de fan-out.
    delivered_count: AtomicU64,
}

impl Default for InMemoryBus {
    fn default() -> Self {
        Self {
            subs: DashMap::new(),
            publish_count: AtomicU64::new(0),
            delivered_count: AtomicU64::new(0),
        }
    }
}

impl InMemoryBus {
    /// Construit un bus vide.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Compteur monotone : nombre de `publish` ayant été soumis.
    /// Exposé pour observabilité + tests `flush()`.
    #[must_use]
    pub fn publish_count(&self) -> u64 {
        self.publish_count.load(Ordering::SeqCst)
    }

    /// Compteur monotone : nombre de `publish` ayant fini leur fan-out.
    #[must_use]
    pub fn delivered_count(&self) -> u64 {
        self.delivered_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl EventBus for InMemoryBus {
    async fn publish(&self, event: StreamEvent) -> CoordResult<()> {
        self.publish_count.fetch_add(1, Ordering::SeqCst);
        // Itère via DashMap : pas de lock global, sub set est `Send`.
        for entry in self.subs.iter() {
            let sub = entry.value();
            // Filtre éventuel : si présent et false, skip silencieux.
            if let Some(filter) = &sub.filter {
                if !filter(&event) {
                    continue;
                }
            }
            match sub.tx.try_send(event.clone()) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    let mut g = sub.dropped.lock();
                    *g = g.saturating_add(1);
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    // Le receiver est tombé.  On laisse la map nettoyer
                    // au prochain unsubscribe explicite ou flush.
                }
            }
        }
        self.delivered_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn subscribe(
        &self,
        name: Option<&str>,
    ) -> (SubscriberToken, mpsc::Receiver<StreamEvent>) {
        let token = match name {
            Some(n) => SubscriberToken::named(n),
            None => SubscriberToken::new(),
        };
        let (tx, rx) = mpsc::channel(PER_SUBSCRIBER_CAP);
        self.subs.insert(
            token.clone(),
            Subscriber {
                tx,
                dropped: Arc::new(Mutex::new(0)),
                filter: None,
            },
        );
        (token, rx)
    }

    fn subscribe_filtered(
        &self,
        name: Option<&str>,
        filter: EventFilter,
    ) -> (SubscriberToken, mpsc::Receiver<StreamEvent>) {
        let token = match name {
            Some(n) => SubscriberToken::named(n),
            None => SubscriberToken::new(),
        };
        let (tx, rx) = mpsc::channel(PER_SUBSCRIBER_CAP);
        self.subs.insert(
            token.clone(),
            Subscriber {
                tx,
                dropped: Arc::new(Mutex::new(0)),
                filter: Some(filter),
            },
        );
        (token, rx)
    }

    fn unsubscribe(&self, token: &SubscriberToken) {
        self.subs.remove(token);
    }

    fn dropped_count(&self, token: &SubscriberToken) -> u64 {
        self.subs
            .get(token)
            .map(|s| *s.value().dropped.lock())
            .unwrap_or(0)
    }

    async fn flush(&self) -> CoordResult<()> {
        // Real flush : attend que delivered_count rattrape publish_count.
        // Borné par 1 s pour éviter un deadlock infini si un publish
        // est resté coincé (cas pathologique).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        loop {
            let pub_n = self.publish_count.load(Ordering::SeqCst);
            let del_n = self.delivered_count.load(Ordering::SeqCst);
            if del_n >= pub_n {
                return Ok(());
            }
            if std::time::Instant::now() > deadline {
                return Ok(()); // best-effort, n'erreur pas
            }
            tokio::task::yield_now().await;
        }
    }

    fn subscriber_count(&self) -> usize {
        self.subs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::bus::{filter_by_swarm, filter_by_tag};
    use crate::events::stream::MissionTag;
    use chrono::Utc;
    use efe_nano_app_coordinator::types::router::TaskClass;
    use uuid::Uuid;

    fn evt(swarm: Uuid) -> StreamEvent {
        StreamEvent::SwarmStarted {
            swarm_id: swarm,
            mission: MissionTag::MissionA,
            user_text: "x".into(),
            task_class: TaskClass::FactualQa,
            ts: Utc::now(),
        }
    }

    #[tokio::test]
    async fn single_subscriber_receives() {
        let bus = InMemoryBus::new();
        let (token, mut rx) = bus.subscribe(Some("test"));
        assert_eq!(bus.subscriber_count(), 1);
        let s = Uuid::now_v7();
        bus.publish(evt(s)).await.unwrap();
        let got = rx.recv().await.unwrap();
        assert_eq!(got.swarm_id(), Some(s));
        bus.unsubscribe(&token);
        assert_eq!(bus.subscriber_count(), 0);
    }

    #[tokio::test]
    async fn multi_subscriber_fanout() {
        let bus = InMemoryBus::new();
        let (_, mut rx_a) = bus.subscribe(Some("a"));
        let (_, mut rx_b) = bus.subscribe(Some("b"));
        let s = Uuid::now_v7();
        bus.publish(evt(s)).await.unwrap();
        let a = rx_a.recv().await.unwrap();
        let b = rx_b.recv().await.unwrap();
        assert_eq!(a.swarm_id(), Some(s));
        assert_eq!(b.swarm_id(), Some(s));
    }

    #[tokio::test]
    async fn slow_subscriber_drops_and_counts() {
        let bus = InMemoryBus::new();
        // Souscrit mais NE LIT PAS.
        let (token, _rx_slow) = bus.subscribe(Some("slow"));
        let s = Uuid::now_v7();
        // Publie 2 × capacité d'évènements.
        for _ in 0..(PER_SUBSCRIBER_CAP * 2) {
            bus.publish(evt(s)).await.unwrap();
        }
        let dropped = bus.dropped_count(&token);
        assert!(dropped > 0, "expected some drops, got {dropped}");
    }

    #[tokio::test]
    async fn unsubscribe_is_idempotent() {
        let bus = InMemoryBus::new();
        let (token, _rx) = bus.subscribe(None);
        bus.unsubscribe(&token);
        bus.unsubscribe(&token); // pas de panic
        assert_eq!(bus.subscriber_count(), 0);
    }

    #[tokio::test]
    async fn flush_blocks_until_drained() {
        let bus = InMemoryBus::new();
        bus.publish(StreamEvent::Heartbeat { ts: Utc::now() })
            .await
            .unwrap();
        bus.flush().await.unwrap();
        assert!(bus.delivered_count() >= bus.publish_count());
    }

    #[tokio::test]
    async fn subscribe_filtered_by_tag_skips_others() {
        let bus = InMemoryBus::new();
        let (_t, mut rx) = bus.subscribe_filtered(Some("hb-only"), filter_by_tag("HEARTBEAT"));
        bus.publish(StreamEvent::Heartbeat { ts: Utc::now() })
            .await
            .unwrap();
        bus.publish(evt(Uuid::now_v7())).await.unwrap();
        // Reçoit le heartbeat.
        let got = rx.recv().await.unwrap();
        assert_eq!(got.tag(), "HEARTBEAT");
        // Pas d'autre évènement (le SwarmStarted a été filtré).
        let timed = tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv()).await;
        assert!(timed.is_err());
    }

    #[tokio::test]
    async fn subscribe_filtered_by_swarm_isolates() {
        let bus = InMemoryBus::new();
        let s1 = Uuid::now_v7();
        let s2 = Uuid::now_v7();
        let (_t, mut rx) = bus.subscribe_filtered(Some("s1-only"), filter_by_swarm(s1));
        bus.publish(evt(s1)).await.unwrap();
        bus.publish(evt(s2)).await.unwrap();
        let got = rx.recv().await.unwrap();
        assert_eq!(got.swarm_id(), Some(s1));
        let timed = tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv()).await;
        assert!(timed.is_err());
    }

    #[tokio::test]
    async fn publish_count_increments() {
        let bus = InMemoryBus::new();
        let n0 = bus.publish_count();
        for _ in 0..5 {
            bus.publish(StreamEvent::Heartbeat { ts: Utc::now() })
                .await
                .unwrap();
        }
        assert_eq!(bus.publish_count(), n0 + 5);
        assert_eq!(bus.delivered_count(), n0 + 5);
    }

    #[tokio::test]
    async fn unfiltered_subscriber_still_gets_all() {
        let bus = InMemoryBus::new();
        let (_t, mut rx) = bus.subscribe(Some("any"));
        bus.publish(StreamEvent::Heartbeat { ts: Utc::now() })
            .await
            .unwrap();
        bus.publish(evt(Uuid::now_v7())).await.unwrap();
        assert!(rx.recv().await.is_some());
        assert!(rx.recv().await.is_some());
    }
}
