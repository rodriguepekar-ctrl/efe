// SPDX-License-Identifier: UNLICENSED
//! `PgRunStore` — la **preuve durable** en Postgres (feature `postgres`).
//!
//! Table `swarm_runs { id, tenant, records JSONB, root, output JSONB, topology, created_at }`.
//! Les `records` sont **write-once** (`ON CONFLICT DO NOTHING`) — de toute façon, toute
//! mutation est détectée par `verify_swarm` (le moat ne fait pas confiance au store). Lecture
//! **tenant-scopée** (isolation). Réutilise le pattern `PgPool` d'`efe-server`.

use async_trait::async_trait;
use efe_swarm_sdk::{AgentCallRecord, LlmProvenanceRecord, RunOutcome};
use serde_json::Value;
use sqlx::types::Json;
use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use std::time::{Duration, UNIX_EPOCH};
use uuid::Uuid;

use crate::error::GatewayError;
use crate::store::{RunStore, StoredRun};

// ADR-016 : colonnes N17 ajoutées (llm_records, n17_leaves, n17_root). DEFAULT pour la
// compatibilité ascendante — les runs persistés avant le lot ont 0 forensique (run Mode A).
const DDL: &str = "CREATE TABLE IF NOT EXISTS swarm_runs (\
    id UUID PRIMARY KEY,\
    tenant TEXT NOT NULL,\
    records JSONB NOT NULL,\
    root TEXT,\
    output JSONB NOT NULL,\
    topology TEXT NOT NULL,\
    created_at BIGINT NOT NULL,\
    llm_records JSONB NOT NULL DEFAULT '[]'::jsonb,\
    n17_leaves JSONB NOT NULL DEFAULT '[]'::jsonb,\
    n17_root TEXT)";
const DDL_INDEX: &str = "CREATE INDEX IF NOT EXISTS swarm_runs_tenant_idx ON swarm_runs (tenant)";
// ADR-016 : migration en place d'une table pré-existante (idempotent — IF NOT EXISTS).
const DDL_ALTER: &[&str] = &[
    "ALTER TABLE swarm_runs ADD COLUMN IF NOT EXISTS llm_records JSONB NOT NULL DEFAULT '[]'::jsonb",
    "ALTER TABLE swarm_runs ADD COLUMN IF NOT EXISTS n17_leaves JSONB NOT NULL DEFAULT '[]'::jsonb",
    "ALTER TABLE swarm_runs ADD COLUMN IF NOT EXISTS n17_root TEXT",
];

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
        // ADR-016 : ajoute les colonnes N17 si la table préexiste (idempotent).
        for stmt in DDL_ALTER {
            sqlx::query(stmt)
                .execute(&self.pool)
                .await
                .map_err(|e| GatewayError::Internal(format!("migration (N17) : {e}")))?;
        }
        Ok(())
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// **Test-only (ADR-016)** : insère un [`StoredRun`] déjà construit (un run Mode C
    /// synthétique à forensique N17 **non vide**), pour éprouver le round-trip JSONB des
    /// colonnes N17 sur un vrai Postgres, **sans Ollama**. Même INSERT que `put`, mais prend
    /// le `StoredRun` directement (court-circuite `from_outcome`).
    #[cfg(any(test, feature = "tamper-testkit"))]
    pub async fn insert_stored(&self, id: Uuid, stored: &StoredRun) -> Result<(), GatewayError> {
        sqlx::query(
            "INSERT INTO swarm_runs \
             (id, tenant, records, root, output, topology, created_at, llm_records, n17_leaves, n17_root) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) ON CONFLICT (id) DO NOTHING",
        )
        .bind(id)
        .bind(&stored.tenant)
        .bind(Json(&stored.records))
        .bind(&stored.root)
        .bind(Json(&stored.output))
        .bind(&stored.topology)
        .bind(stored.created_at_unix() as i64)
        .bind(Json(&stored.llm_records))
        .bind(Json(&stored.n17_leaves))
        .bind(&stored.n17_root)
        .execute(&self.pool)
        .await
        .map_err(|e| GatewayError::Internal(format!("insert_stored : {e}")))?;
        Ok(())
    }
}

#[async_trait]
impl RunStore for PgRunStore {
    async fn put(&self, id: Uuid, tenant: &str, outcome: &RunOutcome) -> Result<(), GatewayError> {
        let stored = StoredRun::from_outcome(tenant, outcome);
        sqlx::query(
            "INSERT INTO swarm_runs \
             (id, tenant, records, root, output, topology, created_at, llm_records, n17_leaves, n17_root) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) ON CONFLICT (id) DO NOTHING",
        )
        .bind(id)
        .bind(&stored.tenant)
        .bind(Json(&stored.records))
        .bind(&stored.root)
        .bind(Json(&stored.output))
        .bind(&stored.topology)
        .bind(stored.created_at_unix() as i64)
        .bind(Json(&stored.llm_records)) // ADR-016 : la forensique N17, durable
        .bind(Json(&stored.n17_leaves))
        .bind(&stored.n17_root)
        .execute(&self.pool)
        .await
        .map_err(|e| GatewayError::Internal(format!("store.put : {e}")))?;
        Ok(())
    }

    async fn put_stored(&self, id: Uuid, stored: StoredRun) -> Result<(), GatewayError> {
        // **Couche 2 (M2)** : même INSERT write-once que `put`, mais depuis un `StoredRun`
        // déjà construit par `mission_graph` (pas de `from_outcome`).
        sqlx::query(
            "INSERT INTO swarm_runs \
             (id, tenant, records, root, output, topology, created_at, llm_records, n17_leaves, n17_root) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) ON CONFLICT (id) DO NOTHING",
        )
        .bind(id)
        .bind(&stored.tenant)
        .bind(Json(&stored.records))
        .bind(&stored.root)
        .bind(Json(&stored.output))
        .bind(&stored.topology)
        .bind(stored.created_at_unix() as i64)
        .bind(Json(&stored.llm_records))
        .bind(Json(&stored.n17_leaves))
        .bind(&stored.n17_root)
        .execute(&self.pool)
        .await
        .map_err(|e| GatewayError::Internal(format!("store.put_stored : {e}")))?;
        Ok(())
    }

    async fn get(&self, id: Uuid, tenant: &str) -> Result<Option<StoredRun>, GatewayError> {
        // Isolation tenant : un run d'un autre tenant est INVISIBLE (clause AND tenant).
        let row = sqlx::query(
            "SELECT records, root, output, topology, created_at, llm_records, n17_leaves, n17_root \
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
        // ADR-016 : la forensique N17 (DEFAULT '[]' pour les runs d'avant le lot).
        let llm_records: Json<Vec<LlmProvenanceRecord>> = row.try_get("llm_records").map_err(map)?;
        let n17_leaves: Json<Vec<String>> = row.try_get("n17_leaves").map_err(map)?;
        let n17_root: Option<String> = row.try_get("n17_root").map_err(map)?;

        Ok(Some(StoredRun {
            tenant: tenant.to_string(),
            records: records.0,
            root,
            llm_records: llm_records.0,
            n17_leaves: n17_leaves.0,
            n17_root,
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

    /// **ADR-016 §8.1 ① (volet Postgres)** — round-trip d'un run Mode C à forensique N17 **NON
    /// vide** : prouve que le bind/décodage sqlx↔JSONB des trois colonnes (llm_records,
    /// n17_leaves, n17_root) survit, et que la re-preuve hors ligne tient depuis le store PG
    /// relu. Ignoré par défaut ; requiert `DATABASE_URL`. Sans ça, le mapping JSONB des
    /// colonnes N17 n'était exercé que keyless (colonnes vides).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "nécessite un Postgres (DATABASE_URL)"]
    async fn pg_roundtrips_nonempty_n17_forensics() {
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
        let store = PgRunStore::connect(&url).await.unwrap();
        store.migrate().await.unwrap();

        // run Mode C synthétique (forensique NON vide) — helper partagé avec les tests lib.
        let (_recs, stored) = crate::test_support::synthetic_mode_c_run("gemma3:12b");
        let produced = stored.llm_records.len();
        let tenant = stored.tenant.clone();
        let expected_leaves = stored.n17_leaves.clone();
        let expected_root = stored.n17_root.clone();
        assert!(produced >= 1 && expected_root.is_some(), "le run synthétique EST Mode C");
        let id = Uuid::new_v4();
        store.insert_stored(id, &stored).await.unwrap();

        // relecture : les TROIS colonnes N17 (JSONB + TEXT) survivent au round-trip sqlx réel.
        let got = store.get(id, &tenant).await.unwrap().expect("run présent");
        assert_eq!(got.llm_records.len(), produced, "forensique N17 relue (compte == produit)");
        assert_eq!(got.llm_records, stored.llm_records, "records N17 identiques après JSONB");
        assert_eq!(got.n17_leaves, expected_leaves, "feuilles relues == scellées");
        assert_eq!(got.n17_root, expected_root, "n17_root relu == scellé");

        // la re-preuve COMPLÈTE tient depuis le store Postgres relu.
        let key = efe_swarm_sdk::HmacKeySource::test_key()
            .derive(efe_nano_sdk::TenantId::system())
            .unwrap();
        let v = efe_swarm_sdk::verify_run_offline(
            &got.records,
            &got.llm_records,
            &got.n17_leaf_bytes(),
            got.n17_root_bytes(),
            &key,
        );
        assert!(v.fully_proven, "re-preuve complète depuis Postgres (Mode C non vide)");
        // isolation : un autre tenant ne voit rien.
        assert!(store.get(id, "autre-tenant").await.unwrap().is_none());
    }
}
