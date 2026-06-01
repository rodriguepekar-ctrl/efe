//! Sprint **D14** — End-to-end integration tests for the
//! Postgres-backed [`CatalogRegistry`] implementation.
//!
//! These tests target the **trait surface** through a real Postgres
//! database. Coverage matrix per the architecture doc § 10.3 :
//!
//!   * Happy paths for the 6 RPCs (register / evolve / get /
//!     list / list_versions / delete).
//!   * Error mapping (`CatalogError` variants → exact error code).
//!   * Concurrency (advisory lock + monotone version assignment).
//!   * Persistence (data survives a registry instance drop).
//!   * Quotas (max_names, max_versions, max_classes,
//!     max_sentences).
//!   * D11 isolation (cross-tenant SELECT/UPDATE/DELETE return
//!     0 rows / NotFound).
//!   * Migration application (idempotent on a re-run).
//!   * Page token round-trip via list_catalogs.
//!
//! # Test infrastructure
//!
//! Tests are **gated** on the `EFE_PG_TEST_URL` environment variable.
//! When unset, every `#[tokio::test]` in this file invokes
//! [`maybe_skip`] which returns `None` and the test exits cleanly.
//! This keeps `cargo test` hermetic on machines without Docker /
//! Postgres available.
//!
//! Local setup recipe :
//!
//! ```bash
//! # Spin up a temporary PG container.
//! docker run --rm -d \
//!     --name efe-test-pg \
//!     -e POSTGRES_USER=efe \
//!     -e POSTGRES_PASSWORD=efe \
//!     -e POSTGRES_DB=efe_catalog_test \
//!     -p 5433:5432 \
//!     postgres:15
//!
//! export EFE_PG_TEST_URL='postgres://efe:efe@127.0.0.1:5433/efe_catalog_test'
//! cargo test -p efe-server --test d14_postgres_catalog_integration
//!
//! docker rm -f efe-test-pg
//! ```
//!
//! Each test pre-truncates the `catalogs` table so its assertions
//! are independent of test order. The `_sqlx_migrations` table is
//! preserved across tests (idempotent migrate).

use std::collections::BTreeMap;
use std::sync::Arc;

use efe_server::catalog::{
    AuditEmitter, CatalogClass, CatalogContent, CatalogError, CatalogKey, CatalogMetadata,
    CatalogPolicy, CatalogRegistry, ConePolicy, ContentDigest, EmbedderSpec, EvolveOp,
    PostgresCatalogRegistry, ThresholdPolicy,
};
use once_cell::sync::OnceCell;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

// ====================================================================
// Test harness — single shared pool, env-gated.
// ====================================================================

static POOL: OnceCell<PgPool> = OnceCell::new();

fn maybe_skip() -> Option<String> {
    match std::env::var("EFE_PG_TEST_URL") {
        Ok(url) if !url.is_empty() => Some(url),
        _ => {
            eprintln!(
                "skipping d14_postgres_catalog_integration : \
                 set EFE_PG_TEST_URL to run"
            );
            None
        }
    }
}

async fn shared_pool() -> Option<PgPool> {
    let url = maybe_skip()?;
    if let Some(p) = POOL.get() {
        return Some(p.clone());
    }
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .expect("connect to EFE_PG_TEST_URL");
    let _ = POOL.set(pool.clone());
    Some(pool)
}

async fn fresh_registry() -> Option<PostgresCatalogRegistry> {
    let pool = shared_pool().await?;
    // Apply migrations once (idempotent).
    let migrator = sqlx::migrate!("./migrations");
    migrator.run(&pool).await.expect("migrations apply");
    // Wipe the catalogs table so each test starts with an empty
    // state. We don't drop the table because that would force a
    // re-migration on every test.
    sqlx::query("TRUNCATE TABLE catalogs")
        .execute(&pool)
        .await
        .expect("truncate catalogs");
    Some(PostgresCatalogRegistry::from_pool(
        pool,
        CatalogPolicy::default(),
        AuditEmitter::new(),
    ))
}

async fn fresh_registry_with_policy(policy: CatalogPolicy) -> Option<PostgresCatalogRegistry> {
    let pool = shared_pool().await?;
    let migrator = sqlx::migrate!("./migrations");
    migrator.run(&pool).await.expect("migrations apply");
    sqlx::query("TRUNCATE TABLE catalogs")
        .execute(&pool)
        .await
        .expect("truncate catalogs");
    Some(PostgresCatalogRegistry::from_pool(
        pool,
        policy,
        AuditEmitter::new(),
    ))
}

fn sample_content() -> CatalogContent {
    CatalogContent {
        embedder: EmbedderSpec {
            family: "sentence-transformers".into(),
            model: "all-MiniLM-L6-v2".into(),
            dim: 384,
            normalize: "l2".into(),
        },
        classes: vec![
            CatalogClass {
                name: "billing".into(),
                description: "B".into(),
                canonical_sentences: vec!["refund please".into()],
                color: "#000".into(),
                deprecated: false,
            },
            CatalogClass {
                name: "support".into(),
                description: "S".into(),
                canonical_sentences: vec!["it does not work".into()],
                color: "#111".into(),
                deprecated: false,
            },
        ],
        cone_policy: ConePolicy {
            max_projection_distance: 0.7,
        },
        thresholds: ThresholdPolicy {
            confidence_high: 0.85,
            confidence_medium: 0.6,
        },
    }
}

fn sample_metadata(principal: &str) -> CatalogMetadata {
    CatalogMetadata {
        created_at_unix_ms: 1_700_000_000_000,
        created_by: principal.to_owned(),
        description: "smoke catalog".into(),
        parent_key: String::new(),
        evolution_op: String::new(),
        labels: BTreeMap::new(),
        content_sha256: ContentDigest(String::new()),
    }
}

// ====================================================================
// Section A — Happy paths
// ====================================================================

#[tokio::test]
async fn register_creates_v1_persists_and_returns() {
    let Some(r) = fresh_registry().await else {
        return;
    };
    let res = r
        .register(
            "acme",
            "billing",
            sample_content(),
            sample_metadata("alice"),
        )
        .await
        .unwrap();
    assert_eq!(res.key.tenant, "acme");
    assert_eq!(res.key.name, "billing");
    assert_eq!(res.key.version, 1);
    assert_eq!(res.metadata.evolution_op, "register");
    assert_eq!(res.metadata.parent_key, "");
    assert_eq!(res.metadata.content_sha256.as_hex().len(), 64);
}

#[tokio::test]
async fn evolve_increments_version_to_v2() {
    let Some(r) = fresh_registry().await else {
        return;
    };
    let v1 = r
        .register(
            "acme",
            "billing",
            sample_content(),
            sample_metadata("alice"),
        )
        .await
        .unwrap();
    let v2 = r
        .evolve(
            &v1.key,
            EvolveOp::AppendClass {
                class: CatalogClass {
                    name: "shipping".into(),
                    description: "S".into(),
                    canonical_sentences: vec!["where is my order".into()],
                    color: "#aaa".into(),
                    deprecated: false,
                },
            },
            sample_metadata("alice"),
        )
        .await
        .unwrap();
    assert_eq!(v2.key.version, 2);
    assert_eq!(v2.metadata.parent_key, "acme/billing@v1");
    assert_eq!(v2.metadata.evolution_op, "append_class");
    assert_eq!(v2.content.classes.len(), 3);
}

#[tokio::test]
async fn get_returns_registered_content() {
    let Some(r) = fresh_registry().await else {
        return;
    };
    let v1 = r
        .register(
            "acme",
            "billing",
            sample_content(),
            sample_metadata("alice"),
        )
        .await
        .unwrap();
    let got = r.get(&v1.key, false).await.unwrap();
    assert_eq!(got.content.classes.len(), 2);
    assert_eq!(got.compiled, None);
}

#[tokio::test]
async fn list_catalogs_returns_only_caller_tenant() {
    let Some(r) = fresh_registry().await else {
        return;
    };
    r.register("acme", "alpha", sample_content(), sample_metadata("alice"))
        .await
        .unwrap();
    r.register("acme", "beta", sample_content(), sample_metadata("alice"))
        .await
        .unwrap();
    r.register("globex", "gamma", sample_content(), sample_metadata("bob"))
        .await
        .unwrap();
    let page = r.list_catalogs("acme", 100, None).await.unwrap();
    assert_eq!(page.catalogs.len(), 2);
    assert!(page.catalogs.iter().all(|c| c.tenant == "acme"));
    let names: Vec<_> = page.catalogs.iter().map(|c| c.name.clone()).collect();
    assert_eq!(names, vec!["alpha".to_string(), "beta".to_string()]);
}

#[tokio::test]
async fn list_versions_returns_keys_ascending() {
    let Some(r) = fresh_registry().await else {
        return;
    };
    let v1 = r
        .register(
            "acme",
            "history",
            sample_content(),
            sample_metadata("alice"),
        )
        .await
        .unwrap();
    r.evolve(
        &v1.key,
        EvolveOp::UpdateThresholds {
            thresholds: ThresholdPolicy {
                confidence_high: 0.9,
                confidence_medium: 0.7,
            },
        },
        sample_metadata("alice"),
    )
    .await
    .unwrap();
    let versions = r.list_versions("acme", "history").await.unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0].version, 1);
    assert_eq!(versions[1].version, 2);
}

#[tokio::test]
async fn delete_removes_version_with_force_false_unreferenced() {
    let Some(r) = fresh_registry().await else {
        return;
    };
    let v1 = r
        .register(
            "acme",
            "deleteme",
            sample_content(),
            sample_metadata("alice"),
        )
        .await
        .unwrap();
    r.delete(&v1.key, false, "alice").await.unwrap();
    let err = r.list_versions("acme", "deleteme").await.unwrap_err();
    assert!(matches!(err, CatalogError::NotFound(_)));
}

// ====================================================================
// Section B — Error mapping
// ====================================================================

#[tokio::test]
async fn get_unknown_returns_not_found() {
    let Some(r) = fresh_registry().await else {
        return;
    };
    let err = r
        .get(
            &CatalogKey {
                tenant: "acme".into(),
                name: "ghost".into(),
                version: 1,
            },
            false,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, CatalogError::NotFound(_)));
}

#[tokio::test]
async fn delete_referenced_without_force_is_conflict() {
    let Some(r) = fresh_registry().await else {
        return;
    };
    let v1 = r
        .register("acme", "ref", sample_content(), sample_metadata("alice"))
        .await
        .unwrap();
    r.evolve(
        &v1.key,
        EvolveOp::UpdateThresholds {
            thresholds: ThresholdPolicy {
                confidence_high: 0.9,
                confidence_medium: 0.7,
            },
        },
        sample_metadata("alice"),
    )
    .await
    .unwrap();
    let err = r.delete(&v1.key, false, "alice").await.unwrap_err();
    assert!(matches!(err, CatalogError::Conflict(_)));
}

#[tokio::test]
async fn delete_force_true_cascades() {
    let Some(r) = fresh_registry().await else {
        return;
    };
    let v1 = r
        .register(
            "acme",
            "cascade",
            sample_content(),
            sample_metadata("alice"),
        )
        .await
        .unwrap();
    r.evolve(
        &v1.key,
        EvolveOp::UpdateThresholds {
            thresholds: ThresholdPolicy {
                confidence_high: 0.9,
                confidence_medium: 0.7,
            },
        },
        sample_metadata("alice"),
    )
    .await
    .unwrap();
    r.delete(&v1.key, true, "alice").await.unwrap();
    let err = r.list_versions("acme", "cascade").await.unwrap_err();
    assert!(matches!(err, CatalogError::NotFound(_)));
}

#[tokio::test]
async fn evolve_unknown_class_returns_invalid_argument() {
    let Some(r) = fresh_registry().await else {
        return;
    };
    let v1 = r
        .register(
            "acme",
            "billing",
            sample_content(),
            sample_metadata("alice"),
        )
        .await
        .unwrap();
    let err = r
        .evolve(
            &v1.key,
            EvolveOp::RemoveClass {
                class_name: "no-such-class".into(),
            },
            sample_metadata("alice"),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, CatalogError::InvalidArgument(_)));
}

// ====================================================================
// Section C — Concurrency (advisory lock + monotone versions)
// ====================================================================

#[tokio::test]
async fn concurrent_register_same_key_distinct_versions() {
    let Some(r) = fresh_registry().await else {
        return;
    };
    let r = Arc::new(r);
    let mut handles = Vec::new();
    for _ in 0..8 {
        let r = r.clone();
        handles.push(tokio::spawn(async move {
            r.register("acme", "race", sample_content(), sample_metadata("svc"))
                .await
        }));
    }
    let mut versions = Vec::new();
    for h in handles {
        let res = h.await.unwrap();
        versions.push(res.unwrap().key.version);
    }
    versions.sort();
    assert_eq!(
        versions,
        vec![1u32, 2, 3, 4, 5, 6, 7, 8],
        "8 concurrent registers must produce versions 1..=8 strictly"
    );
}

#[tokio::test]
async fn concurrent_register_different_names_proceed_in_parallel() {
    let Some(r) = fresh_registry().await else {
        return;
    };
    let r = Arc::new(r);
    let r1 = r.clone();
    let r2 = r.clone();
    let (a, b) = tokio::join!(
        async move {
            r1.register("acme", "alpha", sample_content(), sample_metadata("svc"))
                .await
        },
        async move {
            r2.register("acme", "beta", sample_content(), sample_metadata("svc"))
                .await
        },
    );
    assert_eq!(a.unwrap().key.version, 1);
    assert_eq!(b.unwrap().key.version, 1);
}

// ====================================================================
// Section D — Persistence (registry instance drop survives data)
// ====================================================================

#[tokio::test]
async fn data_survives_registry_drop() {
    let Some(r1) = fresh_registry().await else {
        return;
    };
    let v1 = r1
        .register(
            "acme",
            "persist",
            sample_content(),
            sample_metadata("alice"),
        )
        .await
        .unwrap();
    drop(r1);

    // New registry instance, same pool.
    let pool = shared_pool().await.unwrap();
    let r2 =
        PostgresCatalogRegistry::from_pool(pool, CatalogPolicy::default(), AuditEmitter::new());
    let got = r2.get(&v1.key, false).await.unwrap();
    assert_eq!(got.key.version, 1);
    assert_eq!(got.content.classes.len(), 2);
}

// ====================================================================
// Section E — Quotas
// ====================================================================

#[tokio::test]
async fn max_names_per_tenant_quota_is_enforced() {
    let policy = CatalogPolicy {
        max_names_per_tenant: 2,
        max_versions_per_name: 10,
        max_classes_per_catalog: 64,
        max_sentences_per_class: 16,
    };
    let Some(r) = fresh_registry_with_policy(policy).await else {
        return;
    };
    r.register("acme", "alpha", sample_content(), sample_metadata("alice"))
        .await
        .unwrap();
    r.register("acme", "beta", sample_content(), sample_metadata("alice"))
        .await
        .unwrap();
    let err = r
        .register("acme", "gamma", sample_content(), sample_metadata("alice"))
        .await
        .unwrap_err();
    assert!(
        matches!(err, CatalogError::QuotaExceeded(_)),
        "expected QuotaExceeded, got {err:?}"
    );
}

#[tokio::test]
async fn max_versions_per_name_quota_is_enforced() {
    let policy = CatalogPolicy {
        max_names_per_tenant: 10,
        max_versions_per_name: 2,
        max_classes_per_catalog: 64,
        max_sentences_per_class: 16,
    };
    let Some(r) = fresh_registry_with_policy(policy).await else {
        return;
    };
    let v1 = r
        .register(
            "acme",
            "history",
            sample_content(),
            sample_metadata("alice"),
        )
        .await
        .unwrap();
    r.evolve(
        &v1.key,
        EvolveOp::UpdateThresholds {
            thresholds: ThresholdPolicy {
                confidence_high: 0.9,
                confidence_medium: 0.7,
            },
        },
        sample_metadata("alice"),
    )
    .await
    .unwrap();
    let err = r
        .evolve(
            &CatalogKey {
                tenant: "acme".into(),
                name: "history".into(),
                version: 2,
            },
            EvolveOp::UpdateThresholds {
                thresholds: ThresholdPolicy {
                    confidence_high: 0.92,
                    confidence_medium: 0.71,
                },
            },
            sample_metadata("alice"),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, CatalogError::QuotaExceeded(_)),
        "expected QuotaExceeded, got {err:?}"
    );
}

// ====================================================================
// Section F — D11 isolation
// ====================================================================
//
// The trait does NOT enforce tenant scoping (that's the gRPC handler's
// job — D11 + D12-bis assert_tenant_match). However, each SQL query
// in PostgresCatalogRegistry is parameterised on `tenant`, so even if
// the handler had a bug, cross-tenant access via a different `tenant`
// argument is impossible. These tests verify the SQL-level invariant
// directly.

#[tokio::test]
async fn cross_tenant_get_returns_not_found() {
    let Some(r) = fresh_registry().await else {
        return;
    };
    r.register(
        "acme",
        "billing",
        sample_content(),
        sample_metadata("alice"),
    )
    .await
    .unwrap();
    let err = r
        .get(
            &CatalogKey {
                tenant: "globex".into(), // different tenant
                name: "billing".into(),
                version: 1,
            },
            false,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, CatalogError::NotFound(_)));
}

#[tokio::test]
async fn cross_tenant_list_returns_empty() {
    let Some(r) = fresh_registry().await else {
        return;
    };
    r.register(
        "acme",
        "billing",
        sample_content(),
        sample_metadata("alice"),
    )
    .await
    .unwrap();
    let page = r.list_catalogs("globex", 100, None).await.unwrap();
    assert!(page.catalogs.is_empty());
}

#[tokio::test]
async fn cross_tenant_delete_returns_not_found() {
    let Some(r) = fresh_registry().await else {
        return;
    };
    r.register(
        "acme",
        "billing",
        sample_content(),
        sample_metadata("alice"),
    )
    .await
    .unwrap();
    let err = r
        .delete(
            &CatalogKey {
                tenant: "globex".into(),
                name: "billing".into(),
                version: 1,
            },
            false,
            "alice",
        )
        .await
        .unwrap_err();
    assert!(matches!(err, CatalogError::NotFound(_)));
}

// ====================================================================
// Section G — Migrations
// ====================================================================

#[tokio::test]
async fn migrations_apply_on_empty_db() {
    let Some(pool) = shared_pool().await else {
        return;
    };
    // Drop our table so the migrate has work to do.
    let _ = sqlx::query("DROP TABLE IF EXISTS catalogs CASCADE")
        .execute(&pool)
        .await;
    let _ = sqlx::query("DROP TABLE IF EXISTS _sqlx_migrations CASCADE")
        .execute(&pool)
        .await;
    let migrator = sqlx::migrate!("./migrations");
    migrator.run(&pool).await.expect("first migration");
    // Re-running must be a no-op (idempotent).
    migrator
        .run(&pool)
        .await
        .expect("second migration idempotent");
    // Sanity : the catalogs table now exists.
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.tables \
         WHERE table_name = 'catalogs')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(exists);
}

// ====================================================================
// Section H — SQL injection defense
// ====================================================================

#[tokio::test]
async fn sql_injection_via_tenant_field_is_rejected_by_validation() {
    // **D14 audit fix C1** — pre-fix this test inserted the injected
    // tenant literal and verified it was treated as a literal by sqlx
    // (it WAS — sqlx parametrised the bind). However, the registry's
    // post-fix `validate_identifier` rejects any tenant/name that
    // doesn't match `^[a-z0-9_-]{1,63}$`, so the injection is refused
    // up-front with `InvalidArgument` rather than silently inserting a
    // weird-looking row. The test now asserts the validation path,
    // which is a stronger contract (defense in depth + user-friendly
    // error).
    let Some(r) = fresh_registry().await else {
        return;
    };
    let evil = "acme' OR '1'='1";
    let err = r
        .register(evil, "x", sample_content(), sample_metadata("alice"))
        .await
        .unwrap_err();
    assert!(
        matches!(err, CatalogError::InvalidArgument(_)),
        "expected InvalidArgument from validate_identifier, got {err:?}",
    );
    // Belt-and-braces : confirm no row landed in the catalogs table
    // (e.g. via a parametrised path that bypassed validation).
    let page = r.list_catalogs("acme", 100, None).await.unwrap();
    assert!(
        page.catalogs.is_empty(),
        "rejected register must not have inserted anything ; got {:?}",
        page.catalogs
    );
}

#[tokio::test]
async fn validate_identifier_rejects_uppercase_tenant() {
    // **D14 audit fix C1** — also verify the lowercase invariant
    // explicitly. Pre-fix `register tenant="ACME" v1` would succeed
    // (parent_key='' bypasses the parent_key regex check) and then
    // `evolve to v2` would fail with an opaque check_violation
    // because the synthesised `parent_key="ACME/x@v1"` violates the
    // lowercase regex. With the fix, the register itself is refused.
    let Some(r) = fresh_registry().await else {
        return;
    };
    let err = r
        .register(
            "ACME",
            "billing",
            sample_content(),
            sample_metadata("alice"),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, CatalogError::InvalidArgument(_)),
        "expected InvalidArgument, got {err:?}",
    );
}

// ====================================================================
// Section I — Page tokens
// ====================================================================

#[tokio::test]
async fn list_catalogs_paginates_correctly() {
    let Some(r) = fresh_registry().await else {
        return;
    };
    for n in &["a", "b", "c", "d", "e"] {
        r.register("acme", n, sample_content(), sample_metadata("alice"))
            .await
            .unwrap();
    }
    let p1 = r.list_catalogs("acme", 2, None).await.unwrap();
    assert_eq!(p1.catalogs.len(), 2);
    assert!(!p1.next_page_token.is_empty());

    let p2 = r
        .list_catalogs("acme", 2, Some(&p1.next_page_token))
        .await
        .unwrap();
    assert_eq!(p2.catalogs.len(), 2);
    assert!(!p2.next_page_token.is_empty());

    let p3 = r
        .list_catalogs("acme", 2, Some(&p2.next_page_token))
        .await
        .unwrap();
    assert_eq!(p3.catalogs.len(), 1);
    assert!(p3.next_page_token.is_empty());
}
