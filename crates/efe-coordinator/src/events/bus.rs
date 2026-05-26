// SPDX-License-Identifier: UNLICENSED
//! Trait abstrait `EventBus`.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::CoordResult;

use super::stream::StreamEvent;

/// Token opaque retourné par `subscribe()` ; identifie un consumer
/// pour les diagnostics de lag.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SubscriberToken(pub String);

impl SubscriberToken {
    /// Construit un token aléatoire (UUID v7 stringifié).
    #[must_use]
    pub fn new() -> Self {
        Self(uuid::Uuid::now_v7().to_string())
    }

    /// Token nommé (utile pour logging).
    #[must_use]
    pub fn named(name: impl Into<String>) -> Self {
        Self(name.into())
    }
}

impl Default for SubscriberToken {
    fn default() -> Self {
        Self::new()
    }
}

/// Filtre d'évènements pour `subscribe_filtered`.
///
/// Fonction `Fn(&StreamEvent) -> bool` thread-safe ; appelée pour chaque
/// publication.  Si elle retourne `false`, l'évènement n'est pas envoyé
/// à ce subscriber (mais l'est aux autres).
///
/// ## Garanties
///
/// - Appelée 1 seule fois par évènement et par subscriber.
/// - Lock-free côté impl `InMemoryBus` (DashMap iteration).
/// - Pas de blocage : la fonction doit être pure et rapide.
pub type EventFilter = Arc<dyn Fn(&StreamEvent) -> bool + Send + Sync + 'static>;

/// Helper : filtre par tag.
#[must_use]
pub fn filter_by_tag(tag: &'static str) -> EventFilter {
    Arc::new(move |ev: &StreamEvent| ev.tag() == tag)
}

/// Helper : filtre par `swarm_id` exact.
#[must_use]
pub fn filter_by_swarm(swarm_id: uuid::Uuid) -> EventFilter {
    Arc::new(move |ev: &StreamEvent| ev.swarm_id() == Some(swarm_id))
}

/// Helper : combine deux filtres en AND.
#[must_use]
pub fn filter_and(a: EventFilter, b: EventFilter) -> EventFilter {
    Arc::new(move |ev: &StreamEvent| a(ev) && b(ev))
}

/// Helper : combine deux filtres en OR.
#[must_use]
pub fn filter_or(a: EventFilter, b: EventFilter) -> EventFilter {
    Arc::new(move |ev: &StreamEvent| a(ev) || b(ev))
}

/// Bus pub/sub abstrait.  Une instance est partagée par plusieurs
/// publishers (workflows en parallèle) et plusieurs subscribers (UI client SSE,
/// auditors, monitoring).
///
/// ## Sémantique
///
/// - **publish** est non-bloquant, "fire and forget".  Si un subscriber
///   est trop lent, ses messages sont droppés ; le compteur est exposé
///   via [`EventBus::dropped_count`].
/// - **subscribe** retourne un canal `Receiver<StreamEvent>` qu'il faut
///   consommer en boucle.
/// - **subscribe_filtered** identique mais ne livre que les évènements
///   matchant le `EventFilter`.
/// - **flush** attend que tous les messages publiés soient effectivement
///   délivrés (utile en fin de mission pour cohérence d'audit).
#[async_trait]
pub trait EventBus: Send + Sync + 'static {
    /// Publie un évènement à tous les subscribers.
    async fn publish(&self, event: StreamEvent) -> CoordResult<()>;

    /// Souscrit et retourne un canal de réception + le token associé.
    fn subscribe(
        &self,
        name: Option<&str>,
    ) -> (SubscriberToken, tokio::sync::mpsc::Receiver<StreamEvent>);

    /// Souscrit avec un filtre : seuls les évènements pour lesquels
    /// `filter` retourne true seront livrés à ce subscriber.
    ///
    /// Default impl : appelle `subscribe` puis logge un warning si le
    /// filtre est ignoré (cas des impls qui ne le supportent pas).
    /// `InMemoryBus` override pour un vrai filtrage côté publish.
    fn subscribe_filtered(
        &self,
        name: Option<&str>,
        _filter: EventFilter,
    ) -> (SubscriberToken, tokio::sync::mpsc::Receiver<StreamEvent>) {
        tracing::warn!("EventBus impl does not support filtered subscribe; fallback to all events");
        self.subscribe(name)
    }

    /// Détache un subscriber par token (idempotent).
    fn unsubscribe(&self, token: &SubscriberToken);

    /// Nombre d'évènements perdus par un subscriber donné (lag/overflow).
    fn dropped_count(&self, token: &SubscriberToken) -> u64;

    /// Force le flush des canaux internes.  Retourne quand tous les
    /// `publish` antérieurs ont été délivrés.
    async fn flush(&self) -> CoordResult<()>;

    /// Subscribers actifs (pour observabilité).
    fn subscriber_count(&self) -> usize;
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    #[test]
    fn token_named_round_trips() {
        let t = SubscriberToken::named("ui-client-42");
        assert_eq!(t.0, "ui-client-42");
    }

    #[test]
    fn token_default_is_unique() {
        let a = SubscriberToken::default();
        let b = SubscriberToken::default();
        assert_ne!(a, b);
    }

    #[test]
    fn token_serializes() {
        let t = SubscriberToken::named("x");
        let s = serde_json::to_string(&t).unwrap();
        let back: SubscriberToken = serde_json::from_str(&s).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn filter_by_tag_matches() {
        let f = filter_by_tag("HEARTBEAT");
        let hb = StreamEvent::Heartbeat { ts: Utc::now() };
        assert!(f(&hb));
        let other = StreamEvent::RobotStarted {
            swarm_id: Uuid::now_v7(),
            robot_id: "a".into(),
            ts: Utc::now(),
        };
        assert!(!f(&other));
    }

    #[test]
    fn filter_by_swarm_matches() {
        let s = Uuid::now_v7();
        let f = filter_by_swarm(s);
        let ev = StreamEvent::RobotStarted {
            swarm_id: s,
            robot_id: "a".into(),
            ts: Utc::now(),
        };
        assert!(f(&ev));
        let other = StreamEvent::RobotStarted {
            swarm_id: Uuid::now_v7(),
            robot_id: "a".into(),
            ts: Utc::now(),
        };
        assert!(!f(&other));
        let hb = StreamEvent::Heartbeat { ts: Utc::now() };
        assert!(!f(&hb), "heartbeats have no swarm_id");
    }

    #[test]
    fn filter_and_or_compose() {
        let s = Uuid::now_v7();
        let by_swarm = filter_by_swarm(s);
        let by_tag = filter_by_tag("ROBOT_STARTED");
        let both = filter_and(by_swarm.clone(), by_tag.clone());
        let either = filter_or(by_swarm, by_tag);

        let match_both = StreamEvent::RobotStarted {
            swarm_id: s,
            robot_id: "a".into(),
            ts: Utc::now(),
        };
        let match_tag_only = StreamEvent::RobotStarted {
            swarm_id: Uuid::now_v7(),
            robot_id: "a".into(),
            ts: Utc::now(),
        };
        assert!(both(&match_both));
        assert!(!both(&match_tag_only));
        assert!(either(&match_both));
        assert!(either(&match_tag_only));
    }
}
