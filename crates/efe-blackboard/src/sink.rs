// SPDX-License-Identifier: UNLICENSED
//! **Phase 1 · WS3** — exutoire (sink) d'historique d'écritures.
//!
//! Avant WS3, `Blackboard` accumulait TOUT l'historique dans un `Mutex<Vec<…>>`
//! non borné (croissance mémoire + lock sur le chemin chaud). WS3 borne
//! l'historique en mémoire (ring récent) et **streame** l'historique complet
//! vers un [`WriteSink`] pluggable — consommé par la provenance (Phase 4) ou un
//! journal de replay (Phase 6). Le sink est appelé en synchrone depuis `write` :
//! il DOIT être rapide et non bloquant (cf. [`ChannelSink`], non bloquant via
//! `try_send`).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::blackboard::BlackboardWrite;
use crate::eventbus::SwarmEvent;

/// Exutoire d'historique : reçoit chaque écriture committée.
///
/// **Contrat** : `on_write` est appelé depuis la tâche écrivaine, sous aucun
/// lock du Blackboard. Il DOIT être rapide, non bloquant, et ne pas paniquer.
pub trait WriteSink: Send + Sync + 'static {
    fn on_write(&self, write: &BlackboardWrite);
    /// Optionnel : vidange (no-op par défaut).
    fn flush(&self) {}
}

/// Exutoire d'**événements** : reçoit chaque message publié sur le bus, au seam
/// `publish` (**pas** `subscribe` — donc fiable, sans course aux topics existants).
///
/// Pendant exact de [`WriteSink`] pour le bus : c'est le seam d'observation du **trafic
/// inter-agent** (Phase 5 Inc.3 — dashboard live). **Contrat identique** : `on_event` est
/// appelé synchroniquement depuis la tâche publiante ; il DOIT être rapide, non bloquant,
/// et ne pas paniquer (cf. un tap lossy via `try_send`).
pub trait EventSink: Send + Sync + 'static {
    fn on_event(&self, event: &SwarmEvent);
}

/// Sink no-op (défaut implicite quand aucun sink n'est branché).
#[derive(Debug, Default)]
pub struct NoopSink;
impl WriteSink for NoopSink {
    fn on_write(&self, _write: &BlackboardWrite) {}
}

/// Sink compteur — utile pour tests + métriques légères.
#[derive(Debug, Default)]
pub struct CountingSink {
    count: AtomicU64,
    bytes: AtomicU64,
}
impl CountingSink {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }
    pub fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }
}
impl WriteSink for CountingSink {
    fn on_write(&self, write: &BlackboardWrite) {
        self.count.fetch_add(1, Ordering::Relaxed);
        self.bytes.fetch_add(write.bytes as u64, Ordering::Relaxed);
    }
}

/// Sink capturant — stocke chaque écriture (tests / debug). Borné optionnellement.
#[derive(Debug)]
pub struct CapturingSink {
    writes: std::sync::Mutex<Vec<BlackboardWrite>>,
    cap: usize,
}
impl CapturingSink {
    pub fn new(cap: usize) -> Self {
        Self {
            writes: std::sync::Mutex::new(Vec::new()),
            cap,
        }
    }
    pub fn writes(&self) -> Vec<BlackboardWrite> {
        self.writes.lock().map(|w| w.clone()).unwrap_or_default()
    }
    pub fn len(&self) -> usize {
        self.writes.lock().map(|w| w.len()).unwrap_or(0)
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
impl WriteSink for CapturingSink {
    fn on_write(&self, write: &BlackboardWrite) {
        if let Ok(mut w) = self.writes.lock() {
            if self.cap == 0 || w.len() < self.cap {
                w.push(write.clone());
            }
        }
    }
}

/// Sink streaming non bloquant vers un canal borné (`tokio::sync::mpsc`).
///
/// Le consommateur (provenance / journal) draine le `Receiver`. Sur canal plein,
/// `try_send` échoue → on incrémente `dropped` (best-effort, observable) plutôt
/// que de bloquer le chemin chaud d'écriture.
#[derive(Debug)]
pub struct ChannelSink {
    tx: tokio::sync::mpsc::Sender<BlackboardWrite>,
    dropped: AtomicU64,
}
impl ChannelSink {
    /// Construit un sink + le `Receiver` à drainer côté consommateur.
    pub fn new(capacity: usize) -> (Arc<Self>, tokio::sync::mpsc::Receiver<BlackboardWrite>) {
        let (tx, rx) = tokio::sync::mpsc::channel(capacity.max(1));
        (
            Arc::new(Self {
                tx,
                dropped: AtomicU64::new(0),
            }),
            rx,
        )
    }
    /// Nombre d'écritures perdues (canal plein).
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}
impl WriteSink for ChannelSink {
    fn on_write(&self, write: &BlackboardWrite) {
        if self.tx.try_send(write.clone()).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::AgentId;
    use std::time::SystemTime;

    fn w(p: &str, bytes: usize) -> BlackboardWrite {
        BlackboardWrite {
            pointer: p.into(),
            version: 1,
            agent_id: AgentId::new("test-agent").unwrap(),
            at: SystemTime::now(),
            bytes,
        }
    }

    #[test]
    fn counting_sink_counts_and_sums_bytes() {
        let s = CountingSink::new();
        s.on_write(&w("/a", 10));
        s.on_write(&w("/b", 25));
        assert_eq!(s.count(), 2);
        assert_eq!(s.bytes(), 35);
    }

    #[test]
    fn capturing_sink_respects_cap() {
        let s = CapturingSink::new(2);
        s.on_write(&w("/a", 1));
        s.on_write(&w("/b", 1));
        s.on_write(&w("/c", 1)); // au-delà du cap → ignoré
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn noop_sink_does_nothing() {
        let s = NoopSink;
        s.on_write(&w("/a", 1));
        s.flush();
    }

    #[tokio::test]
    async fn channel_sink_streams_then_drops_on_full() {
        let (sink, mut rx) = ChannelSink::new(2);
        sink.on_write(&w("/a", 1));
        sink.on_write(&w("/b", 1));
        // canal plein (cap 2, non drainé) → la 3e est perdue.
        sink.on_write(&w("/c", 1));
        assert_eq!(sink.dropped(), 1);
        assert!(rx.recv().await.is_some());
        assert!(rx.recv().await.is_some());
    }
}
