// SPDX-License-Identifier: UNLICENSED
//! `PgMemoryStore` — la **mémoire/apprentissage inter-runs DURABLE** en Postgres (ADR-019,
//! MEM-3 ; feature `postgres`). Mêmes garanties que l'`InMemoryMemoryStore` : tenant-scopé
//! (I-MEM6), porte clean (I-MEM3), idempotent par `run_id`, `memory_root` re-prouvable (I-MEM5).
//! Le journal des replis est la **source de vérité** ; cellules + root s'en re-dérivent à la
//! lecture. Réutilise le pattern `PgPool` de `PgRunStore`.

use async_trait::async_trait;
use efe_nano_sdk::TenantId;
use efe_swarm_memory::{
    materialize, seal_memory, FamilyKey, FoldOutcome, FoldRecord, MemError, MemorySnapshot,
    MemoryStore, MemoryUpdate, RunSummary,
};
use efe_swarm_sdk::HmacKeySource;
use sqlx::types::Json;
use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use std::str::FromStr;
use uuid::Uuid;

use crate::error::GatewayError;

/// Journal des replis, **append-only**, indexé par `(tenant, family)`. `seq BIGSERIAL` fixe
/// l'ordre d'INSERTION (pour reconstruire un instantané passé, `snapshot_at`) ; l'ordre
/// CANONIQUE (par run_id) du root est re-trié à la lecture (ADR-007). `UNIQUE (tenant, family,
/// run_id)` ⇒ idempotence : un run ne replie qu'une fois. La porte clean (I-MEM3) est appliquée
/// AVANT l'INSERT (un run sale n'écrit jamais de ligne).
const MEM_DDL: &str = "CREATE TABLE IF NOT EXISTS swarm_memory_folds (\
    seq BIGSERIAL PRIMARY KEY,\
    tenant TEXT NOT NULL,\
    family TEXT NOT NULL,\
    run_id UUID NOT NULL,\
    summary JSONB NOT NULL,\
    UNIQUE (tenant, family, run_id))";
const MEM_DDL_INDEX: &str =
    "CREATE INDEX IF NOT EXISTS swarm_memory_folds_tf_seq_idx ON swarm_memory_folds (tenant, family, seq)";

/// Store Postgres de la mémoire inter-runs (prod).
pub struct PgMemoryStore {
    pool: PgPool,
    keys: HmacKeySource,
}

impl PgMemoryStore {
    /// Réutilise un `PgPool` existant + la source de clé tenant (HKDF, server-side).
    pub fn from_pool(pool: PgPool, keys: HmacKeySource) -> Self {
        Self { pool, keys }
    }

    /// Ouvre un pool (8 connexions) depuis une URL Postgres.
    pub async fn connect(url: &str, keys: HmacKeySource) -> Result<Self, GatewayError> {
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .connect(url)
            .await
            .map_err(|e| GatewayError::Internal(format!("connexion Postgres (mémoire) : {e}")))?;
        Ok(Self { pool, keys })
    }

    /// Crée la table + l'index si absents (idempotent).
    pub async fn migrate(&self) -> Result<(), GatewayError> {
        sqlx::query(MEM_DDL)
            .execute(&self.pool)
            .await
            .map_err(|e| GatewayError::Internal(format!("migration mémoire (table) : {e}")))?;
        sqlx::query(MEM_DDL_INDEX)
            .execute(&self.pool)
            .await
            .map_err(|e| GatewayError::Internal(format!("migration mémoire (index) : {e}")))?;
        Ok(())
    }

    /// Dérive la clé HKDF du tenant (miroir exact de l'`InMemoryMemoryStore`).
    fn derive(&self, tenant: &str) -> Result<Vec<u8>, MemError> {
        let tid = if tenant == "system" {
            TenantId::system()
        } else {
            TenantId::from_str(tenant).map_err(|e| MemError::KeyDerivation(e.to_string()))?
        };
        self.keys.derive(tid).map_err(|e| MemError::KeyDerivation(e.to_string()))
    }

    /// Charge le journal `(tenant, family)` en ordre d'INSERTION (`seq`), optionnellement
    /// borné aux `limit` premiers replis (pour `snapshot_at`). Décode chaque `summary` JSONB.
    async fn load_journal(
        &self,
        tenant: &str,
        family: &FamilyKey,
        limit: Option<i64>,
    ) -> Result<Vec<FoldRecord>, MemError> {
        let backend = |e: sqlx::Error| MemError::Backend(e.to_string());
        let sql = match limit {
            Some(_) => "SELECT run_id, summary FROM swarm_memory_folds \
                        WHERE tenant = $1 AND family = $2 ORDER BY seq ASC LIMIT $3",
            None => "SELECT run_id, summary FROM swarm_memory_folds \
                     WHERE tenant = $1 AND family = $2 ORDER BY seq ASC",
        };
        let mut q = sqlx::query(sql).bind(tenant).bind(family.as_str());
        if let Some(n) = limit {
            q = q.bind(n);
        }
        let rows = q.fetch_all(&self.pool).await.map_err(backend)?;
        let mut journal = Vec::with_capacity(rows.len());
        for row in rows {
            let run_id: Uuid = row.try_get("run_id").map_err(backend)?;
            let summary: Json<RunSummary> = row.try_get("summary").map_err(backend)?;
            journal.push(FoldRecord { run_id, family: family.clone(), summary: summary.0 });
        }
        Ok(journal)
    }
}

#[async_trait]
impl MemoryStore for PgMemoryStore {
    async fn snapshot(&self, tenant: &str, family: &FamilyKey) -> Result<MemorySnapshot, MemError> {
        let journal = self.load_journal(tenant, family, None).await?;
        Ok(MemorySnapshot::freeze(journal.len() as u64, materialize(&journal)))
    }

    async fn snapshot_at(&self, tenant: &str, family: &FamilyKey, version: u64) -> Result<MemorySnapshot, MemError> {
        // Préfixe des `version` premiers replis (ordre d'insertion) → instantané passé (I-MEM7).
        // Cast SATURANT : un `version` hors plage (≥ 2^63) dégrade en « toutes les lignes »,
        // exactement comme le clamp `.min(len)` de l'InMemory — parité stricte entre les stores.
        let lim = i64::try_from(version).unwrap_or(i64::MAX);
        let journal = self.load_journal(tenant, family, Some(lim)).await?;
        Ok(MemorySnapshot::freeze(journal.len() as u64, materialize(&journal)))
    }

    async fn fold(&self, tenant: &str, update: MemoryUpdate) -> Result<FoldOutcome, MemError> {
        let key = self.derive(tenant)?;
        let backend = |e: sqlx::Error| MemError::Backend(e.to_string());
        // I-MEM3 : un run NON-propre ne replie pas (aucun INSERT). Idempotence par UNIQUE
        // (tenant, family, run_id) + ON CONFLICT DO NOTHING → un run_id déjà plié est un no-op.
        let folded = if update.summary.clean {
            let res = sqlx::query(
                "INSERT INTO swarm_memory_folds (tenant, family, run_id, summary) \
                 VALUES ($1, $2, $3, $4) ON CONFLICT (tenant, family, run_id) DO NOTHING",
            )
            .bind(tenant)
            .bind(update.family.as_str())
            .bind(update.run_id)
            .bind(Json(&update.summary))
            .execute(&self.pool)
            .await
            .map_err(backend)?;
            res.rows_affected() > 0
        } else {
            false
        };
        // Re-dérive l'état courant (cellules + root) depuis le journal — la source de vérité.
        let journal = self.load_journal(tenant, &update.family, None).await?;
        let cells = materialize(&journal);
        let sealed = seal_memory(&journal, &key);
        Ok(FoldOutcome {
            version: journal.len() as u64,
            content_hash: cells.content_hash(),
            memory_root: sealed.memory_root,
            folded,
        })
    }

    async fn journal(&self, tenant: &str, family: &FamilyKey) -> Result<Vec<FoldRecord>, MemError> {
        self.load_journal(tenant, family, None).await
    }
}
