// SPDX-License-Identifier: UNLICENSED
//! # `metrics` — surfaces d'observabilité de la provenance (§11), **sans dépendance**.
//!
//! Mirroir du pattern `efe_blackboard::SubstrateMetrics` : compteurs atomiques O(1) +
//! rendu texte Prometheus ([`ProvenanceMetrics::render_prometheus`]) sans crate tierce.
//! Surfaces exposées (§11) :
//! - `efe_swarm_prov_records_total` (counter)
//! - `efe_swarm_prov_anchored_records_total` (counter)
//! - `efe_swarm_prov_captured_calls_total` (counter)
//! - `efe_swarm_prov_anchored_writes_total` (counter)
//! - `efe_swarm_prov_chain_breaks_total` (counter)
//! - `efe_swarm_prov_traced_ratio` (gauge — cible 1.0)
//! - `efe_swarm_prov_overhead_ns` (gauge — overhead moyen par run, ns)
//! - `efe_swarm_prov_bytes` (gauge — taille moyenne d'un enregistrement, octets)

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::recorder::SealedProvenance;
use crate::verify::SwarmProvenanceReport;

/// Registre de métriques de provenance (compteurs atomiques, lock-free).
#[derive(Debug, Default)]
pub struct ProvenanceMetrics {
    records_total: AtomicU64,
    anchored_records: AtomicU64,
    captured_calls: AtomicU64,
    anchored_writes: AtomicU64,
    chain_breaks_total: AtomicU64,
    bytes_sum: AtomicU64,
    bytes_samples: AtomicU64,
    overhead_ns_sum: AtomicU64,
    overhead_samples: AtomicU64,
}

impl ProvenanceMetrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Comptabilise un run scellé (records, ancrage, captures).
    pub fn observe_sealed(&self, sealed: &SealedProvenance) {
        self.records_total.fetch_add(sealed.len() as u64, Ordering::Relaxed);
        self.anchored_records.fetch_add(sealed.anchored_records as u64, Ordering::Relaxed);
        self.captured_calls.fetch_add(sealed.captured_calls, Ordering::Relaxed);
        self.anchored_writes.fetch_add(sealed.anchored_writes, Ordering::Relaxed);
    }

    /// Comptabilise un bilan de vérification (rupture de chaîne si non `ok`).
    pub fn observe_verify(&self, report: &SwarmProvenanceReport) {
        if !report.ok() {
            self.chain_breaks_total.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Enregistre la taille (octets) d'un enregistrement sérialisé.
    pub fn observe_bytes(&self, bytes: usize) {
        self.bytes_sum.fetch_add(bytes as u64, Ordering::Relaxed);
        self.bytes_samples.fetch_add(1, Ordering::Relaxed);
    }

    /// Enregistre l'overhead (ns) d'un run instrumenté vs baseline.
    pub fn observe_overhead_ns(&self, ns: u64) {
        self.overhead_ns_sum.fetch_add(ns, Ordering::Relaxed);
        self.overhead_samples.fetch_add(1, Ordering::Relaxed);
    }

    /// Fraction d'enregistrements ancrés à une cellule durable (cible 1.0).
    pub fn traced_ratio(&self) -> f64 {
        let total = self.records_total.load(Ordering::Relaxed);
        if total == 0 {
            return 1.0;
        }
        self.anchored_records.load(Ordering::Relaxed) as f64 / total as f64
    }

    /// Taille moyenne d'un enregistrement sérialisé (octets).
    pub fn mean_bytes(&self) -> f64 {
        let n = self.bytes_samples.load(Ordering::Relaxed);
        if n == 0 {
            return 0.0;
        }
        self.bytes_sum.load(Ordering::Relaxed) as f64 / n as f64
    }

    /// Overhead moyen par run (ns).
    pub fn mean_overhead_ns(&self) -> f64 {
        let n = self.overhead_samples.load(Ordering::Relaxed);
        if n == 0 {
            return 0.0;
        }
        self.overhead_ns_sum.load(Ordering::Relaxed) as f64 / n as f64
    }

    pub fn chain_breaks_total(&self) -> u64 {
        self.chain_breaks_total.load(Ordering::Relaxed)
    }

    pub fn snapshot(&self) -> ProvenanceMetricsSnapshot {
        ProvenanceMetricsSnapshot {
            records_total: self.records_total.load(Ordering::Relaxed),
            anchored_records: self.anchored_records.load(Ordering::Relaxed),
            captured_calls: self.captured_calls.load(Ordering::Relaxed),
            anchored_writes: self.anchored_writes.load(Ordering::Relaxed),
            chain_breaks_total: self.chain_breaks_total.load(Ordering::Relaxed),
            traced_ratio: self.traced_ratio(),
            mean_overhead_ns: self.mean_overhead_ns(),
            mean_bytes: self.mean_bytes(),
        }
    }

    /// Rendu Prometheus (texte, sans crate tierce) — même style que `SubstrateMetrics`.
    pub fn render_prometheus(&self) -> String {
        let s = self.snapshot();
        let mut o = String::new();
        let counter = |o: &mut String, name: &str, help: &str, v: u64| {
            o.push_str(&format!("# HELP {name} {help}\n# TYPE {name} counter\n{name} {v}\n"));
        };
        let gauge = |o: &mut String, name: &str, help: &str, v: f64| {
            o.push_str(&format!("# HELP {name} {help}\n# TYPE {name} gauge\n{name} {v}\n"));
        };
        counter(&mut o, "efe_swarm_prov_records_total", "Enregistrements de provenance scellés.", s.records_total);
        counter(&mut o, "efe_swarm_prov_anchored_records_total", "Enregistrements ancrés à une cellule.", s.anchored_records);
        counter(&mut o, "efe_swarm_prov_captured_calls_total", "Appels d'agent captés (seam B).", s.captured_calls);
        counter(&mut o, "efe_swarm_prov_anchored_writes_total", "Écritures ancrées (seam A).", s.anchored_writes);
        counter(&mut o, "efe_swarm_prov_chain_breaks_total", "Vérifications en échec.", s.chain_breaks_total);
        gauge(&mut o, "efe_swarm_prov_traced_ratio", "Fraction d'enregistrements tracés (cible 1.0).", s.traced_ratio);
        gauge(&mut o, "efe_swarm_prov_overhead_ns", "Overhead moyen par run (ns).", s.mean_overhead_ns);
        gauge(&mut o, "efe_swarm_prov_bytes", "Taille moyenne d'un enregistrement (octets).", s.mean_bytes);
        o
    }
}

/// Capture immuable + sérialisable des métriques.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProvenanceMetricsSnapshot {
    pub records_total: u64,
    pub anchored_records: u64,
    pub captured_calls: u64,
    pub anchored_writes: u64,
    pub chain_breaks_total: u64,
    pub traced_ratio: f64,
    pub mean_overhead_ns: f64,
    pub mean_bytes: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{AgentKind, AgentRole};
    use crate::recorder::{CallCapture, ProvenanceRecorder};
    use crate::{verify_swarm, HmacKeySource};
    use efe_blackboard::BlackboardWrite;
    use efe_nano_sdk::{AgentId, SwarmId, TenantId};
    use serde_json::json;
    use std::time::SystemTime;
    use uuid::Uuid;

    fn sealed() -> crate::recorder::SealedProvenance {
        let r = ProvenanceRecorder::new(SwarmId::from_uuid(Uuid::from_u128(2)), TenantId::system());
        for v in 0..3u64 {
            r.capture(CallCapture::new(
                AgentId::new("generator").unwrap(),
                AgentRole::Generator,
                AgentKind::ModeA,
                &json!({ "v": v }),
                &json!({ "o": v }),
            ));
            r.anchor(&BlackboardWrite {
                pointer: format!("/cand/{v}"),
                version: 1,
                agent_id: AgentId::new("generator").unwrap(),
                at: SystemTime::UNIX_EPOCH,
                bytes: 8,
            });
        }
        r.seal_with_key(HmacKeySource::test_key().derive(TenantId::system()).unwrap())
    }

    #[test]
    fn observe_sealed_increments_counters() {
        let m = ProvenanceMetrics::new();
        let s = sealed();
        m.observe_sealed(&s);
        let snap = m.snapshot();
        assert_eq!(snap.records_total, 3);
        assert_eq!(snap.anchored_records, 3);
        assert!((snap.traced_ratio - 1.0).abs() < 1e-9);
    }

    #[test]
    fn observe_verify_counts_breaks() {
        let m = ProvenanceMetrics::new();
        let s = sealed();
        let key = HmacKeySource::test_key().derive(TenantId::system()).unwrap();
        // run sain → pas de rupture.
        m.observe_verify(&verify_swarm(&s.records, &key));
        assert_eq!(m.chain_breaks_total(), 0);
        // mauvaise clé → rupture comptée.
        m.observe_verify(&verify_swarm(&s.records, &vec![0u8; 32]));
        assert_eq!(m.chain_breaks_total(), 1);
    }

    #[test]
    fn means_and_render() {
        let m = ProvenanceMetrics::new();
        m.observe_bytes(400);
        m.observe_bytes(600);
        m.observe_overhead_ns(1000);
        m.observe_overhead_ns(3000);
        assert!((m.mean_bytes() - 500.0).abs() < 1e-9);
        assert!((m.mean_overhead_ns() - 2000.0).abs() < 1e-9);
        let txt = m.render_prometheus();
        assert!(txt.contains("efe_swarm_prov_records_total"));
        assert!(txt.contains("efe_swarm_prov_traced_ratio"));
        assert!(txt.contains("efe_swarm_prov_overhead_ns"));
        assert!(txt.contains("efe_swarm_prov_bytes"));
        assert!(txt.contains("# TYPE efe_swarm_prov_traced_ratio gauge"));
    }

    #[test]
    fn snapshot_serde_round_trip() {
        let m = ProvenanceMetrics::new();
        m.observe_sealed(&sealed());
        let snap = m.snapshot();
        let s = serde_json::to_string(&snap).unwrap();
        let back: ProvenanceMetricsSnapshot = serde_json::from_str(&s).unwrap();
        assert_eq!(back, snap);
    }

    #[test]
    fn empty_metrics_are_sane() {
        let m = ProvenanceMetrics::new();
        assert_eq!(m.traced_ratio(), 1.0);
        assert_eq!(m.mean_bytes(), 0.0);
        assert_eq!(m.mean_overhead_ns(), 0.0);
    }
}
