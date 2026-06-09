// SPDX-License-Identifier: UNLICENSED
//! `DashboardBus` — fan-in des seams → fan-out (**rejeu** + **live**).
//!
//! Chaque seam (write/event/appel/seal) appelle [`DashboardBus::emit`]. Le bus :
//! - **horodate + numérote** (`seq` monotone) ;
//! - **pousse dans un ring borné** (drop-oldest) → un abonné tardif **rejoue** le passé
//!   (indispensable : un run keyless dure ~ms, le navigateur arrive après) ;
//! - **diffuse** sur un `broadcast` (tail live) — pour les runs longs (self-host).
//!
//! **Best-effort assumé** : la diffusion est lossy (les abonnés lents voient `Lagged`). Le
//! dashboard est un **flux d'observation**, pas une source d'audit ; la vérité durable reste
//! la chaîne de provenance (`verify_swarm`, Inc.2).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tokio::sync::broadcast;
use uuid::Uuid;

use efe_nano_sdk::TenantId;

use crate::event::DashboardEvent;
use crate::metrics::DashboardMetrics;

/// Enveloppe diffusée : ordre total (`seq`) + horodatage + l'événement (aplati).
#[derive(Clone, Debug, Serialize)]
pub struct DashboardEnvelope {
    pub seq: u64,
    pub at_ms: u64,
    pub run_id: Uuid,
    #[serde(flatten)]
    pub event: DashboardEvent,
}

impl DashboardEnvelope {
    pub fn is_terminal(&self) -> bool {
        self.event.is_terminal()
    }
}

/// Hub d'observation **par run** : ring de rejeu + diffusion broadcast.
pub struct DashboardBus {
    tenant: TenantId,
    run_id: Uuid,
    seq: AtomicU64,
    emitted: AtomicU64,
    tx: broadcast::Sender<DashboardEnvelope>,
    ring: Mutex<VecDeque<DashboardEnvelope>>,
    ring_cap: usize,
    metrics: Option<Arc<DashboardMetrics>>,
}

impl DashboardBus {
    /// Crée un bus pour un run (ring borné à `ring_cap` events).
    pub fn new(tenant: TenantId, run_id: Uuid, ring_cap: usize) -> Arc<Self> {
        Self::new_metered(tenant, run_id, ring_cap, None)
    }

    /// Comme [`Self::new`], en branchant un registre de métriques **partagé** (Prometheus).
    pub fn new_metered(
        tenant: TenantId,
        run_id: Uuid,
        ring_cap: usize,
        metrics: Option<Arc<DashboardMetrics>>,
    ) -> Arc<Self> {
        let cap = ring_cap.max(1);
        let (tx, _rx) = broadcast::channel(cap.clamp(64, 65_536));
        Arc::new(Self {
            tenant,
            run_id,
            seq: AtomicU64::new(0),
            emitted: AtomicU64::new(0),
            tx,
            ring: Mutex::new(VecDeque::new()),
            ring_cap: cap,
            metrics,
        })
    }

    pub fn tenant(&self) -> TenantId {
        self.tenant
    }
    pub fn run_id(&self) -> Uuid {
        self.run_id
    }
    pub fn emitted(&self) -> u64 {
        self.emitted.load(Ordering::Relaxed)
    }
    /// Abonnés SSE **en vol** sur ce bus (pour la jauge `…_active_subscribers`).
    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }

    /// Émet un événement : numérote, range dans le ring (drop-oldest), diffuse (lossy).
    pub fn emit(&self, event: DashboardEvent) {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let env = DashboardEnvelope { seq, at_ms: now_ms(), run_id: self.run_id, event };
        self.emitted.fetch_add(1, Ordering::Relaxed);
        if let Some(m) = &self.metrics {
            m.inc_emitted();
        }
        if let Ok(mut ring) = self.ring.lock() {
            if ring.len() >= self.ring_cap {
                ring.pop_front(); // borne mémoire : on tronque le début (annoncé à l'UI)
            }
            ring.push_back(env.clone());
        }
        let _ = self.tx.send(env); // aucun abonné live = OK (le ring conserve tout)
    }

    /// S'abonne au flux **live** (tail). Combiner avec [`Self::replay`] pour le catch-up.
    pub fn subscribe(&self) -> broadcast::Receiver<DashboardEnvelope> {
        self.tx.subscribe()
    }

    /// Instantané du **ring de rejeu** (tout ce qui s'est déjà passé, borné).
    pub fn replay(&self) -> Vec<DashboardEnvelope> {
        self.ring.lock().map(|r| r.iter().cloned().collect()).unwrap_or_default()
    }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bus() -> Arc<DashboardBus> {
        DashboardBus::new(TenantId::system(), Uuid::from_u128(7), 4)
    }
    fn mark(i: u32) -> DashboardEvent {
        DashboardEvent::AgentCall { agent_id: format!("a{i}"), role: "Generator".into(), agent_kind: "ModeA".into(), cost_cents: 1 }
    }

    #[test]
    fn emit_numbers_and_replays() {
        let b = bus();
        b.emit(mark(0));
        b.emit(mark(1));
        let r = b.replay();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].seq, 0);
        assert_eq!(r[1].seq, 1);
        assert_eq!(b.emitted(), 2);
    }

    #[test]
    fn ring_drops_oldest_when_full() {
        let b = bus(); // cap 4
        for i in 0..6 {
            b.emit(mark(i));
        }
        let r = b.replay();
        assert_eq!(r.len(), 4, "ring borné");
        assert_eq!(r.first().unwrap().seq, 2, "le début est tronqué (drop-oldest)");
        assert_eq!(r.last().unwrap().seq, 5);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn subscribe_receives_live_tail() {
        let b = bus();
        let mut rx = b.subscribe();
        b.emit(mark(0));
        let env = rx.recv().await.unwrap();
        assert_eq!(env.seq, 0);
        assert_eq!(env.event.kind(), "agent_call");
    }

    #[test]
    fn terminal_is_detected() {
        let b = bus();
        b.emit(DashboardEvent::RunFinished { elapsed_ms: 1, agents_used: 1, output_preview: "x".into(), verified: true, root: None });
        assert!(b.replay().last().unwrap().is_terminal());
    }
}
