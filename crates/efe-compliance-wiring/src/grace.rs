// SPDX-License-Identifier: UNLICENSED
//! **Sprint 16 — N24 + ADR-4 + G14 + I19** : tenant grace window
//! resolver.
//!
//! Tenants migrating from pre-N18 deployments may have catalogue
//! entries WITHOUT signatures.  Forcing a same-day cutover would
//! break adoption.  The grace window lets them keep dispatching
//! under `experimental_relaxed` until a per-tenant deadline.
//!
//! Sprint 16 ships an in-memory resolver (set deadlines via
//! `WiredComplianceRunner::set_grace_deadline`).  Sprint 17 wires
//! a persistent store (Postgres-backed via N20).

use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use uuid::Uuid;

/// **N24 + ADR-4** : 90-day default grace window.
pub const DEFAULT_GRACE_WINDOW_DAYS: i64 = 90;

/// **N24 + G14** : grace window resolver trait.
///
/// `in_grace_window(tenant_id, at)` returns `true` if the tenant
/// is still within the per-tenant compliance deadline ; the gate
/// then skips signature + gold verification for that tenant.
#[async_trait]
pub trait GraceWindowResolver: Send + Sync + 'static {
    /// **G14** : true if the tenant is currently within the grace
    /// window.
    async fn in_grace_window(&self, tenant_id: Uuid, at: SystemTime) -> bool;

    /// **G14** : the tenant's deadline if set.  `None` means "no
    /// grace was ever granted" — the gate fails closed for
    /// unsigned entries.
    async fn deadline_for(&self, tenant_id: Uuid) -> Option<DateTime<Utc>>;
}

/// **N24** : an in-memory grace window store.
pub struct InMemoryGraceWindow {
    deadlines: Arc<DashMap<Uuid, DateTime<Utc>>>,
}

impl std::fmt::Debug for InMemoryGraceWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryGraceWindow")
            .field("len", &self.deadlines.len())
            .finish()
    }
}

impl Default for InMemoryGraceWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryGraceWindow {
    /// **N24** : empty grace store.
    pub fn new() -> Self {
        Self {
            deadlines: Arc::new(DashMap::new()),
        }
    }

    /// **N24 + G14** : set a tenant's deadline.
    pub fn set_deadline(&self, tenant_id: Uuid, deadline: DateTime<Utc>) {
        self.deadlines.insert(tenant_id, deadline);
    }

    /// **N24 + G14** : convenience — grant `DEFAULT_GRACE_WINDOW_DAYS`
    /// from `now`.
    pub fn grant_default_grace(&self, tenant_id: Uuid, now: DateTime<Utc>) {
        let deadline = now + chrono::Duration::days(DEFAULT_GRACE_WINDOW_DAYS);
        self.set_deadline(tenant_id, deadline);
    }

    /// **N24 + G14** : remove a tenant's grace.  Subsequent
    /// dispatches fail closed for unsigned entries.
    pub fn revoke(&self, tenant_id: Uuid) -> bool {
        self.deadlines.remove(&tenant_id).is_some()
    }

    /// **N24** : number of tenants with a grace deadline set.
    pub fn len(&self) -> usize {
        self.deadlines.len()
    }

    /// **N24** : true if no tenants have grace.
    pub fn is_empty(&self) -> bool {
        self.deadlines.is_empty()
    }
}

#[async_trait]
impl GraceWindowResolver for InMemoryGraceWindow {
    async fn in_grace_window(&self, tenant_id: Uuid, at: SystemTime) -> bool {
        let Some(deadline) = self.deadline_for(tenant_id).await else {
            return false;
        };
        let at_utc: DateTime<Utc> = at.into();
        at_utc < deadline
    }

    async fn deadline_for(&self, tenant_id: Uuid) -> Option<DateTime<Utc>> {
        self.deadlines.get(&tenant_id).map(|e| *e.value())
    }
}

/// **N24** : a "never grant grace" resolver — every unsigned
/// entry fails closed.  Used by the strict tier1 policy.
pub struct DenyAllGrace;

impl std::fmt::Debug for DenyAllGrace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DenyAllGrace")
    }
}

#[async_trait]
impl GraceWindowResolver for DenyAllGrace {
    async fn in_grace_window(&self, _tenant_id: Uuid, _at: SystemTime) -> bool {
        false
    }
    async fn deadline_for(&self, _tenant_id: Uuid) -> Option<DateTime<Utc>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_store_returns_no_grace() {
        let g = InMemoryGraceWindow::new();
        let t = Uuid::new_v4();
        assert!(!g.in_grace_window(t, SystemTime::now()).await);
        assert!(g.deadline_for(t).await.is_none());
        assert!(g.is_empty());
    }

    #[tokio::test]
    async fn set_then_in_window_true() {
        let g = InMemoryGraceWindow::new();
        let t = Uuid::new_v4();
        let now = Utc::now();
        g.set_deadline(t, now + chrono::Duration::days(30));
        assert!(g.in_grace_window(t, SystemTime::now()).await);
    }

    #[tokio::test]
    async fn past_deadline_returns_false() {
        let g = InMemoryGraceWindow::new();
        let t = Uuid::new_v4();
        let now = Utc::now();
        g.set_deadline(t, now - chrono::Duration::days(1));
        assert!(!g.in_grace_window(t, SystemTime::now()).await);
    }

    #[tokio::test]
    async fn grant_default_grace_uses_90_days() {
        let g = InMemoryGraceWindow::new();
        let t = Uuid::new_v4();
        let now = Utc::now();
        g.grant_default_grace(t, now);
        let d = g.deadline_for(t).await.unwrap();
        let delta = d - now;
        assert_eq!(delta.num_days(), DEFAULT_GRACE_WINDOW_DAYS);
    }

    #[tokio::test]
    async fn revoke_removes_grace() {
        let g = InMemoryGraceWindow::new();
        let t = Uuid::new_v4();
        g.grant_default_grace(t, Utc::now());
        assert!(g.revoke(t));
        assert!(g.deadline_for(t).await.is_none());
        assert!(!g.revoke(t));
    }

    #[tokio::test]
    async fn deny_all_grace_never_grants() {
        let g = DenyAllGrace;
        assert!(!g.in_grace_window(Uuid::new_v4(), SystemTime::now()).await);
        assert!(g.deadline_for(Uuid::new_v4()).await.is_none());
    }

    #[tokio::test]
    async fn deadline_boundary_excludes_equal_timestamp() {
        let g = InMemoryGraceWindow::new();
        let t = Uuid::new_v4();
        let now = Utc::now();
        let deadline = now + chrono::Duration::seconds(60);
        g.set_deadline(t, deadline);
        let at: SystemTime = deadline.into();
        // `at_utc < deadline` is false when equal — the boundary
        // is exclusive.
        assert!(!g.in_grace_window(t, at).await);
    }

    #[test]
    fn debug_format_no_panic() {
        let g = InMemoryGraceWindow::new();
        let _ = format!("{g:?}");
        let d = DenyAllGrace;
        let _ = format!("{d:?}");
    }
}
