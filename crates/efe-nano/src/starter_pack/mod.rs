//! EFE-shipped starter pack of nano agents.
//!
//! All agents at TIER-1 from Day 1 (audited by EFE internally — see
//! M50 architecture § 15 "Exception au cycle 90-day TIER-1 promotion").
//!
//! Phase 1 ships :
//! - Sprint 2 (5 agents) : `json_validate`, `xml_validate`, `regex_extract`,
//!   `date_normalize`, `currency_normalize`
//! - Sprint 3 (5 agents) : `pdf_extract_text`, `html_extract_main_content`,
//!   `markdown_extract_headings`, `csv_normalize`, `sql_parse_safe`
//!
//! Phase 2 Sprint 4 adds (Mode B — Local Model) :
//! - `ner_clinical`, `ner_legal`, `intent_classify_generic`,
//!   `lang_detect`, `embed_text`
//!
//! Each agent module exports `manifest()` (and optionally `handler()` for
//! Mode A agents — Mode B/C agents have no handler closure : the runtime
//! invokes them via the model loader + VM pool).

// ── Mode A (Phase 1) ─────────────────────────────────────────────────
pub mod csv_normalize;
pub mod currency_normalize;
pub mod date_normalize;
pub mod html_extract_main_content;
pub mod json_validate;
pub mod markdown_extract_headings;
pub mod pdf_extract_text;
pub mod regex_extract;
pub mod sql_parse_safe;
pub mod xml_validate;

// ── Mode B (Phase 2 Sprint 4) ────────────────────────────────────────
pub mod embed_text;
pub mod intent_classify_generic;
pub mod lang_detect;
pub mod ner_clinical;
pub mod ner_legal;

// ── Mode C (Phase 2 Sprint 5) ────────────────────────────────────────
pub mod classify_topic_llm;
pub mod extract_structured;
pub mod qa_grounded;
pub mod sentiment_nuanced;
pub mod summarize_strict;

use crate::runtime::pure_rust::HandlerRegistry;

/// Convenience : register all Phase 1 (Sprints 1-3) Mode A starter
/// agents in the catalog + runtime handler registry.
///
/// Returns the number of agents registered (10 on success).
///
/// **Note** : Mode B agents (Phase 2 Sprint 4) are registered via
/// [`register_all_starter_pack_with_mode_b`] which additionally requires
/// a `ModelLoader` reference for their backing models.
pub async fn register_all_starter_pack(
    catalog: &dyn efe_nano_sdk::Catalog,
    registry: &HandlerRegistry,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    let tenant = efe_nano_sdk::TenantId::system();

    // Each tuple : (manifest, handler-entry, handler-closure-factory).
    // We register sequentially so any error bubbles up with context.
    macro_rules! register_mode_a {
        ($mod:ident) => {{
            let manifest = $mod::manifest();
            let entry = match &manifest.execution_mode {
                efe_nano_sdk::ExecutionMode::PureRust { entry } => entry.clone(),
                other => {
                    return Err(format!(
                        "{} : mode A registration expects PureRust, got {:?}",
                        stringify!($mod),
                        other
                    )
                    .into())
                }
            };
            catalog
                .publish(tenant, manifest)
                .await
                .map_err(|e| format!("publish {} : {e}", stringify!($mod)))?;
            registry.register(entry, $mod::handler()).await;
        }};
    }

    // Sprint 2 — 5 agents.
    register_mode_a!(json_validate);
    register_mode_a!(xml_validate);
    register_mode_a!(regex_extract);
    register_mode_a!(date_normalize);
    register_mode_a!(currency_normalize);

    // Sprint 3 — 5 more agents.
    register_mode_a!(markdown_extract_headings);
    register_mode_a!(csv_normalize);
    register_mode_a!(sql_parse_safe);
    register_mode_a!(html_extract_main_content);
    register_mode_a!(pdf_extract_text);

    Ok(10)
}

/// Phase 2 Sprint 4 convenience : register Phase 1 Mode A agents PLUS
/// the 5 Mode B starter agents in the catalog.
///
/// Mode B agents have no handler closure to register in the
/// `HandlerRegistry` — they are invoked via the
/// [`crate::runtime::LocalModelRuntime`] → `VmPool` → `ModelLoader`
/// chain.  The caller is responsible for wiring a `ModelLoader` that
/// knows about the 5 builtin models (`ner_clinical_v1`, `ner_legal_v1`,
/// `intent_classify_v1`, `lang_detect_v1`, `embedder_v1`) — using
/// [`crate::runtime::InProcessModelLoader::with_phase_2_builtins`] is
/// the canonical way.
///
/// Returns the number of agents registered (15 on success).
pub async fn register_all_starter_pack_with_mode_b(
    catalog: &dyn efe_nano_sdk::Catalog,
    registry: &HandlerRegistry,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    // Phase 1 first.
    register_all_starter_pack(catalog, registry).await?;

    let tenant = efe_nano_sdk::TenantId::system();
    macro_rules! register_mode_b {
        ($mod:ident) => {{
            let manifest = $mod::manifest();
            match &manifest.execution_mode {
                efe_nano_sdk::ExecutionMode::LocalModel { .. } => {}
                other => {
                    return Err(format!(
                        "{} : mode B registration expects LocalModel, got {:?}",
                        stringify!($mod),
                        other
                    )
                    .into())
                }
            }
            catalog
                .publish(tenant, manifest)
                .await
                .map_err(|e| format!("publish {} : {e}", stringify!($mod)))?;
        }};
    }

    register_mode_b!(ner_clinical);
    register_mode_b!(ner_legal);
    register_mode_b!(intent_classify_generic);
    register_mode_b!(lang_detect);
    register_mode_b!(embed_text);

    Ok(15)
}

/// Phase 2 Sprint 5 convenience : register Phase 1 Mode A + Sprint 4
/// Mode B + Sprint 5 Mode C agents (20 total).
///
/// Mode C agents have no handler closure to register in the
/// `HandlerRegistry` — the [`crate::runtime::LlmAugmentedRuntime`]
/// dispatches them via [`crate::runtime::LlmClient`].  The caller is
/// responsible for wiring a `MultiModeRuntime::mode_a_b_c` with a
/// Mode C runtime that knows about all 5 starter prompt templates
/// (`m50.summarize_strict.v1`, `m50.extract_structured.v1`,
/// `m50.classify_topic.v1`, `m50.sentiment_nuanced.v1`,
/// `m50.qa_grounded.v1`).
///
/// Returns the number of agents registered (20 on success).
pub async fn register_all_starter_pack_with_mode_b_and_c(
    catalog: &dyn efe_nano_sdk::Catalog,
    registry: &HandlerRegistry,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    // Phase 1 + Sprint 4 (Mode A + B).
    register_all_starter_pack_with_mode_b(catalog, registry).await?;

    let tenant = efe_nano_sdk::TenantId::system();
    macro_rules! register_mode_c {
        ($mod:ident) => {{
            let manifest = $mod::manifest();
            match &manifest.execution_mode {
                efe_nano_sdk::ExecutionMode::LlmAugmented { .. } => {}
                other => {
                    return Err(format!(
                        "{} : mode C registration expects LlmAugmented, got {:?}",
                        stringify!($mod),
                        other
                    )
                    .into())
                }
            }
            catalog
                .publish(tenant, manifest)
                .await
                .map_err(|e| format!("publish {} : {e}", stringify!($mod)))?;
        }};
    }

    register_mode_c!(summarize_strict);
    register_mode_c!(extract_structured);
    register_mode_c!(classify_topic_llm);
    register_mode_c!(sentiment_nuanced);
    register_mode_c!(qa_grounded);

    Ok(20)
}

/// Returns the JSON Schemas for all Sprint 5 Mode C agent outputs.
/// Used by [`crate::runtime::LlmAugmentedRuntime::warm_output_validator`]
/// to pre-compile schemas at dispatcher construction time.
pub fn mode_c_output_schemas() -> Vec<serde_json::Value> {
    vec![
        summarize_strict::manifest().output_schema,
        extract_structured::manifest().output_schema,
        classify_topic_llm::manifest().output_schema,
        sentiment_nuanced::manifest().output_schema,
        qa_grounded::manifest().output_schema,
    ]
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::InMemoryCatalog;
    use efe_nano_sdk::{
        Catalog, ListFilter, NanoAgentId, NanoAgentRef, StabilityTier, TenantId,
    };

    #[tokio::test]
    async fn register_all_starter_pack_works() {
        let catalog = InMemoryCatalog::new();
        let registry = HandlerRegistry::new();

        let count = register_all_starter_pack(&catalog, &registry).await.unwrap();
        assert_eq!(count, 10);

        // All 10 must be present and TIER-1.
        let tenant = TenantId::system();
        let agents = catalog
            .list(tenant, ListFilter::include_all())
            .await
            .unwrap();
        assert_eq!(agents.len(), 10, "expected 10 starter pack agents");
        for a in &agents {
            assert_eq!(
                a.stability_tier,
                StabilityTier::Tier1,
                "{} must be Tier1",
                a.id.as_str()
            );
        }

        // Spot check a few by id.
        for id in [
            "json_validate",
            "xml_validate",
            "regex_extract",
            "date_normalize",
            "currency_normalize",
            "markdown_extract_headings",
            "csv_normalize",
            "sql_parse_safe",
            "html_extract_main_content",
            "pdf_extract_text",
        ] {
            let r = NanoAgentRef::new(NanoAgentId::new(id).unwrap());
            let agent = catalog.get(&r, tenant).await.unwrap();
            assert_eq!(agent.id.as_str(), id);
            // All handler entries must be in the registry.
            if let efe_nano_sdk::ExecutionMode::PureRust { entry } = &agent.execution_mode {
                assert!(
                    registry.lookup(entry).await.is_some(),
                    "handler '{entry}' not registered"
                );
            }
        }

        assert_eq!(registry.len().await, 10);
    }

    #[tokio::test]
    async fn idempotency_publish_is_rejected_on_re_register() {
        let catalog = InMemoryCatalog::new();
        let registry = HandlerRegistry::new();
        register_all_starter_pack(&catalog, &registry).await.unwrap();
        let err = register_all_starter_pack(&catalog, &registry).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn all_agents_categorized_correctly() {
        let catalog = InMemoryCatalog::new();
        let registry = HandlerRegistry::new();
        register_all_starter_pack(&catalog, &registry).await.unwrap();
        let tenant = TenantId::system();

        let validation = catalog
            .list(tenant, ListFilter::by_category(efe_nano_sdk::Category::Validation))
            .await
            .unwrap();
        assert!(validation.len() >= 3);

        let extraction = catalog
            .list(tenant, ListFilter::by_category(efe_nano_sdk::Category::Extraction))
            .await
            .unwrap();
        assert!(extraction.len() >= 3);

        let transformation = catalog
            .list(tenant, ListFilter::by_category(efe_nano_sdk::Category::Transformation))
            .await
            .unwrap();
        assert!(transformation.len() >= 2);
    }

    #[tokio::test]
    async fn register_all_starter_pack_with_mode_b_yields_15() {
        let catalog = InMemoryCatalog::new();
        let registry = HandlerRegistry::new();
        let count = register_all_starter_pack_with_mode_b(&catalog, &registry)
            .await
            .unwrap();
        assert_eq!(count, 15);

        let tenant = TenantId::system();
        let agents = catalog
            .list(tenant, ListFilter::include_all())
            .await
            .unwrap();
        assert_eq!(agents.len(), 15);

        // The 5 Mode B agents.
        for id in [
            "ner_clinical",
            "ner_legal",
            "intent_classify_generic",
            "lang_detect",
            "embed_text",
        ] {
            let r = NanoAgentRef::new(NanoAgentId::new(id).unwrap());
            let agent = catalog.get(&r, tenant).await.unwrap();
            match &agent.execution_mode {
                efe_nano_sdk::ExecutionMode::LocalModel { .. } => {}
                other => panic!("{id} expected LocalModel, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn mode_b_agents_are_tier_1() {
        let catalog = InMemoryCatalog::new();
        let registry = HandlerRegistry::new();
        register_all_starter_pack_with_mode_b(&catalog, &registry)
            .await
            .unwrap();
        let tenant = TenantId::system();
        for id in [
            "ner_clinical",
            "ner_legal",
            "intent_classify_generic",
            "lang_detect",
            "embed_text",
        ] {
            let r = NanoAgentRef::new(NanoAgentId::new(id).unwrap());
            let a = catalog.get(&r, tenant).await.unwrap();
            assert_eq!(a.stability_tier, StabilityTier::Tier1);
        }
    }

    #[tokio::test]
    async fn mode_b_handler_registry_unchanged_size() {
        // Mode B agents do NOT register a Mode A handler — they go through
        // the model loader.  So registering Mode B should NOT increase the
        // handler registry beyond the 10 Mode A ones.
        let catalog = InMemoryCatalog::new();
        let registry = HandlerRegistry::new();
        register_all_starter_pack_with_mode_b(&catalog, &registry)
            .await
            .unwrap();
        assert_eq!(registry.len().await, 10);
    }

    #[tokio::test]
    async fn register_all_starter_pack_with_mode_b_and_c_yields_20() {
        let catalog = InMemoryCatalog::new();
        let registry = HandlerRegistry::new();
        let count = register_all_starter_pack_with_mode_b_and_c(&catalog, &registry)
            .await
            .unwrap();
        assert_eq!(count, 20);
        let tenant = TenantId::system();
        let agents = catalog
            .list(tenant, ListFilter::include_all())
            .await
            .unwrap();
        assert_eq!(agents.len(), 20);

        // The 5 Mode C agents.
        for id in [
            "summarize_strict",
            "extract_structured",
            "classify_topic_llm",
            "sentiment_nuanced",
            "qa_grounded",
        ] {
            let r = NanoAgentRef::new(NanoAgentId::new(id).unwrap());
            let agent = catalog.get(&r, tenant).await.unwrap();
            match &agent.execution_mode {
                efe_nano_sdk::ExecutionMode::LlmAugmented { .. } => {}
                other => panic!("{id} expected LlmAugmented, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn mode_c_agents_default_to_pre_llm_redact() {
        let catalog = InMemoryCatalog::new();
        let registry = HandlerRegistry::new();
        register_all_starter_pack_with_mode_b_and_c(&catalog, &registry)
            .await
            .unwrap();
        let tenant = TenantId::system();
        for id in [
            "summarize_strict",
            "extract_structured",
            "classify_topic_llm",
            "sentiment_nuanced",
            "qa_grounded",
        ] {
            let r = NanoAgentRef::new(NanoAgentId::new(id).unwrap());
            let a = catalog.get(&r, tenant).await.unwrap();
            assert_eq!(
                a.compliance.pii_redaction_strategy,
                efe_nano_sdk::PiiStrategy::PreLlmRedact,
                "{id} should default to PreLlmRedact"
            );
        }
    }

    #[tokio::test]
    async fn mode_c_handler_registry_still_10_after_register() {
        // Mode C also doesn't register Mode A handlers.
        let catalog = InMemoryCatalog::new();
        let registry = HandlerRegistry::new();
        register_all_starter_pack_with_mode_b_and_c(&catalog, &registry)
            .await
            .unwrap();
        assert_eq!(registry.len().await, 10);
    }

    #[tokio::test]
    async fn mode_c_output_schemas_returns_5_schemas() {
        let schemas = mode_c_output_schemas();
        assert_eq!(schemas.len(), 5);
        for s in &schemas {
            assert!(s.is_object());
            assert!(s.get("type").is_some());
        }
    }
}
