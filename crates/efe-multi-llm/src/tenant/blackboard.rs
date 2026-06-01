// SPDX-License-Identifier: UNLICENSED
//! `TenantPheromoneBlackboard` — wrap `DashMap<TenantId, PheromoneBlackboard>`.
//!
//! ## Invariant I12
//!
//! Isolation physique : un tenant A NE PEUT JAMAIS lire les phéromones d'un
//! tenant B.  Le compilateur force l'API à passer par `for_tenant(t)` qui
//! retourne le blackboard SOUS-JACENT, qui est lui-même un type
//! `PheromoneBlackboard` — sans aucun moyen de croiser les tenants
//! au sein d'un même handle.

use std::sync::Arc;

use dashmap::DashMap;
use uuid::Uuid;

use efe_coordinator::PheromoneBlackboard;

/// Type pour clarté.
pub(crate) type TenantId = Uuid;

/// Blackboard partitionnée par tenant.
#[derive(Clone, Default)]
pub struct TenantPheromoneBlackboard {
    per_tenant: Arc<DashMap<TenantId, PheromoneBlackboard>>,
}

impl TenantPheromoneBlackboard {
    /// Construit vide.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Récupère (ou crée vide) la blackboard pour un tenant donné.
    ///
    /// **Pas d'accès cross-tenant possible** : la map interne est
    /// strictement keyed par `TenantId`.
    #[must_use]
    pub fn for_tenant(&self, t: TenantId) -> PheromoneBlackboard {
        self.per_tenant.entry(t).or_default().value().clone()
    }

    /// Nombre de tenants ayant déjà au moins 1 entry.
    #[must_use]
    pub fn tenant_count(&self) -> usize {
        self.per_tenant.len()
    }

    /// Liste des tenant IDs présents.
    #[must_use]
    pub fn tenant_ids(&self) -> Vec<TenantId> {
        self.per_tenant.iter().map(|e| *e.key()).collect()
    }

    /// Supprime un tenant (et son blackboard) — utile pour la
    /// suppression GDPR / cleanup.
    pub fn drop_tenant(&self, t: TenantId) {
        self.per_tenant.remove(&t);
    }

    /// Total bytes utilisés par tous les tenants confondus (observabilité).
    #[must_use]
    pub fn total_bytes_used(&self) -> usize {
        self.per_tenant
            .iter()
            .map(|e| e.value().bytes_used())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_coordinator::{BlackboardEntry, EntryKind};

    #[test]
    fn fresh_is_empty() {
        let bb = TenantPheromoneBlackboard::new();
        assert_eq!(bb.tenant_count(), 0);
        assert_eq!(bb.total_bytes_used(), 0);
    }

    #[test]
    fn deposit_per_tenant_isolated() {
        let bb = TenantPheromoneBlackboard::new();
        let t_a = Uuid::now_v7();
        let t_b = Uuid::now_v7();
        let board_a = bb.for_tenant(t_a);
        let board_b = bb.for_tenant(t_b);
        board_a
            .deposit(BlackboardEntry::new(
                "x",
                EntryKind::Trace,
                serde_json::json!({"tenant": "A"}),
            ))
            .unwrap();
        // Tenant B doit avoir 0 entries.
        assert_eq!(board_a.len(), 1);
        assert_eq!(board_b.len(), 0);
    }

    #[test]
    fn for_tenant_returns_same_instance_on_repeat() {
        let bb = TenantPheromoneBlackboard::new();
        let t = Uuid::now_v7();
        let a = bb.for_tenant(t);
        a.deposit(BlackboardEntry::new(
            "x",
            EntryKind::Trace,
            serde_json::json!({}),
        ))
        .unwrap();
        let b = bb.for_tenant(t);
        // Même tenant ID → même blackboard sous-jacente.
        assert_eq!(b.len(), 1);
    }

    #[test]
    fn drop_tenant_removes_blackboard() {
        let bb = TenantPheromoneBlackboard::new();
        let t = Uuid::now_v7();
        let board = bb.for_tenant(t);
        board
            .deposit(BlackboardEntry::new(
                "x",
                EntryKind::Trace,
                serde_json::json!({}),
            ))
            .unwrap();
        assert_eq!(bb.tenant_count(), 1);
        bb.drop_tenant(t);
        assert_eq!(bb.tenant_count(), 0);
    }

    #[test]
    fn total_bytes_used_aggregates() {
        let bb = TenantPheromoneBlackboard::new();
        for _ in 0..3 {
            let t = Uuid::now_v7();
            bb.for_tenant(t)
                .deposit(BlackboardEntry::new(
                    "x",
                    EntryKind::Trace,
                    serde_json::json!({"pad": "x".repeat(1000)}),
                ))
                .unwrap();
        }
        assert!(bb.total_bytes_used() > 3000);
    }

    #[test]
    fn tenant_ids_listing() {
        let bb = TenantPheromoneBlackboard::new();
        let t1 = Uuid::now_v7();
        let t2 = Uuid::now_v7();
        let _ = bb.for_tenant(t1);
        let _ = bb.for_tenant(t2);
        let ids = bb.tenant_ids();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&t1));
        assert!(ids.contains(&t2));
    }
}
