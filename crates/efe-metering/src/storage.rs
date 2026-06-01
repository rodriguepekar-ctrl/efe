// SPDX-License-Identifier: UNLICENSED
//! M31 — Rollup storage abstraction + reference impl.
//!
//! The rollup runner reads every hour-bucket from the counter, sums
//! it into `usage_rollup_hour`, and (at day end) into
//! `usage_rollup_day`. The actual SQL inserts live in `efe-server`
//! (the PG-owning crate) ; this trait abstracts the storage so unit
//! tests don't need a live Postgres.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::error::Result;
use crate::meter::Meter;

/// Row persisted per (tenant, period_start, meter).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollupRow {
    /// Tenant identifier.
    pub tenant_id: String,
    /// Period start, ISO-8601 UTC. For an hour rollup this is the
    /// top of the hour ; for a day rollup the top of the day.
    pub period_start: String,
    /// Meter.
    pub meter: Meter,
    /// Aggregated counter value over the period.
    pub value: u64,
}

/// Trait for the rollup persistence layer.
#[async_trait]
pub trait RollupStorage: Send + Sync {
    /// Insert (or upsert) a batch of hour rollups.
    async fn upsert_hour(&self, rows: &[RollupRow]) -> Result<()>;

    /// Insert (or upsert) a batch of day rollups.
    async fn upsert_day(&self, rows: &[RollupRow]) -> Result<()>;

    /// Read the hour rollups for one tenant over a date range
    /// (inclusive both ends, ISO-8601). Returns rows in
    /// (period_start ASC, meter ASC) order.
    async fn list_hours(
        &self,
        tenant: &str,
        since: &str,
        until: &str,
    ) -> Result<Vec<RollupRow>>;

    /// Read the day rollups for one tenant over a date range.
    async fn list_days(
        &self,
        tenant: &str,
        since: &str,
        until: &str,
    ) -> Result<Vec<RollupRow>>;
}

// ─────────────────────────── IN-MEMORY ───────────────────────────

/// Type alias for the `(tenant_id, period_start, meter)` →
/// value map used by [`InMemoryStorage`]. Pulled out for clippy
/// "very complex type" + readability.
pub type InMemoryRollupMap = HashMap<(String, String, Meter), u64>;

/// Test-only in-memory storage. Keyed by (tenant, period_start,
/// meter) → value.
#[derive(Default, Clone)]
pub struct InMemoryStorage {
    hours: std::sync::Arc<tokio::sync::Mutex<InMemoryRollupMap>>,
    days: std::sync::Arc<tokio::sync::Mutex<InMemoryRollupMap>>,
}

impl InMemoryStorage {
    /// Construct a fresh empty storage.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl RollupStorage for InMemoryStorage {
    async fn upsert_hour(&self, rows: &[RollupRow]) -> Result<()> {
        let mut g = self.hours.lock().await;
        for r in rows {
            g.insert(
                (r.tenant_id.clone(), r.period_start.clone(), r.meter),
                r.value,
            );
        }
        Ok(())
    }

    async fn upsert_day(&self, rows: &[RollupRow]) -> Result<()> {
        let mut g = self.days.lock().await;
        for r in rows {
            g.insert(
                (r.tenant_id.clone(), r.period_start.clone(), r.meter),
                r.value,
            );
        }
        Ok(())
    }

    async fn list_hours(
        &self,
        tenant: &str,
        since: &str,
        until: &str,
    ) -> Result<Vec<RollupRow>> {
        let g = self.hours.lock().await;
        let mut out: Vec<RollupRow> = g
            .iter()
            .filter(|((t, ps, _), _)| {
                t == tenant && ps.as_str() >= since && ps.as_str() <= until
            })
            .map(|((t, ps, m), v)| RollupRow {
                tenant_id: t.clone(),
                period_start: ps.clone(),
                meter: *m,
                value: *v,
            })
            .collect();
        out.sort_by(|a, b| {
            a.period_start
                .cmp(&b.period_start)
                .then_with(|| a.meter.name().cmp(b.meter.name()))
        });
        Ok(out)
    }

    async fn list_days(
        &self,
        tenant: &str,
        since: &str,
        until: &str,
    ) -> Result<Vec<RollupRow>> {
        let g = self.days.lock().await;
        let mut out: Vec<RollupRow> = g
            .iter()
            .filter(|((t, ps, _), _)| {
                t == tenant && ps.as_str() >= since && ps.as_str() <= until
            })
            .map(|((t, ps, m), v)| RollupRow {
                tenant_id: t.clone(),
                period_start: ps.clone(),
                meter: *m,
                value: *v,
            })
            .collect();
        out.sort_by(|a, b| {
            a.period_start
                .cmp(&b.period_start)
                .then_with(|| a.meter.name().cmp(b.meter.name()))
        });
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn upsert_and_list_hour() {
        let s = InMemoryStorage::new();
        let r = RollupRow {
            tenant_id: "t".into(),
            period_start: "2026-05-11T12:00:00Z".into(),
            meter: Meter::ClassifyRequests,
            value: 7,
        };
        s.upsert_hour(&[r.clone()]).await.unwrap();
        let got = s
            .list_hours("t", "2026-05-11T00:00:00Z", "2026-05-11T23:59:59Z")
            .await
            .unwrap();
        assert_eq!(got, vec![r]);
    }

    #[tokio::test]
    async fn list_filters_by_tenant_and_range() {
        let s = InMemoryStorage::new();
        let row = |t: &str, ps: &str| RollupRow {
            tenant_id: t.into(),
            period_start: ps.into(),
            meter: Meter::ClassifyRequests,
            value: 1,
        };
        s.upsert_hour(&[
            row("t1", "2026-05-11T10:00:00Z"),
            row("t1", "2026-05-12T10:00:00Z"),
            row("t2", "2026-05-11T10:00:00Z"),
        ])
        .await
        .unwrap();
        let got = s
            .list_hours("t1", "2026-05-11T00:00:00Z", "2026-05-11T23:59:59Z")
            .await
            .unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].tenant_id, "t1");
    }

    #[tokio::test]
    async fn day_rollups_isolated_from_hour_rollups() {
        let s = InMemoryStorage::new();
        s.upsert_hour(&[RollupRow {
            tenant_id: "t".into(),
            period_start: "2026-05-11T12:00:00Z".into(),
            meter: Meter::ClassifyRequests,
            value: 10,
        }])
        .await
        .unwrap();
        // Day rollups must be empty.
        let days = s
            .list_days("t", "2026-05-11", "2026-05-11")
            .await
            .unwrap();
        assert!(days.is_empty());
    }
}
