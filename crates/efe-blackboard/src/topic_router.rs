// SPDX-License-Identifier: UNLICENSED
//! **Phase 1 · WS1** — fabrique d'événements routée par TOPIC (corrige F1).
//!
//! L'[`crate::EventBus`] d'origine est un **broadcast global** : chaque abonné
//! reçoit CHAQUE événement → fan-out O(N²) à N agents (goulot #1, mesuré par la
//! baseline `swarm_load`). Le [`TopicRouter`] partitionne le bus en **un canal
//! broadcast par topic** : un abonné ne reçoit que les événements du/des topics
//! auxquels il s'abonne → fan-out O(abonnés_intéressés).
//!
//! Réutilise [`crate::SubstrateMetrics`] (WS5) : publish / drop / latence de
//! propagation comptés exactement comme pour l'EventBus.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;

use crate::eventbus::{EventBusSubscription, SwarmEvent};
use crate::metrics::SubstrateMetrics;

/// **WS1** : routeur d'événements par topic — un canal broadcast par topic.
pub struct TopicRouter {
    topics: DashMap<String, broadcast::Sender<SwarmEvent>>,
    capacity: usize,
    published_total: Arc<AtomicU64>,
    dropped_total: Arc<AtomicU64>,
    metrics: Option<Arc<SubstrateMetrics>>,
}

impl std::fmt::Debug for TopicRouter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TopicRouter")
            .field("topics", &self.topics.len())
            .field("capacity", &self.capacity)
            .field("published_total", &self.published_total.load(Ordering::Relaxed))
            .field("dropped_total", &self.dropped_total.load(Ordering::Relaxed))
            .finish()
    }
}

impl TopicRouter {
    /// Construit un routeur ; `capacity` = profondeur de chaque canal broadcast.
    pub fn new(capacity: usize) -> Self {
        Self {
            topics: DashMap::new(),
            capacity,
            published_total: Arc::new(AtomicU64::new(0)),
            dropped_total: Arc::new(AtomicU64::new(0)),
            metrics: None,
        }
    }

    /// Construit avec un registre de métriques (WS5) branché.
    pub fn with_metrics(capacity: usize, metrics: Arc<SubstrateMetrics>) -> Self {
        let mut r = Self::new(capacity);
        r.metrics = Some(metrics);
        r
    }

    /// Sender du topic (créé paresseusement à la 1re référence).
    fn channel(&self, topic: &str) -> broadcast::Sender<SwarmEvent> {
        if let Some(s) = self.topics.get(topic) {
            return s.clone();
        }
        self.topics
            .entry(topic.to_string())
            .or_insert_with(|| broadcast::channel(self.capacity.max(1)).0)
            .clone()
    }

    /// **WS1** : publie un événement, routé sur SON topic (`ev.topic`). Renvoie le
    /// nombre d'abonnés du topic l'ayant reçu (0 si aucun).
    pub fn publish(&self, ev: SwarmEvent) -> usize {
        let sender = self.channel(&ev.topic);
        let n = sender.send(ev).unwrap_or(0);
        self.published_total.fetch_add(1, Ordering::Relaxed);
        if let Some(m) = &self.metrics {
            m.record_event_published();
        }
        n
    }

    /// **WS1** : s'abonne à UN topic — ne reçoit QUE ses événements.
    pub fn subscribe(&self, topic: &str) -> EventBusSubscription {
        let rx = self.channel(topic).subscribe();
        EventBusSubscription::from_parts(rx, self.dropped_total.clone(), self.metrics.clone())
    }

    /// **WS1** : s'abonne à TOUS les topics dont le nom commence par `prefix`
    /// (ex. un agrégateur écoutant `"job.shard"` reçoit `job.shard.0`, `.1`, …).
    /// Fusionne les flux via une tâche de transfert par topic vers un canal mpsc.
    ///
    /// ⚠ Ne capte que les topics EXISTANT au moment de l'appel (les souscriptions
    /// d'agents/publications les créent) — abonnez-vous après l'enregistrement des
    /// workers. Mal employé (préfixe trop large) il recrée un firehose.
    pub fn subscribe_prefix(&self, prefix: &str) -> MergedSubscription {
        // Collecte les senders correspondants AVANT de spawner (libère les gardes de shard).
        let senders: Vec<broadcast::Sender<SwarmEvent>> = self
            .topics
            .iter()
            .filter(|e| e.key().starts_with(prefix))
            .map(|e| e.value().clone())
            .collect();
        let topic_count = senders.len();

        let (tx, rx) = mpsc::channel(self.capacity.max(1));
        let mut forwarders = Vec::with_capacity(topic_count);
        for sender in senders {
            let mut sub = EventBusSubscription::from_parts(
                sender.subscribe(),
                self.dropped_total.clone(),
                self.metrics.clone(),
            );
            let tx = tx.clone();
            forwarders.push(tokio::spawn(async move {
                while let Some(ev) = sub.recv().await {
                    if tx.send(ev).await.is_err() {
                        break; // consommateur disparu
                    }
                }
            }));
        }
        // `tx` (original) droppé en sortie → le mpsc se ferme quand tous les
        // forwarders se terminent (canaux source fermés à la chute du routeur).
        MergedSubscription {
            rx,
            forwarders,
            topic_count,
        }
    }

    /// Nombre de topics actifs (canaux créés).
    pub fn topic_count(&self) -> usize {
        self.topics.len()
    }

    /// Nombre d'abonnés d'un topic donné.
    pub fn subscriber_count(&self, topic: &str) -> usize {
        self.topics.get(topic).map(|s| s.receiver_count()).unwrap_or(0)
    }

    /// Total cumulé d'événements publiés (tous topics).
    pub fn published_total(&self) -> u64 {
        self.published_total.load(Ordering::Relaxed)
    }

    /// Total cumulé d'événements perdus (lag d'un abonné, tous topics).
    pub fn dropped_total(&self) -> u64 {
        self.dropped_total.load(Ordering::Relaxed)
    }
}

/// **WS1** : souscription fusionnée sur plusieurs topics (par préfixe). Reçoit, en
/// un seul flux, les événements de tous les topics correspondants.
#[derive(Debug)]
pub struct MergedSubscription {
    rx: mpsc::Receiver<SwarmEvent>,
    forwarders: Vec<JoinHandle<()>>,
    topic_count: usize,
}

impl MergedSubscription {
    /// Reçoit le prochain événement fusionné ; `None` quand tous les topics sources
    /// sont fermés (routeur droppé).
    pub async fn recv(&mut self) -> Option<SwarmEvent> {
        self.rx.recv().await
    }

    /// Réception non bloquante.
    pub fn try_recv(&mut self) -> Option<SwarmEvent> {
        self.rx.try_recv().ok()
    }

    /// Nombre de topics fusionnés au moment de l'abonnement.
    pub fn topic_count(&self) -> usize {
        self.topic_count
    }
}

impl Drop for MergedSubscription {
    fn drop(&mut self) {
        // Stoppe les tâches de transfert (évite les fuites si on drop tôt).
        for h in &self.forwarders {
            h.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::{AgentId, SwarmId};
    use serde_json::json;
    use std::time::SystemTime;

    fn ev(topic: &str) -> SwarmEvent {
        SwarmEvent {
            swarm_id: SwarmId::new(),
            published_by: AgentId::new("producer").unwrap(),
            topic: topic.into(),
            payload: json!({"x": 1}),
            published_at: SystemTime::now(),
        }
    }

    #[tokio::test]
    async fn routes_only_to_matching_topic() {
        let r = TopicRouter::new(16);
        let mut a = r.subscribe("alpha");
        let mut b = r.subscribe("beta");
        r.publish(ev("alpha"));
        // alpha reçoit, beta NON (pas le firehose).
        assert_eq!(a.try_recv().map(|e| e.topic), Some("alpha".to_string()));
        assert!(b.try_recv().is_none());
    }

    #[tokio::test]
    async fn multiple_subscribers_same_topic_all_receive() {
        let r = TopicRouter::new(16);
        let mut s1 = r.subscribe("t");
        let mut s2 = r.subscribe("t");
        r.publish(ev("t"));
        assert!(s1.try_recv().is_some());
        assert!(s2.try_recv().is_some());
        assert_eq!(r.subscriber_count("t"), 2);
    }

    #[tokio::test]
    async fn publish_without_subscriber_is_zero_not_error() {
        let r = TopicRouter::new(16);
        assert_eq!(r.publish(ev("ghost")), 0);
        assert_eq!(r.published_total(), 1);
        assert_eq!(r.topic_count(), 1);
    }

    #[tokio::test]
    async fn metrics_count_publishes() {
        let m = SubstrateMetrics::new();
        let r = TopicRouter::with_metrics(16, m.clone());
        let _s = r.subscribe("t");
        r.publish(ev("t"));
        r.publish(ev("t"));
        assert_eq!(m.snapshot().ev_published_total, 2);
    }

    #[tokio::test]
    async fn subscribe_prefix_merges_matching_topics_only() {
        let r = TopicRouter::new(256);
        // créer les topics (subscribe crée le canal).
        let _w0 = r.subscribe("job.shard.0");
        let _w1 = r.subscribe("job.shard.1");
        let _other = r.subscribe("unrelated");
        let mut merged = r.subscribe_prefix("job.shard");
        assert_eq!(merged.topic_count(), 2, "2 topics correspondent au préfixe");
        r.publish(ev("job.shard.0"));
        r.publish(ev("job.shard.1"));
        r.publish(ev("unrelated")); // hors préfixe → exclu
        let a = merged.recv().await.unwrap();
        let b = merged.recv().await.unwrap();
        assert!(a.topic.starts_with("job.shard"));
        assert!(b.topic.starts_with("job.shard"));
    }

    #[tokio::test]
    async fn merged_subscription_terminates_when_router_dropped() {
        let r = TopicRouter::new(64);
        let _s = r.subscribe("p.a");
        let mut merged = r.subscribe_prefix("p.");
        r.publish(ev("p.a"));
        assert!(merged.recv().await.is_some());
        drop(r); // ferme les sources → forwarders terminent → mpsc fermé.
        assert!(merged.recv().await.is_none(), "termine quand le routeur tombe");
    }

    #[tokio::test]
    async fn fanout_is_isolated_per_topic() {
        // 3 topics, 1 abonné chacun ; publier sur l'un n'atteint pas les autres.
        let r = TopicRouter::new(16);
        let mut subs: Vec<_> = ["a", "b", "c"].iter().map(|t| r.subscribe(t)).collect();
        r.publish(ev("b"));
        assert!(subs[0].try_recv().is_none());
        assert!(subs[1].try_recv().is_some());
        assert!(subs[2].try_recv().is_none());
    }
}
