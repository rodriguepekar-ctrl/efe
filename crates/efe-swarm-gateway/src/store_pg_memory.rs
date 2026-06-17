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
    MemoryStore, MemoryUpdate, MemoryWitness, RunSummary,
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

/// **Témoin racine persisté** (MEM-3.b, I-MEM5) : root + feuilles scellées au dernier repli,
/// conservés SÉPARÉMENT du journal (`UPSERT` par `(tenant, family)`). C'est l'ancre qui rend
/// `/verify` tamper-évident : re-dériver le journal courant et le comparer à CE témoin détecte une
/// édition hors-bande d'une ligne de `swarm_memory_folds`. Le jumeau, côté mémoire, de `n17_root`.
const ROOT_DDL: &str = "CREATE TABLE IF NOT EXISTS swarm_memory_roots (\
    tenant TEXT NOT NULL,\
    family TEXT NOT NULL,\
    witness JSONB NOT NULL,\
    PRIMARY KEY (tenant, family))";

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
        sqlx::query(ROOT_DDL)
            .execute(&self.pool)
            .await
            .map_err(|e| GatewayError::Internal(format!("migration mémoire (témoins) : {e}")))?;
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
        // **ATOMICITÉ (revue adversariale)** : INSERT du repli + relecture + UPSERT du témoin dans
        // UNE transaction, sous un **verrou d'avis** par (tenant, family). Sans ça, deux folds
        // concurrents de la même famille réordonnent l'INSERT et l'UPSERT (last-writer-wins) → le
        // témoin « retarde » le journal → `/verify` lève une FAUSSE alarme CountMismatch sur un
        // journal honnête. Le verrou (relâché au commit) sérialise SEULEMENT la même famille ;
        // familles distinctes = clés de hash distinctes → aucune contention croisée.
        let mut tx = self.pool.begin().await.map_err(backend)?;
        let lock_key = format!("{tenant}|{}", update.family.as_str());
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
            .bind(&lock_key)
            .execute(&mut *tx)
            .await
            .map_err(backend)?;

        // I-MEM3 : un run NON-propre ne replie pas (aucun INSERT). Idempotence par UNIQUE
        // (tenant, family, run_id) + ON CONFLICT DO NOTHING → un run_id déjà plié est un no-op.
        let folded = if update.summary.clean {
            sqlx::query(
                "INSERT INTO swarm_memory_folds (tenant, family, run_id, summary) \
                 VALUES ($1, $2, $3, $4) ON CONFLICT (tenant, family, run_id) DO NOTHING",
            )
            .bind(tenant)
            .bind(update.family.as_str())
            .bind(update.run_id)
            .bind(Json(&update.summary))
            .execute(&mut *tx)
            .await
            .map_err(backend)?
            .rows_affected()
                > 0
        } else {
            false
        };

        // Relit le journal SUR LA TRANSACTION (voit l'INSERT non encore commité) → la source de
        // vérité dont cellules + root + témoin se re-dérivent, cohérente avec ce qui sera commité.
        let rows = sqlx::query(
            "SELECT run_id, summary FROM swarm_memory_folds \
             WHERE tenant = $1 AND family = $2 ORDER BY seq ASC",
        )
        .bind(tenant)
        .bind(update.family.as_str())
        .fetch_all(&mut *tx)
        .await
        .map_err(backend)?;
        let mut journal = Vec::with_capacity(rows.len());
        for row in rows {
            let run_id: Uuid = row.try_get("run_id").map_err(backend)?;
            let summary: Json<RunSummary> = row.try_get("summary").map_err(backend)?;
            journal.push(FoldRecord { run_id, family: update.family.clone(), summary: summary.0 });
        }

        let cells = materialize(&journal);
        let sealed = seal_memory(&journal, &key);
        // MEM-3.b : persiste le TÉMOIN (root + feuilles), SÉPARÉ du journal — l'ancre tamper-
        // évidente de `/verify`. `UPSERT` dans la MÊME transaction ⇒ toujours synchrone avec le
        // journal commité (atomicité). Journal vide ⇒ pas de racine ⇒ pas de témoin à écrire.
        if let Some(witness) = MemoryWitness::from_sealed(&sealed) {
            sqlx::query(
                "INSERT INTO swarm_memory_roots (tenant, family, witness) VALUES ($1, $2, $3) \
                 ON CONFLICT (tenant, family) DO UPDATE SET witness = EXCLUDED.witness",
            )
            .bind(tenant)
            .bind(update.family.as_str())
            .bind(Json(&witness))
            .execute(&mut *tx)
            .await
            .map_err(backend)?;
        }
        tx.commit().await.map_err(backend)?;
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

    async fn witness(&self, tenant: &str, family: &FamilyKey) -> Result<Option<MemoryWitness>, MemError> {
        let backend = |e: sqlx::Error| MemError::Backend(e.to_string());
        let row = sqlx::query("SELECT witness FROM swarm_memory_roots WHERE tenant = $1 AND family = $2")
            .bind(tenant)
            .bind(family.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(backend)?;
        match row {
            Some(r) => {
                let w: Json<MemoryWitness> = r.try_get("witness").map_err(backend)?;
                Ok(Some(w.0))
            }
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_swarm_memory::{family_key, verify_memory, FamilyMode};

    /// Intégration RÉELLE (Postgres) — la **parité** prod du `PgMemoryStore` avec l'InMemory,
    /// asserée contre un vrai backend (les chemins SQL distincts : `ON CONFLICT DO NOTHING`,
    /// `ORDER BY seq LIMIT`, round-trip JSONB, témoin `UPSERT`). Ignoré par défaut ; requiert
    /// `DATABASE_URL`. Lancer : `DATABASE_URL=postgres://… cargo test -p efe-swarm-gateway \
    /// --features postgres -- --ignored pg_memory`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "nécessite un Postgres (DATABASE_URL)"]
    async fn pg_memory_parity_idempotence_gate_isolation_snapshot_at_and_witness() {
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
        let store = PgMemoryStore::connect(&url, HmacKeySource::test_key()).await.unwrap();
        store.migrate().await.unwrap();

        // tenants ALÉATOIRES → isolation des re-runs sur une base partagée (le journal persiste).
        let ta = Uuid::new_v4().to_string();
        let tb = Uuid::new_v4().to_string();
        let task = serde_json::json!({ "text": "x" });
        let fam = family_key(&task, "consensus", &["g".into()], FamilyMode::Structural);
        let up = |run: Uuid, k: u32| MemoryUpdate { run_id: run, family: fam.clone(), summary: RunSummary::clean(k) };

        // ① I-MEM3 + idempotence (chemins SQL `ON CONFLICT DO NOTHING` + clean-gate).
        let run1 = Uuid::new_v4();
        let o1 = store.fold(&ta, up(run1, 6)).await.unwrap();
        assert!(o1.folded, "run propre → replié (folded=true)");
        assert_eq!(o1.version, 1);
        let o1b = store.fold(&ta, up(run1, 6)).await.unwrap();
        assert!(!o1b.folded, "même run_id → no-op (ON CONFLICT DO NOTHING)");
        assert_eq!((o1b.version, o1b.content_hash), (1, o1.content_hash), "version + état inchangés");
        let mut dirty = up(Uuid::new_v4(), 6);
        dirty.summary.clean = false;
        assert!(!store.fold(&ta, dirty).await.unwrap().folded, "run sale → aucun INSERT (quarantaine)");
        assert_eq!(store.snapshot(&ta, &fam).await.unwrap().version, 1, "toujours v1");

        // ② snapshot_at = préfixe `ORDER BY seq LIMIT` (I-MEM7) — parité avec le clamp InMemory.
        let h1 = store.snapshot(&ta, &fam).await.unwrap().content_hash;
        store.fold(&ta, up(Uuid::new_v4(), 4)).await.unwrap();
        store.fold(&ta, up(Uuid::new_v4(), 8)).await.unwrap();
        let cur = store.snapshot(&ta, &fam).await.unwrap();
        assert_eq!(cur.version, 3);
        let past = store.snapshot_at(&ta, &fam, 1).await.unwrap();
        assert_eq!((past.version, past.content_hash), (1, h1), "préfixe v1 reconstruit (BLAKE3 stable)");
        assert_ne!(past.content_hash, cur.content_hash, "≠ courant (préfixe passé distinct)");

        // ③ I-MEM6 : un autre tenant ne voit RIEN (WHERE tenant=$1).
        assert_eq!(store.snapshot(&tb, &fam).await.unwrap().version, 0, "tenant B isolé");

        // ④ I-MEM5 — le TÉMOIN persisté (JSONB) re-prouve, et une falsification HORS-BANDE casse.
        let key = store.derive(&ta).unwrap();
        let witness = store.witness(&ta, &fam).await.unwrap().expect("témoin persisté après repli");
        let journal = store.journal(&ta, &fam).await.unwrap();
        assert!(
            verify_memory(&journal, &witness.leaf_hashes, Some(witness.memory_root), &key).intact,
            "journal Pg relu → re-prouvé contre le témoin (round-trip JSONB OK)"
        );
        let mut tampered = journal.clone();
        tampered[0].summary.k_used = 999;
        let bad = verify_memory(&tampered, &witness.leaf_hashes, Some(witness.memory_root), &key);
        assert!(!bad.intact, "édition hors-bande détectée contre le témoin Pg (I-MEM5)");
        assert!(bad.first_break.is_some(), "et localisée");
    }
}
