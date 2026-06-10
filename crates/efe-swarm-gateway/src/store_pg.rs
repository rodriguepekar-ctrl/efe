// SPDX-License-Identifier: UNLICENSED
//! `PgRunStore` — la **preuve durable** en Postgres (feature `postgres`).
//!
//! Table `swarm_runs { id, tenant, records JSONB, root, output JSONB, topology, created_at }`.
//! Les `records` sont **write-once** (`ON CONFLICT DO NOTHING`) — de toute façon, toute
//! mutation est détectée par `verify_swarm` (le moat ne fait pas confiance au store). Lecture
//! **tenant-scopée** (isolation). Réutilise le pattern `PgPool` d'`efe-server`.

use async_trait::async_trait;
use efe_swarm_sdk::{AgentCallRecord, RunOutcome};
use serde_json::Value;
use sqlx::types::Json;
use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use std::time::{Duration, UNIX_EPOCH};
use uuid::Uuid;

use crate::error::GatewayError;
use crate::store::{RunStore, StoredRun};

const DDL: &str = "CREATE TABLE IF NOT EXISTS swarm_runs (\
    id UUID PRIMARY KEY,\
    tenant TEXT NOT NULL,\
    records JSONB NOT NULL,\
    root TEXT,\
    output JSONB NOT NULL,\
    topology TEXT NOT NULL,\
    created_at BIGINT NOT NULL)";
const DDL_INDEX: &str = "CREATE INDEX IF NOT EXISTS swarm_runs_tenant_idx ON swarm_runs (tenant)";

/// Store Postgres des runs (prod). Les records sont immuables (write-once).
pub struct PgRunStore {
    pool: PgPool,
}

impl PgRunStore {
    /// Réutilise un `PgPool` existant (ex. celui d'`efe-server`).
    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Ouvre un pool (8 connexions) depuis une URL Postgres.
    pub async fn connect(url: &str) -> Result<Self, GatewayError> {
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .connect(url)
            .await
            .map_err(|e| GatewayError::Internal(format!("connexion Postgres : {e}")))?;
        Ok(Self { pool })
    }

    /// Crée la table + l'index si absents (idempotent).
    pub async fn migrate(&self) -> Result<(), GatewayError> {
        sqlx::query(DDL)
            .execute(&self.pool)
            .await
            .map_err(|e| GatewayError::Internal(format!("migration (table) : {e}")))?;
        sqlx::query(DDL_INDEX)
            .execute(&self.pool)
            .await
            .map_err(|e| GatewayError::Internal(format!("migration (index) : {e}")))?;
        Ok(())
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

#[async_trait]
impl RunStore for PgRunStore {
    async fn put(&self, id: Uuid, tenant: &str, outcome: &RunOutcome) -> Result<(), GatewayError> {
        let stored = StoredRun::from_outcome(tenant, outcome);
        sqlx::query(
            "INSERT INTO swarm_runs (id, tenant, records, root, output, topology, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (id) DO NOTHING",
        )
        .bind(id)
        .bind(&stored.tenant)
        .bind(Json(&stored.records))
        .bind(&stored.root)
        .bind(Json(&stored.output))
        .bind(&stored.topology)
        .bind(stored.created_at_unix() as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| GatewayError::Internal(format!("store.put : {e}")))?;
        Ok(())
    }

    async fn get(&self, id: Uuid, tenant: &str) -> Result<Option<StoredRun>, GatewayError> {
        // Isolation tenant : un run d'un autre tenant est INVISIBLE (clause AND tenant).
        let row = sqlx::query(
            "SELECT records, root, output, topology, created_at \
             FROM swarm_runs WHERE id = $1 AND tenant = $2",
        )
        .bind(id)
        .bind(tenant)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| GatewayError::Internal(format!("store.get : {e}")))?;

        let Some(row) = row else {
            return Ok(None);
        };
        let map = |e: sqlx::Error| GatewayError::Internal(format!("décodage ligne : {e}"));
        let records: Json<Vec<AgentCallRecord>> = row.try_get("records").map_err(map)?;
        let output: Json<Value> = row.try_get("output").map_err(map)?;
        let root: Option<String> = row.try_get("root").map_err(map)?;
        let topology: String = row.try_get("topology").map_err(map)?;
        let created_at_unix: i64 = row.try_get("created_at").map_err(map)?;

        Ok(Some(StoredRun {
            tenant: tenant.to_string(),
            records: records.0,
            root,
            output: output.0,
            topology,
            created_at: UNIX_EPOCH + Duration::from_secs(created_at_unix.max(0) as u64),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_swarm_sdk::{Budget, Swarm, TopologySpec};
    use serde_json::json;

    /// Intégration RÉELLE (Postgres). Ignoré par défaut ; requiert `DATABASE_URL`.
    /// Lancer : `DATABASE_URL=postgres://… cargo test -p efe-swarm-gateway --features postgres -- --ignored pg_roundtrip`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "nécessite un Postgres (DATABASE_URL)"]
    async fn pg_roundtrip_is_tenant_scoped() {
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
        let store = PgRunStore::connect(&url).await.unwrap();
        store.migrate().await.unwrap();

        let outcome = Swarm::keyless()
            .run_traced(json!({ "text": "x", "expected": "x" }), TopologySpec::Auto, Budget::cents(100))
            .await
            .unwrap();
        let id = Uuid::new_v4();
        store.put(id, "tenant-a", &outcome).await.unwrap();

        // tenant-A lit son run ; tenant-B ne voit RIEN (isolation).
        let got = store.get(id, "tenant-a").await.unwrap().expect("run présent");
        assert!(!got.records.is_empty());
        assert!(store.get(id, "tenant-b").await.unwrap().is_none());

        // write-once : un second put du même id ne réécrit pas (ON CONFLICT DO NOTHING).
        store.put(id, "tenant-a", &outcome).await.unwrap();
    }
}
