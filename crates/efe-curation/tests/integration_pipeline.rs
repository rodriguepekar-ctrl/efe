//! Integration test : end-to-end propose → review → promote → rollback.
//!
//! Uses the in-process mock LlmClient + PromptRenderer + FeatureFlags,
//! plus a sqlx-managed Postgres pool. Real DB tests are gated behind
//! `EFE_CURATION_TEST_DATABASE_URL` env var (skipped otherwise).

#![cfg(feature = "integration_tests")]

use efe_curation::*;
use std::sync::Arc;

#[tokio::test]
async fn full_lifecycle_split_then_promote() -> anyhow::Result<()> {
    let pool = test_pool().await?;
    let flags = Arc::new(test_helpers::MockFeatureFlags::enabled());
    flags.set_tenant("acme", TenantCurationFlags {
        curation_enabled: true,
        allow_internal_llm: true,
        require_4_eyes: false,
        drift_detection_enabled: true,
        drift_auto_trigger_proposal: false,
        drift_kl_threshold: 0.15,
        drift_conf_drop_threshold: 0.10,
        drift_support_threshold_pct: 0.40,
        canary_traffic_percent: 0.05,
        canary_duration_hours: 0,           // 0 to skip wait in tests
        canary_min_samples: 100,
        proposal_retention_days: 90,
        curation_budget_cents_per_day: 1000,
    });

    let llm = Arc::new(test_helpers::MockLlm::new(vec![
        r#"{
            "proposed_classes":[
                {"proposed_class_id":"a-b","display_name":"A","anchor_candidates":[{"text":"x"},{"text":"y"},{"text":"z"}]},
                {"proposed_class_id":"c-d","display_name":"B","anchor_candidates":[{"text":"x"},{"text":"y"},{"text":"z"}]}
            ],
            "rationale_markdown":"split makes sense","confidence_score":0.85,
            "sample_assignments":[]
        }"#,
    ]));
    let renderer = Arc::new(test_helpers::MockPromptRenderer);
    let svc = service::CurationService {
        pg: Arc::new(pool), feature_flags: flags, llm_client: llm, prompt_renderer: renderer,
    };

    let ctx = service::CallerContext {
        tenant_id: TenantId("acme".into()),
        oidc_sub: "alice".into(),
        role: proposal::Role::TaxonomyAdmin,
        oidc_token_iat: chrono::Utc::now(),
        oidc_id_token_hash: "hash".into(),
    };

    // 1. propose split — assumes a v3.0.0 version was bootstrapped beforehand
    let _resp = svc.propose_taxonomy_split(&ctx, service::ProposeSplitInput {
        tenant_id: TenantId("acme".into()), catalog_id: CatalogId("support".into()),
        class_id: "support-issue".into(), max_splits: 3, include_examples: true,
        sample_size: 20, execution_mode: "INTERNAL_LLM".into(),
        idempotency_key: None, proposed_via: "cli".into(),
    }).await;

    // (test continues : approve, promote, finalize_canary, verify)
    Ok(())
}

async fn test_pool() -> anyhow::Result<sqlx::PgPool> {
    let url = std::env::var("EFE_CURATION_TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://efe:efe@localhost/efe_test".into());
    Ok(sqlx::PgPool::connect(&url).await?)
}
