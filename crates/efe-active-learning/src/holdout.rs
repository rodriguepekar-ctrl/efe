//! Deterministic holdout split — 10% reservation via SHA-256 hash mod 10.
//!
//! Per-tenant per-catalog deterministic so no state needed ; recalculable
//! any time. Same hash function is replicated in the `v_gold_examples_*`
//! Postgres views (cf. migration 0015).

use crate::{ActiveLearningResult, CatalogId, TenantId};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

/// Returns `true` iff a gold_example belongs to the holdout split for
/// the given tenant + catalog. Uses SHA-256 of the canonical key
/// `"<tenant>|<catalog>|<example_id>"` then takes the last byte mod 10.
///
/// Bin == 0 → holdout (10 % of mass).
pub fn is_in_holdout(tenant: &TenantId, catalog: &CatalogId, example_id: Uuid) -> bool {
    let key = format!("{}|{}|{}", tenant.0, catalog.0, example_id);
    let digest = Sha256::digest(key.as_bytes());
    (digest[31] % 10) == 0
}

/// Retrieve the training set for a tenant + catalog (excludes holdout).
pub async fn fetch_training_examples(
    pg: &PgPool, tenant: &TenantId, catalog: &CatalogId,
) -> ActiveLearningResult<Vec<TrainingExample>> {
    let rows: Vec<(Uuid, String, String, Option<chrono::DateTime<chrono::Utc>>)> =
        sqlx::query_as(
            r#"SELECT ge.id, ge.text, ge.expected_class_id, ge.pii_redacted_at
               FROM v_gold_examples_training ge
               JOIN gold_dataset gd ON gd.id = ge.gold_dataset_id
               WHERE gd.tenant_id = $1 AND gd.catalog_id = $2"#,
        )
        .bind(&tenant.0).bind(&catalog.0)
        .fetch_all(pg).await?;
    Ok(rows.into_iter()
        .filter(|(_, _, _, redacted)| redacted.is_none())  // exclude tombstoned
        .map(|(id, text, class, _)| TrainingExample { id, text, class_id: class })
        .collect())
}

/// Retrieve the holdout set for a tenant + catalog.
pub async fn fetch_holdout_examples(
    pg: &PgPool, tenant: &TenantId, catalog: &CatalogId,
) -> ActiveLearningResult<Vec<TrainingExample>> {
    let rows: Vec<(Uuid, String, String, Option<chrono::DateTime<chrono::Utc>>)> =
        sqlx::query_as(
            r#"SELECT ge.id, ge.text, ge.expected_class_id, ge.pii_redacted_at
               FROM v_gold_examples_holdout ge
               JOIN gold_dataset gd ON gd.id = ge.gold_dataset_id
               WHERE gd.tenant_id = $1 AND gd.catalog_id = $2"#,
        )
        .bind(&tenant.0).bind(&catalog.0)
        .fetch_all(pg).await?;
    Ok(rows.into_iter()
        .filter(|(_, _, _, redacted)| redacted.is_none())
        .map(|(id, text, class, _)| TrainingExample { id, text, class_id: class })
        .collect())
}

/// Persist the holdout snapshot used by a retune run, for reproducible
/// eval across versions.
pub async fn snapshot_holdout(
    pg: &PgPool, retune_run_id: Uuid, holdout: &[TrainingExample],
) -> ActiveLearningResult<()> {
    if holdout.is_empty() { return Ok(()); }
    let mut tx = pg.begin().await?;
    for ex in holdout {
        sqlx::query(
            r#"INSERT INTO bundle_holdout_split (id, retune_run_id, gold_example_id)
               VALUES ($1, $2, $3)
               ON CONFLICT (retune_run_id, gold_example_id) DO NOTHING"#,
        )
        .bind(Uuid::new_v4()).bind(retune_run_id).bind(ex.id)
        .execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct TrainingExample {
    pub id: Uuid,
    pub text: String,
    pub class_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_same_inputs_same_result() {
        let t = TenantId("acme".into());
        let c = CatalogId("c1".into());
        let id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let r1 = is_in_holdout(&t, &c, id);
        let r2 = is_in_holdout(&t, &c, id);
        assert_eq!(r1, r2);
    }

    #[test]
    fn approximately_10_percent() {
        let t = TenantId("test".into());
        let c = CatalogId("c1".into());
        let n = 10_000;
        let count = (0..n).filter(|_| {
            is_in_holdout(&t, &c, Uuid::new_v4())
        }).count();
        let pct = count as f64 / n as f64;
        assert!(
            (0.085..0.115).contains(&pct),
            "expected ~10% holdout, got {:.4}", pct
        );
    }

    #[test]
    fn per_tenant_isolation() {
        let id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let t1 = TenantId("acme".into());
        let t2 = TenantId("beta".into());
        let c = CatalogId("c1".into());
        // For this UUID, acme and beta might disagree on holdout —
        // test the "isolated" property : at least 200 different UUIDs
        // produce different splits between tenants in some cases.
        let mut differ = 0;
        for _ in 0..1000 {
            let id = Uuid::new_v4();
            if is_in_holdout(&t1, &c, id) != is_in_holdout(&t2, &c, id) {
                differ += 1;
            }
        }
        // ~18% disagreement expected (P(A in holdout) * P(B not) +
        // P(A not) * P(B in) = 0.1*0.9 + 0.9*0.1 = 0.18)
        assert!(
            (140..240).contains(&differ),
            "expected ~180 disagreements, got {}", differ
        );
    }

    #[test]
    fn per_catalog_isolation() {
        let t = TenantId("acme".into());
        let c1 = CatalogId("support".into());
        let c2 = CatalogId("billing".into());
        let mut differ = 0;
        for _ in 0..1000 {
            let id = Uuid::new_v4();
            if is_in_holdout(&t, &c1, id) != is_in_holdout(&t, &c2, id) {
                differ += 1;
            }
        }
        assert!((140..240).contains(&differ),
                "expected ~180 disagreements, got {}", differ);
    }
}
