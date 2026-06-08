// SPDX-License-Identifier: UNLICENSED
//! **Phase 1 · WS6** — seam de journal d'événements + replay déterministe (F8).
//!
//! L'ordre d'arrivée des événements sur le bus est non déterministe. Pour des
//! tests reproductibles et le replay (Phase 4/6), [`EventLog`] enregistre la
//! séquence ORDONNÉE des événements et peut la **rejouer** à l'identique.
//!
//! ## Deux journaux complémentaires
//! - **EventLog** (ici) : journal en mémoire des [`SwarmEvent`] (avec payloads) —
//!   capturé via un abonné dédié, rejouable sur un [`TopicRouter`].
//! - **Le namespace reliable** (WS1) EST déjà un journal DURABLE ordonné AVEC
//!   valeurs : les cellules `"<ns>/<seq>"` du Blackboard, drainables par
//!   `ReliableReader` depuis `seq=1`. C'est le journal de vérité ; l'EventLog est
//!   l'outil de capture/replay best-effort.
//!
//! ## Déterminisme de l'état final
//! Pour une charge à CLÉS DISTINCTES (cas de l'essaim : chaque agent écrit sa
//! propre clé), l'état final du Blackboard est INDÉPENDANT de l'ordre — comparable
//! via [`crate::BlackboardSnapshot::value_map`] entre deux runs.

use std::sync::Mutex;
use std::sync::Arc;

use crate::eventbus::{EventBusSubscription, SwarmEvent};
use crate::topic_router::TopicRouter;

/// Journal ordonné d'événements — capture & replay.
#[derive(Debug, Default)]
pub struct EventLog {
    events: Mutex<Vec<SwarmEvent>>,
}

impl EventLog {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Enregistre un événement (à la fin du journal).
    pub fn record(&self, ev: SwarmEvent) {
        if let Ok(mut e) = self.events.lock() {
            e.push(ev);
        }
    }

    /// Capture en continu un abonnement dans le journal jusqu'à la fermeture du bus.
    /// (Abonné de journalisation dédié — ne prive pas les autres abonnés, broadcast.)
    pub async fn capture(&self, mut sub: EventBusSubscription) {
        while let Some(ev) = sub.recv().await {
            self.record(ev);
        }
    }

    /// Nombre d'événements journalisés.
    pub fn len(&self) -> usize {
        self.events.lock().map(|e| e.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Copie ordonnée du journal.
    pub fn events(&self) -> Vec<SwarmEvent> {
        self.events.lock().map(|e| e.clone()).unwrap_or_default()
    }

    /// Vide le journal.
    pub fn clear(&self) {
        if let Ok(mut e) = self.events.lock() {
            e.clear();
        }
    }

    /// **WS6** : rejoue le journal — republie chaque événement DANS L'ORDRE sur le
    /// routeur fourni. Renvoie le nombre d'événements republiés. Replay déterministe :
    /// même séquence d'entrée → mêmes publications, dans le même ordre.
    pub fn replay(&self, router: &TopicRouter) -> usize {
        let evs = self.events();
        let n = evs.len();
        for ev in evs {
            router.publish(ev);
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::{AgentId, SwarmId};
    use serde_json::json;
    use std::time::SystemTime;

    fn ev(topic: &str, i: i64) -> SwarmEvent {
        SwarmEvent {
            swarm_id: SwarmId::new(),
            published_by: AgentId::new("producer").unwrap(),
            topic: topic.into(),
            payload: json!({ "i": i }),
            published_at: SystemTime::now(),
        }
    }

    #[test]
    fn records_in_order() {
        let log = EventLog::new();
        for i in 0..5 {
            log.record(ev("t", i));
        }
        assert_eq!(log.len(), 5);
        let order: Vec<i64> = log.events().iter().map(|e| e.payload["i"].as_i64().unwrap()).collect();
        assert_eq!(order, vec![0, 1, 2, 3, 4]);
    }

    #[tokio::test]
    async fn replay_republishes_in_same_order() {
        let log = EventLog::new();
        for i in 0..4 {
            log.record(ev("topic", i));
        }
        let router = TopicRouter::new(64);
        let mut sub = router.subscribe("topic");
        let n = log.replay(&router);
        assert_eq!(n, 4);
        let mut got = Vec::new();
        while let Some(e) = sub.try_recv() {
            got.push(e.payload["i"].as_i64().unwrap());
        }
        assert_eq!(got, vec![0, 1, 2, 3], "replay préserve l'ordre");
    }

    #[tokio::test]
    async fn capture_drains_subscription_into_log() {
        let router = Arc::new(TopicRouter::new(64));
        let log = EventLog::new();
        // abonné de journalisation.
        let cap = {
            let sub = router.subscribe("t");
            let log = log.clone();
            tokio::spawn(async move { log.capture(sub).await })
        };
        for i in 0..3 {
            router.publish(ev("t", i));
        }
        drop(router); // ferme → capture termine.
        cap.await.unwrap();
        assert_eq!(log.len(), 3);
    }

    #[test]
    fn clear_empties_log() {
        let log = EventLog::new();
        log.record(ev("t", 1));
        assert!(!log.is_empty());
        log.clear();
        assert!(log.is_empty());
    }
}
