// SPDX-License-Identifier: UNLICENSED
//! `DashboardMetrics` — surfaces Prometheus du dashboard live (§11), **sans dépendance**.
//!
//! Même style que `ProvenanceMetrics`/`SubstrateMetrics` : compteurs atomiques O(1) + rendu
//! texte. Surfaces :
//! - `efe_swarm_dash_events_emitted_total` (counter) — events diffusés sur les bus.
//! - `efe_swarm_dash_events_dropped_total` (counter) — events perdus (abonnés SSE en retard).
//! - `efe_swarm_dash_active_runs` (gauge) — bus de dashboard actifs (registre).
//! - `efe_swarm_dash_active_subscribers` (gauge) — abonnés SSE en vol (tous runs).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Registre de métriques du dashboard (lock-free).
#[derive(Debug, Default)]
pub struct DashboardMetrics {
    events_emitted: AtomicU64,
    events_dropped: AtomicU64,
}

impl DashboardMetrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Compte un événement diffusé (appelé par [`crate::DashboardBus::emit`]).
    pub fn inc_emitted(&self) {
        self.events_emitted.fetch_add(1, Ordering::Relaxed);
    }

    /// Compte des événements perdus par un abonné SSE en retard (`broadcast` `Lagged(n)`).
    pub fn add_dropped(&self, n: u64) {
        self.events_dropped.fetch_add(n, Ordering::Relaxed);
    }

    pub fn events_emitted(&self) -> u64 {
        self.events_emitted.load(Ordering::Relaxed)
    }
    pub fn events_dropped(&self) -> u64 {
        self.events_dropped.load(Ordering::Relaxed)
    }

    /// Rendu Prometheus. `active_runs` / `active_subscribers` sont des jauges **instantanées**
    /// (lues du registre par l'appelant — la passerelle).
    pub fn render_prometheus(&self, active_runs: usize, active_subscribers: usize) -> String {
        let mut o = String::new();
        let counter = |o: &mut String, name: &str, help: &str, v: u64| {
            o.push_str(&format!("# HELP {name} {help}\n# TYPE {name} counter\n{name} {v}\n"));
        };
        let gauge = |o: &mut String, name: &str, help: &str, v: u64| {
            o.push_str(&format!("# HELP {name} {help}\n# TYPE {name} gauge\n{name} {v}\n"));
        };
        counter(&mut o, "efe_swarm_dash_events_emitted_total", "Événements de dashboard diffusés.", self.events_emitted());
        counter(&mut o, "efe_swarm_dash_events_dropped_total", "Événements perdus (abonné SSE en retard).", self.events_dropped());
        gauge(&mut o, "efe_swarm_dash_active_runs", "Bus de dashboard actifs.", active_runs as u64);
        gauge(&mut o, "efe_swarm_dash_active_subscribers", "Abonnés SSE en vol.", active_subscribers as u64);
        o
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_renders() {
        let m = DashboardMetrics::new();
        m.inc_emitted();
        m.inc_emitted();
        m.add_dropped(3);
        assert_eq!(m.events_emitted(), 2);
        assert_eq!(m.events_dropped(), 3);
        let txt = m.render_prometheus(1, 4);
        assert!(txt.contains("efe_swarm_dash_events_emitted_total 2"));
        assert!(txt.contains("efe_swarm_dash_events_dropped_total 3"));
        assert!(txt.contains("efe_swarm_dash_active_runs 1"));
        assert!(txt.contains("efe_swarm_dash_active_subscribers 4"));
        assert!(txt.contains("# TYPE efe_swarm_dash_active_runs gauge"));
    }
}
