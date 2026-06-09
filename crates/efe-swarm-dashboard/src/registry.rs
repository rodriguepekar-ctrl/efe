// SPDX-License-Identifier: UNLICENSED
//! `DashboardRegistry` — `run_id → DashboardBus`, **tenant-scopé**, borné.
//!
//! Cycle de vie distinct du `RunStore` (durable, la preuve) : le registry est **éphémère**
//! (le live), borné (drop-oldest) et isolé par tenant. La passerelle l'enregistre au lancement
//! d'un job et le purge après la fin du run (+ grâce).

use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use uuid::Uuid;

use efe_nano_sdk::TenantId;

use crate::bus::DashboardBus;

/// TTL par défaut d'un bus dans le registre (grâce après fin de run pour les viewers tardifs).
pub const DEFAULT_TTL: Duration = Duration::from_secs(600);

struct Entry {
    tenant: TenantId,
    bus: Arc<DashboardBus>,
    created: Instant,
}

/// Registre **borné + TTL** des bus de dashboard actifs (par run), isolé par tenant.
pub struct DashboardRegistry {
    runs: DashMap<Uuid, Entry>,
    cap: usize,
    ttl: Duration,
}

impl DashboardRegistry {
    /// Registre borné à `cap` runs simultanés (drop-oldest au-delà), TTL par défaut.
    pub fn new(cap: usize) -> Self {
        Self::with_ttl(cap, DEFAULT_TTL)
    }

    /// Registre borné à `cap` runs + `ttl` (les bus expirés sont purgés).
    pub fn with_ttl(cap: usize, ttl: Duration) -> Self {
        Self { runs: DashMap::new(), cap: cap.max(1), ttl }
    }

    /// Enregistre le bus d'un run (purge d'abord les expirés, puis le plus ancien si plein).
    pub fn insert(&self, bus: Arc<DashboardBus>) {
        self.prune_expired();
        self.prune_if_full();
        self.runs.insert(bus.run_id(), Entry { tenant: bus.tenant(), bus: bus.clone(), created: Instant::now() });
    }

    /// Récupère le bus d'un run **pour ce tenant** (isolation : invisible sinon).
    pub fn get(&self, run_id: Uuid, tenant: &TenantId) -> Option<Arc<DashboardBus>> {
        self.runs.get(&run_id).filter(|e| &e.tenant == tenant).map(|e| e.bus.clone())
    }

    /// Tous les bus actifs **de ce tenant** (firehose `/events`).
    pub fn active_for(&self, tenant: &TenantId) -> Vec<Arc<DashboardBus>> {
        self.runs.iter().filter(|e| &e.value().tenant == tenant).map(|e| e.value().bus.clone()).collect()
    }

    /// Retire un run (après fin + grâce).
    pub fn remove(&self, run_id: Uuid) {
        self.runs.remove(&run_id);
    }

    pub fn len(&self) -> usize {
        self.runs.len()
    }
    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    /// Total des abonnés SSE en vol, tous runs confondus (jauge `…_active_subscribers`).
    pub fn total_subscribers(&self) -> usize {
        self.runs.iter().map(|e| e.value().bus.subscriber_count()).sum()
    }

    /// Abonnés SSE en vol **pour un tenant** (plafond anti-DoS, §10).
    pub fn subscribers_for(&self, tenant: &TenantId) -> usize {
        self.runs.iter().filter(|e| &e.value().tenant == tenant).map(|e| e.value().bus.subscriber_count()).sum()
    }

    /// Purge les bus expirés (créés il y a plus que `ttl`).
    pub fn prune_expired(&self) {
        let ttl = self.ttl;
        self.runs.retain(|_, e| e.created.elapsed() < ttl);
    }

    fn prune_if_full(&self) {
        while self.runs.len() >= self.cap {
            let oldest = self.runs.iter().min_by_key(|e| e.value().created).map(|e| *e.key());
            match oldest {
                Some(id) => {
                    self.runs.remove(&id);
                }
                None => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bus(run: u128, tenant: TenantId) -> Arc<DashboardBus> {
        DashboardBus::new(tenant, Uuid::from_u128(run), 64)
    }

    #[test]
    fn get_is_tenant_scoped() {
        let reg = DashboardRegistry::new(8);
        let ta = TenantId::from_uuid(Uuid::from_u128(0xA));
        let tb = TenantId::from_uuid(Uuid::from_u128(0xB));
        let b = bus(1, ta);
        reg.insert(b.clone());
        assert!(reg.get(Uuid::from_u128(1), &ta).is_some());
        assert!(reg.get(Uuid::from_u128(1), &tb).is_none(), "isolation tenant");
        assert!(reg.get(Uuid::from_u128(999), &ta).is_none());
    }

    #[test]
    fn bounded_drops_oldest() {
        let reg = DashboardRegistry::new(2);
        let t = TenantId::system();
        reg.insert(bus(1, t));
        reg.insert(bus(2, t));
        reg.insert(bus(3, t)); // évince le plus ancien (run 1)
        assert_eq!(reg.len(), 2);
        assert!(reg.get(Uuid::from_u128(1), &t).is_none());
        assert!(reg.get(Uuid::from_u128(3), &t).is_some());
    }

    #[test]
    fn active_for_lists_tenant_runs() {
        let reg = DashboardRegistry::new(8);
        let t = TenantId::system();
        reg.insert(bus(1, t));
        reg.insert(bus(2, t));
        assert_eq!(reg.active_for(&t).len(), 2);
        reg.remove(Uuid::from_u128(1));
        assert_eq!(reg.active_for(&t).len(), 1);
    }

    #[test]
    fn ttl_purges_expired_buses() {
        let reg = DashboardRegistry::with_ttl(8, Duration::from_millis(5));
        let t = TenantId::system();
        reg.insert(bus(1, t));
        std::thread::sleep(Duration::from_millis(12));
        reg.insert(bus(2, t)); // insert purge d'abord les expirés → run 1 évincé (TTL)
        assert!(reg.get(Uuid::from_u128(1), &t).is_none(), "run expiré purgé (TTL)");
        assert!(reg.get(Uuid::from_u128(2), &t).is_some());
    }

    #[test]
    fn subscribers_for_is_tenant_scoped() {
        let reg = DashboardRegistry::new(8);
        let ta = TenantId::from_uuid(Uuid::from_u128(0xA));
        let b = bus(1, ta);
        let _rx = b.subscribe(); // 1 abonné en vol
        reg.insert(b);
        assert_eq!(reg.subscribers_for(&ta), 1);
        assert_eq!(reg.subscribers_for(&TenantId::system()), 0);
    }
}
