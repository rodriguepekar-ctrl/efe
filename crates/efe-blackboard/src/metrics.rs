// SPDX-License-Identifier: UNLICENSED
//! **Phase 1 · WS5** — observabilité live du substrat d'essaim.
//!
//! [`SubstrateMetrics`] : un registre de métriques **sans dépendance externe**
//! (atomics `std` uniquement), conçu pour le chemin chaud (tout est lock-free).
//! Il agrège :
//! - **compteurs** : écritures Blackboard, conflits CAS, rejets de taille,
//!   événements publiés / perdus (lag) ;
//! - **jauges** : octets Blackboard, agents actifs, événements en vol ;
//! - **histogramme** : latence de **propagation inter-agent** (publish → consume),
//!   d'où l'on tire p50 / p90 / p99 — la métrique-clé du roadmap Phase 1.
//!
//! Exposition : [`SubstrateMetrics::snapshot`] (struct sérialisable JSON) et
//! [`SubstrateMetrics::render_prometheus`] (texte Prometheus, sans crate tierce).
//!
//! [`MetricsObserver`] branche le tout sur le hook [`crate::observer::BlackboardObserver`]
//! existant (alternative légère au câblage direct `Blackboard::set_metrics`).

use std::sync::atomic::{AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::blackboard::BlackboardWrite;
use crate::observer::BlackboardObserver;

/// Bornes supérieures (en microsecondes) des buckets de l'histogramme de latence.
/// Un bucket « overflow » implicite capte tout ce qui dépasse la dernière borne.
const BOUNDS_US: &[u64] = &[
    50, 100, 250, 500, 1_000, 2_500, 5_000, 10_000, 25_000, 50_000, 100_000, 250_000, 500_000,
    1_000_000,
];

// ─────────────────────────────────────────────────────────────────────────────
//  Histogramme de latence (lock-free, bornes fixes)
// ─────────────────────────────────────────────────────────────────────────────

/// Histogramme à bornes fixes pour des durées (latence de propagation).
#[derive(Debug)]
pub struct LatencyHistogram {
    /// `BOUNDS_US.len() + 1` compteurs (le dernier = overflow > dernière borne).
    buckets: Vec<AtomicU64>,
    sum_us: AtomicU64,
    count: AtomicU64,
}

impl Default for LatencyHistogram {
    fn default() -> Self {
        Self::new()
    }
}

impl LatencyHistogram {
    pub fn new() -> Self {
        Self {
            buckets: (0..BOUNDS_US.len() + 1).map(|_| AtomicU64::new(0)).collect(),
            sum_us: AtomicU64::new(0),
            count: AtomicU64::new(0),
        }
    }

    /// Enregistre une durée.
    pub fn record(&self, d: Duration) {
        let us = d.as_micros().min(u64::MAX as u128) as u64;
        let idx = BOUNDS_US.iter().position(|&b| us <= b).unwrap_or(BOUNDS_US.len());
        self.buckets[idx].fetch_add(1, Ordering::Relaxed);
        self.sum_us.fetch_add(us, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    /// Moyenne en millisecondes (0 si vide).
    pub fn mean_ms(&self) -> f64 {
        let c = self.count.load(Ordering::Relaxed);
        if c == 0 {
            return 0.0;
        }
        (self.sum_us.load(Ordering::Relaxed) as f64 / c as f64) / 1000.0
    }

    /// Quantile approché (borne supérieure du bucket atteint), en millisecondes.
    pub fn quantile_ms(&self, q: f64) -> f64 {
        let total = self.count.load(Ordering::Relaxed);
        if total == 0 {
            return 0.0;
        }
        let target = (q * total as f64).ceil() as u64;
        let mut cum = 0u64;
        for (i, b) in self.buckets.iter().enumerate() {
            cum += b.load(Ordering::Relaxed);
            if cum >= target {
                // borne sup du bucket i (overflow → dernière borne finie).
                let bound = if i < BOUNDS_US.len() { BOUNDS_US[i] } else { *BOUNDS_US.last().unwrap() };
                return bound as f64 / 1000.0;
            }
        }
        *BOUNDS_US.last().unwrap() as f64 / 1000.0
    }

    fn cumulative(&self) -> Vec<(f64, u64)> {
        let mut out = Vec::with_capacity(BOUNDS_US.len() + 1);
        let mut cum = 0u64;
        for (i, b) in self.buckets.iter().enumerate() {
            cum += b.load(Ordering::Relaxed);
            let le = if i < BOUNDS_US.len() {
                BOUNDS_US[i] as f64 / 1000.0
            } else {
                f64::INFINITY
            };
            out.push((le, cum));
        }
        out
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  SubstrateMetrics — le registre central
// ─────────────────────────────────────────────────────────────────────────────

/// Registre de métriques du substrat (Blackboard + EventBus). À tenir dans un
/// `Arc` et partager : `Blackboard::set_metrics` + `EventBus::with_metrics`.
#[derive(Debug)]
pub struct SubstrateMetrics {
    // Blackboard — compteurs
    bb_writes_total: AtomicU64,
    bb_conflicts_total: AtomicU64,
    bb_cas_attempts_total: AtomicU64,
    bb_rejections_total: AtomicU64,
    // Blackboard — jauge
    bb_bytes: AtomicUsize,
    // EventBus — compteurs
    ev_published_total: AtomicU64,
    ev_dropped_total: AtomicU64,
    // jauges d'essaim
    active_agents: AtomicI64,
    inflight_events: AtomicI64,
    // latence de propagation inter-agent (publish → consume)
    propagation: LatencyHistogram,
}

impl Default for SubstrateMetrics {
    fn default() -> Self {
        Self::new_inner()
    }
}

impl SubstrateMetrics {
    fn new_inner() -> Self {
        Self {
            bb_writes_total: AtomicU64::new(0),
            bb_conflicts_total: AtomicU64::new(0),
            bb_cas_attempts_total: AtomicU64::new(0),
            bb_rejections_total: AtomicU64::new(0),
            bb_bytes: AtomicUsize::new(0),
            ev_published_total: AtomicU64::new(0),
            ev_dropped_total: AtomicU64::new(0),
            active_agents: AtomicI64::new(0),
            inflight_events: AtomicI64::new(0),
            propagation: LatencyHistogram::new(),
        }
    }

    /// Construit un registre partagé.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::new_inner())
    }

    // ── Blackboard ────────────────────────────────────────────────────────────
    pub fn record_bb_write(&self) {
        self.bb_writes_total.fetch_add(1, Ordering::Relaxed);
    }
    pub fn record_bb_conflict(&self) {
        self.bb_conflicts_total.fetch_add(1, Ordering::Relaxed);
    }
    /// Une tentative de CAS (`write_if`), qu'elle réussisse ou non.
    pub fn record_bb_cas_attempt(&self) {
        self.bb_cas_attempts_total.fetch_add(1, Ordering::Relaxed);
    }
    pub fn record_bb_rejection(&self) {
        self.bb_rejections_total.fetch_add(1, Ordering::Relaxed);
    }
    pub fn set_bb_bytes(&self, bytes: usize) {
        self.bb_bytes.store(bytes, Ordering::Relaxed);
    }

    // ── EventBus ──────────────────────────────────────────────────────────────
    pub fn record_event_published(&self) {
        self.ev_published_total.fetch_add(1, Ordering::Relaxed);
    }
    pub fn record_event_dropped(&self, n: u64) {
        self.ev_dropped_total.fetch_add(n, Ordering::Relaxed);
    }
    /// Latence de propagation inter-agent (mesurée à la consommation).
    pub fn record_propagation(&self, d: Duration) {
        self.propagation.record(d);
    }

    // ── Jauges d'essaim ──────────────────────────────────────────────────────
    pub fn agents_inc(&self) {
        self.active_agents.fetch_add(1, Ordering::Relaxed);
    }
    pub fn agents_dec(&self) {
        self.active_agents.fetch_sub(1, Ordering::Relaxed);
    }
    pub fn inflight_add(&self, delta: i64) {
        self.inflight_events.fetch_add(delta, Ordering::Relaxed);
    }

    /// Capture immuable des métriques (sérialisable JSON).
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            bb_writes_total: self.bb_writes_total.load(Ordering::Relaxed),
            bb_conflicts_total: self.bb_conflicts_total.load(Ordering::Relaxed),
            bb_cas_attempts_total: self.bb_cas_attempts_total.load(Ordering::Relaxed),
            bb_rejections_total: self.bb_rejections_total.load(Ordering::Relaxed),
            bb_bytes: self.bb_bytes.load(Ordering::Relaxed),
            ev_published_total: self.ev_published_total.load(Ordering::Relaxed),
            ev_dropped_total: self.ev_dropped_total.load(Ordering::Relaxed),
            active_agents: self.active_agents.load(Ordering::Relaxed),
            inflight_events: self.inflight_events.load(Ordering::Relaxed),
            propagation_count: self.propagation.count(),
            propagation_mean_ms: self.propagation.mean_ms(),
            propagation_p50_ms: self.propagation.quantile_ms(0.50),
            propagation_p90_ms: self.propagation.quantile_ms(0.90),
            propagation_p99_ms: self.propagation.quantile_ms(0.99),
        }
    }

    /// Rendu au format texte Prometheus (sans crate tierce).
    pub fn render_prometheus(&self) -> String {
        let s = self.snapshot();
        let mut o = String::new();
        let counter = |o: &mut String, name: &str, help: &str, v: u64| {
            o.push_str(&format!("# HELP {name} {help}\n# TYPE {name} counter\n{name} {v}\n"));
        };
        let gauge = |o: &mut String, name: &str, help: &str, v: i64| {
            o.push_str(&format!("# HELP {name} {help}\n# TYPE {name} gauge\n{name} {v}\n"));
        };
        counter(&mut o, "swarm_blackboard_writes_total", "Écritures Blackboard.", s.bb_writes_total);
        counter(&mut o, "swarm_blackboard_conflicts_total", "Conflits CAS write_if.", s.bb_conflicts_total);
        counter(&mut o, "swarm_cas_attempts_total", "Tentatives CAS write_if (succès ou conflit).", s.bb_cas_attempts_total);
        counter(&mut o, "swarm_blackboard_rejections_total", "Écritures rejetées (cap taille).", s.bb_rejections_total);
        gauge(&mut o, "swarm_blackboard_bytes", "Octets cumulés Blackboard.", s.bb_bytes as i64);
        counter(&mut o, "swarm_events_published_total", "Événements publiés sur le bus.", s.ev_published_total);
        counter(&mut o, "swarm_events_dropped_total", "Événements perdus (lag abonné).", s.ev_dropped_total);
        gauge(&mut o, "swarm_active_agents", "Agents actifs.", s.active_agents);
        gauge(&mut o, "swarm_inflight_events", "Événements en vol.", s.inflight_events);
        // histogramme propagation
        o.push_str("# HELP swarm_event_propagation_seconds Latence de propagation inter-agent.\n");
        o.push_str("# TYPE swarm_event_propagation_seconds histogram\n");
        for (le, cum) in self.propagation.cumulative() {
            let le_s = if le.is_infinite() { "+Inf".to_string() } else { format!("{:.6}", le / 1000.0) };
            o.push_str(&format!("swarm_event_propagation_seconds_bucket{{le=\"{le_s}\"}} {cum}\n"));
        }
        o.push_str(&format!(
            "swarm_event_propagation_seconds_count {}\n",
            s.propagation_count
        ));
        o.push_str(&format!(
            "swarm_event_propagation_seconds_sum {:.6}\n",
            s.propagation_mean_ms / 1000.0 * s.propagation_count as f64
        ));
        o
    }
}

/// Capture immuable et sérialisable des métriques.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    pub bb_writes_total: u64,
    pub bb_conflicts_total: u64,
    pub bb_cas_attempts_total: u64,
    pub bb_rejections_total: u64,
    pub bb_bytes: usize,
    pub ev_published_total: u64,
    pub ev_dropped_total: u64,
    pub active_agents: i64,
    pub inflight_events: i64,
    pub propagation_count: u64,
    pub propagation_mean_ms: f64,
    pub propagation_p50_ms: f64,
    pub propagation_p90_ms: f64,
    pub propagation_p99_ms: f64,
}

// ─────────────────────────────────────────────────────────────────────────────
//  MetricsObserver — branche les métriques sur le hook observer existant
// ─────────────────────────────────────────────────────────────────────────────

/// Observateur qui incrémente le compteur d'écritures via le seam
/// [`BlackboardObserver`]. Alternative légère à `Blackboard::set_metrics`
/// (qui, lui, capte AUSSI conflits/rejets/octets). N'utilisez qu'UNE des deux
/// voies pour compter les écritures, sous peine de double comptage.
#[derive(Debug)]
pub struct MetricsObserver {
    metrics: Arc<SubstrateMetrics>,
}

impl MetricsObserver {
    pub fn new(metrics: Arc<SubstrateMetrics>) -> Self {
        Self { metrics }
    }
}

impl BlackboardObserver for MetricsObserver {
    fn on_write(&self, _write: &BlackboardWrite) {
        self.metrics.record_bb_write();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_quantiles_monotone() {
        let h = LatencyHistogram::new();
        for ms in [0u64, 1, 1, 2, 5, 10, 50, 100, 500] {
            h.record(Duration::from_millis(ms));
        }
        assert_eq!(h.count(), 9);
        assert!(h.quantile_ms(0.5) <= h.quantile_ms(0.99));
        assert!(h.mean_ms() > 0.0);
    }

    #[test]
    fn empty_histogram_zero() {
        let h = LatencyHistogram::new();
        assert_eq!(h.quantile_ms(0.99), 0.0);
        assert_eq!(h.mean_ms(), 0.0);
    }

    #[test]
    fn counters_and_snapshot() {
        let m = SubstrateMetrics::new();
        m.record_bb_write();
        m.record_bb_write();
        m.record_bb_conflict();
        m.record_bb_rejection();
        m.set_bb_bytes(1234);
        m.record_event_published();
        m.record_event_dropped(3);
        m.record_propagation(Duration::from_micros(750));
        m.agents_inc();
        m.inflight_add(5);
        let s = m.snapshot();
        assert_eq!(s.bb_writes_total, 2);
        assert_eq!(s.bb_conflicts_total, 1);
        assert_eq!(s.bb_rejections_total, 1);
        assert_eq!(s.bb_bytes, 1234);
        assert_eq!(s.ev_published_total, 1);
        assert_eq!(s.ev_dropped_total, 3);
        assert_eq!(s.active_agents, 1);
        assert_eq!(s.inflight_events, 5);
        assert_eq!(s.propagation_count, 1);
    }

    #[test]
    fn prometheus_render_has_expected_lines() {
        let m = SubstrateMetrics::new();
        m.record_bb_write();
        m.record_propagation(Duration::from_millis(2));
        let txt = m.render_prometheus();
        assert!(txt.contains("swarm_blackboard_writes_total 1"));
        assert!(txt.contains("# TYPE swarm_event_propagation_seconds histogram"));
        assert!(txt.contains("le=\"+Inf\""));
    }

    #[test]
    fn snapshot_serde_round_trip() {
        let m = SubstrateMetrics::new();
        m.record_bb_write();
        let s = m.snapshot();
        let j = serde_json::to_string(&s).unwrap();
        let back: MetricsSnapshot = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }
}
