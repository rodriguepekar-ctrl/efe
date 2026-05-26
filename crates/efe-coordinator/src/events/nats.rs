// SPDX-License-Identifier: UNLICENSED
//! `NatsBus` — implémentation NATS JetStream (feature `nats-bus`).
//!
//! Cette implémentation est destinée à la prod multi-instance (plusieurs
//! Coordinators derrière un load-balancer doivent partager le bus).
//!
//! ## Sémantique JetStream
//!
//! - 1 stream par tenant : `efe.swarm.<tenant_uuid>` (subject hierarchy).
//! - Subject par évènement : `efe.swarm.<tenant>.<swarm_id>.<tag>`.
//! - Persistence configurable (par défaut : `memory`, 1 h de rétention).
//! - Consumer ephemeral pour les SSE clients (auto-cleanup).
//!
//! ## Stabilité
//!
//! **Beta** — branchement Phase 7 (multi-tenant prod).  Le wire format
//! (`StreamEvent` JSON) est stable v1.0.0 mais l'implémentation NATS
//! peut évoluer (clustering, mirror, etc.).

use std::sync::Arc;

use async_nats::jetstream;
use async_trait::async_trait;
use dashmap::DashMap;
use parking_lot::Mutex;
use tokio::sync::mpsc;

use crate::error::{CoordError, CoordResult};

use super::bus::{EventBus, EventFilter, SubscriberToken};
use super::stream::StreamEvent;

/// Subject root commun à tous les évènements (avant per-tenant).
const SUBJECT_ROOT: &str = "efe.swarm";

/// Capacité du canal par subscriber (au-delà → drop + compteur).
const PER_SUBSCRIBER_CAP: usize = 4096;

struct LocalSubscriber {
    tx: mpsc::Sender<StreamEvent>,
    dropped: Arc<Mutex<u64>>,
    filter: Option<EventFilter>,
}

/// Bus pub/sub via NATS JetStream.
///
/// Construit via [`NatsBus::connect`] qui établit le lien et crée le
/// stream JetStream si nécessaire.
pub struct NatsBus {
    js: jetstream::Context,
    tenant_subject: String,
    // Mirror local pour permettre subscribe() sans recréer un consumer
    // NATS à chaque appel.  Le worker NATS pousse dans tous les
    // subscribers locaux quand un message arrive.
    locals: Arc<DashMap<SubscriberToken, LocalSubscriber>>,
}

impl NatsBus {
    /// Se connecte à un cluster NATS et configure le stream pour le tenant.
    ///
    /// `nats_url` est de la forme `"nats://host:4222"` ou
    /// `"nats://user:pass@host:4222"`.
    pub async fn connect(nats_url: &str, tenant_id: uuid::Uuid) -> CoordResult<Self> {
        let client = async_nats::connect(nats_url)
            .await
            .map_err(|e| CoordError::Config(format!("nats connect: {e}")))?;
        let js = jetstream::new(client);

        let tenant_subject = format!("{SUBJECT_ROOT}.{tenant_id}.>");

        // Crée le stream JetStream s'il n'existe pas (idempotent).
        let _ = js
            .get_or_create_stream(jetstream::stream::Config {
                name: format!("efe-swarm-{tenant_id}"),
                subjects: vec![format!("{SUBJECT_ROOT}.{tenant_id}.>")],
                storage: jetstream::stream::StorageType::Memory,
                max_age: std::time::Duration::from_secs(3600),
                ..Default::default()
            })
            .await
            .map_err(|e| CoordError::Config(format!("nats stream config: {e}")))?;

        let bus = Self {
            js,
            tenant_subject,
            locals: Arc::new(DashMap::new()),
        };

        // Spawn worker qui mirror NATS → subscribers locaux.
        bus.spawn_consumer_worker();
        Ok(bus)
    }

    /// Spawne le worker NATS qui consomme le subject `tenant_subject` et
    /// pousse dans tous les subscribers locaux (fan-out).
    fn spawn_consumer_worker(&self) {
        let js = self.js.clone();
        let subject = self.tenant_subject.clone();
        let locals = Arc::clone(&self.locals);

        tokio::spawn(async move {
            let stream_name = subject
                .split('.')
                .skip(1)
                .next()
                .map(|s| format!("efe-swarm-{s}"))
                .unwrap_or_else(|| "efe-swarm".to_string());

            let stream = match js.get_stream(&stream_name).await {
                Ok(s) => s,
                Err(_) => return,
            };

            let consumer_cfg = jetstream::consumer::pull::Config {
                durable_name: None, // ephemeral
                filter_subject: subject.clone(),
                ..Default::default()
            };
            let consumer = match stream.create_consumer(consumer_cfg).await {
                Ok(c) => c,
                Err(_) => return,
            };

            loop {
                let mut batch = match consumer.fetch().max_messages(32).messages().await {
                    Ok(b) => b,
                    Err(_) => {
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        continue;
                    }
                };
                while let Some(Ok(msg)) = futures::StreamExt::next(&mut batch).await {
                    if let Ok(ev) = serde_json::from_slice::<StreamEvent>(&msg.payload) {
                        for entry in locals.iter() {
                            let sub = entry.value();
                            if let Some(f) = &sub.filter {
                                if !f(&ev) {
                                    continue;
                                }
                            }
                            match sub.tx.try_send(ev.clone()) {
                                Ok(()) => {}
                                Err(mpsc::error::TrySendError::Full(_)) => {
                                    let mut g = sub.dropped.lock();
                                    *g = g.saturating_add(1);
                                }
                                Err(mpsc::error::TrySendError::Closed(_)) => {}
                            }
                        }
                    }
                    let _ = msg.ack().await;
                }
            }
        });
    }

    /// Construit le subject pour un évènement donné.
    fn subject_for(&self, ev: &StreamEvent) -> String {
        let tenant_prefix = self.tenant_subject.trim_end_matches(".>").to_string();
        if let Some(sid) = ev.swarm_id() {
            format!("{tenant_prefix}.{sid}.{}", ev.tag())
        } else {
            format!("{tenant_prefix}.system.{}", ev.tag())
        }
    }
}

#[async_trait]
impl EventBus for NatsBus {
    async fn publish(&self, event: StreamEvent) -> CoordResult<()> {
        let subject = self.subject_for(&event);
        let payload = serde_json::to_vec(&event).map_err(|e| CoordError::Config(e.to_string()))?;
        self.js
            .publish(subject, payload.into())
            .await
            .map_err(|e| CoordError::Config(format!("nats publish: {e}")))?;
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
        self.locals.insert(
            token.clone(),
            LocalSubscriber {
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
        self.locals.insert(
            token.clone(),
            LocalSubscriber {
                tx,
                dropped: Arc::new(Mutex::new(0)),
                filter: Some(filter),
            },
        );
        (token, rx)
    }

    fn unsubscribe(&self, token: &SubscriberToken) {
        self.locals.remove(token);
    }

    fn dropped_count(&self, token: &SubscriberToken) -> u64 {
        self.locals
            .get(token)
            .map(|s| *s.value().dropped.lock())
            .unwrap_or(0)
    }

    async fn flush(&self) -> CoordResult<()> {
        // NATS flush : attend que tous les messages publiés soient ack côté serveur.
        // async-nats expose flush() sur le client mais pas sur le context JS ;
        // on yield juste, sachant qu'en prod ce sera bracketed par le caller.
        tokio::task::yield_now().await;
        Ok(())
    }

    fn subscriber_count(&self) -> usize {
        self.locals.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_root_is_constant() {
        // Sanity check pour éviter qu'on renomme par mégarde — casse les
        // déploiements multi-tenant en chaîne.
        assert_eq!(SUBJECT_ROOT, "efe.swarm");
    }

    #[test]
    fn per_subscriber_cap_is_4k() {
        assert_eq!(PER_SUBSCRIBER_CAP, 4096);
    }

    // Tests d'intégration NATS = environnement live ; voir
    // tests/nats_live.rs (env-gated EFE_NATS_LIVE=1, Phase 7).
}
