// SPDX-License-Identifier: UNLICENSED
//! Heartbeat — émet périodiquement [`StreamEvent::Heartbeat`] sur le bus
//! tant qu'un swarm est actif.
//!
//! **Pourquoi** : les proxies HTTP (nginx, AWS ALB, Cloudflare) coupent les
//! connexions SSE inactives au bout de ~60 s.  Émettre un évènement vide
//! toutes les ~15 s force la stack TCP/HTTP à garder la connexion ouverte.
//!
//! ## Usage
//!
//! ```ignore
//! let handle = HeartbeatHandle::spawn(
//!     coord.bus.clone(),
//!     std::time::Duration::from_secs(15),
//! );
//! // … lance le workflow …
//! handle.stop().await;  // ou drop pour stop implicite
//! ```

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;

use crate::events::{EventBus, StreamEvent};

/// Période par défaut entre 2 heartbeats (15 secondes).
///
/// Choix : < 60 s (timeout typique des proxies), > 5 s (évite le spam).
pub const DEFAULT_HEARTBEAT_PERIOD: Duration = Duration::from_secs(15);

/// Handle qui contrôle un heartbeat background task.
///
/// Drop = stop implicite (le notify est partagé par strong-arc).
pub struct HeartbeatHandle {
    notify: Arc<Notify>,
    join: Option<tokio::task::JoinHandle<()>>,
}

impl HeartbeatHandle {
    /// Spawne un heartbeat qui émet sur `bus` toutes les `period`.
    ///
    /// La tâche s'arrête quand [`HeartbeatHandle::stop`] est appelée
    /// (ou quand le handle est droppé).
    #[must_use]
    pub fn spawn(bus: Arc<dyn EventBus>, period: Duration) -> Self {
        let notify = Arc::new(Notify::new());
        let notify_c = Arc::clone(&notify);
        let join = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = notify_c.notified() => break,
                    _ = tokio::time::sleep(period) => {
                        let _ = bus
                            .publish(StreamEvent::Heartbeat {
                                ts: chrono::Utc::now(),
                            })
                            .await;
                    }
                }
            }
        });
        Self {
            notify,
            join: Some(join),
        }
    }

    /// Spawne avec la période par défaut (15 s).
    #[must_use]
    pub fn spawn_default(bus: Arc<dyn EventBus>) -> Self {
        Self::spawn(bus, DEFAULT_HEARTBEAT_PERIOD)
    }

    /// Stop le heartbeat et attend la fin de la tâche.
    pub async fn stop(mut self) {
        self.notify.notify_one();
        if let Some(join) = self.join.take() {
            let _ = join.await;
        }
    }
}

impl Drop for HeartbeatHandle {
    fn drop(&mut self) {
        // Best-effort : notify pour signaler l'arrêt.  Si la task tourne
        // encore, elle finira au prochain tick — pas d'await bloquant ici.
        self.notify.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::InMemoryBus;
    use std::time::Duration;

    #[tokio::test]
    async fn heartbeat_emits_periodically() {
        let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::new());
        let (_t, mut rx) = bus.subscribe(Some("test"));
        let h = HeartbeatHandle::spawn(Arc::clone(&bus), Duration::from_millis(50));
        // Attend 200 ms → on devrait avoir au moins 2 heartbeats.
        tokio::time::sleep(Duration::from_millis(220)).await;
        h.stop().await;

        let mut count = 0;
        while let Ok(Some(ev)) =
            tokio::time::timeout(Duration::from_millis(20), rx.recv()).await
        {
            if ev.tag() == "HEARTBEAT" {
                count += 1;
            }
        }
        assert!(count >= 2, "expected ≥ 2 heartbeats, got {count}");
    }

    #[tokio::test]
    async fn heartbeat_stop_terminates_task() {
        let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::new());
        let h = HeartbeatHandle::spawn(Arc::clone(&bus), Duration::from_millis(50));
        h.stop().await;
        // Le handle ne peut plus émettre — vérifie que pas plus de 1 heartbeat
        // n'arrive après stop.
        let (_t, mut rx) = bus.subscribe(Some("post-stop"));
        tokio::time::sleep(Duration::from_millis(150)).await;
        let count = std::iter::from_fn(|| rx.try_recv().ok())
            .filter(|e| e.tag() == "HEARTBEAT")
            .count();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn heartbeat_drop_signals_notify() {
        let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::new());
        let h = HeartbeatHandle::spawn(Arc::clone(&bus), Duration::from_millis(10));
        drop(h);
        // Drop doit notifier ; pas de panic, le task finira proprement.
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    #[test]
    fn default_period_is_15s() {
        assert_eq!(DEFAULT_HEARTBEAT_PERIOD, Duration::from_secs(15));
    }
}
