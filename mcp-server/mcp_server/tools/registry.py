"""Tool registry — profile-aware tool collection.

The registry dynamically filters the exposed tool set based on the
active profile (`core` / `advanced` / `agent-native` / `experimental`).
"""
from __future__ import annotations

from . import (
    audit_health_check,
    bulk_classify,
    calibrate_thresholds,
    check_bundle_compatibility,
    classify,
    compare_classes,
    consolidate_versions,
    curate_catalog,
    detect_drift,
    diagnose_catalog,
    diff_bundles,
    dry_run_deploy,
    evolve_catalog,
    explain_classification,
    export_catalog,
    find_similar_examples,
    generate_helm_values,
    generate_rollback_plan,
    get_admin_quotas,
    get_audit_log,
    get_deployment_history,
    get_deployment_status,
    get_sbom,
    inspect_bundle,
    list_available_bundles,
    list_catalogs,
    m20_analyze_confusion_matrix,
    m20_compare_bundles_quality,
    m20_draft_runbook,
    m20_generate_test_sentences,
    m20_summarize_classification,
    m21_classify_mode2,
    m21_get_llm_budget,
    m21_invalidate_llm_cache,
    m21_list_llm_providers,
    m21_set_llm_budget,
    m22_approve_mode3_review,
    m22_classify_mode3,
    m22_get_concept_stack,
    m22_get_mode3_reviews,
    m22_reject_mode3_review,
    m23_attest_compliance,
    m23_create_api_key,
    m23_create_tenant,
    m23_delete_tenant,
    m23_export_audit_log,
    m23_get_tenant,
    m23_get_tenant_health,
    m23_list_api_keys,
    m23_list_tenants,
    m23_revoke_api_key,
    m23_rotate_api_key,
    m23_seed_catalog,
    m23_set_quotas,
    m23_suspend_tenant,
    m23_update_tenant,
    m23_verify_compliance,
    promote_bundle,
    propose_examples,
    register_bundle,
    register_catalog,
    request_deployment,
    score_distribution,
    suggest_classes,
    verify_bundle_signature,
)
from ..profile import Profile, feature_enabled, tools_for
from .types import Tool

# Sub-package tool bundles. Each `__init__.py` exposes a `TOOLS`
# tuple of `Tool` instances ; we splat them into `_ALL_TOOLS` below.
from .curation import TOOLS as _CURATION_TOOLS              # M23
from .active_learning import TOOLS as _ACTIVE_LEARNING_TOOLS  # M24
from .agent_runtime import TOOLS as _AGENT_RUNTIME_TOOLS      # M25


# All tools, regardless of profile. Filtered at registry-build time.
_ALL_TOOLS: tuple[Tool, ...] = (
    # M16 core
    classify.TOOL,
    list_catalogs.TOOL,
    get_audit_log.TOOL,
    get_admin_quotas.TOOL,
    register_catalog.TOOL,
    evolve_catalog.TOOL,
    # M17 advanced (8)
    explain_classification.TOOL,
    compare_classes.TOOL,
    score_distribution.TOOL,
    find_similar_examples.TOOL,
    diagnose_catalog.TOOL,
    detect_drift.TOOL,
    audit_health_check.TOOL,
    export_catalog.TOOL,
    # M17 agent-native (6)
    suggest_classes.TOOL,
    propose_examples.TOOL,
    curate_catalog.TOOL,
    calibrate_thresholds.TOOL,
    consolidate_versions.TOOL,
    bulk_classify.TOOL,
    # M18 D47 — bundle introspection (5)
    list_available_bundles.TOOL,
    inspect_bundle.TOOL,
    verify_bundle_signature.TOOL,
    diff_bundles.TOOL,
    check_bundle_compatibility.TOOL,
    # M18 D48 — SBOM (1)
    get_sbom.TOOL,
    # M18 D49 — deployment helpers (5)
    get_deployment_status.TOOL,
    get_deployment_history.TOOL,
    dry_run_deploy.TOOL,
    generate_helm_values.TOOL,
    generate_rollback_plan.TOOL,
    # M18 D50 — deploy write tools (3, agent-native-deploy only)
    register_bundle.TOOL,
    promote_bundle.TOOL,
    request_deployment.TOOL,
    # M20 — LLM-helper tools (5, opt-in via EFE_FEATURE_M20_HELPER_TOOLS_ENABLED)
    m20_summarize_classification.TOOL,
    m20_compare_bundles_quality.TOOL,
    m20_generate_test_sentences.TOOL,
    m20_analyze_confusion_matrix.TOOL,
    m20_draft_runbook.TOOL,
    # M21 — Mode-2 LLM-routed tools (5, opt-in via EFE_FEATURE_M21_MODE2_ENABLED)
    m21_classify_mode2.TOOL,
    m21_get_llm_budget.TOOL,
    m21_set_llm_budget.TOOL,
    m21_list_llm_providers.TOOL,
    m21_invalidate_llm_cache.TOOL,
    # M22 — Mode-3 multi-turn iterative reasoning tools (5, opt-in
    # via EFE_FEATURE_M22_MODE3_ENABLED). Requires M21 active.
    m22_classify_mode3.TOOL,
    m22_get_mode3_reviews.TOOL,
    m22_approve_mode3_review.TOOL,
    m22_reject_mode3_review.TOOL,
    m22_get_concept_stack.TOOL,
    # D8 (GLOSS Sprint A) — admin self-service tools (16, opt-in via
    # EFE_FEATURE_D8_ADMIN_ENABLED). Read tools exposed in
    # `agent-native` ; write tools restricted to `agent-native-deploy`.
    # Cf. `docs/EFE_GO_LIVE_ONBOARDING_ARCHITECTURE.md` § 10
    # + audit pass GLOSS-A (extended scope to cover full CLI surface).
    # Read tools (6) :
    m23_get_tenant_health.TOOL,
    m23_list_tenants.TOOL,
    m23_get_tenant.TOOL,
    m23_verify_compliance.TOOL,
    m23_list_api_keys.TOOL,
    m23_export_audit_log.TOOL,
    # Write tools (10) :
    m23_create_tenant.TOOL,
    m23_update_tenant.TOOL,
    m23_suspend_tenant.TOOL,
    m23_delete_tenant.TOOL,
    m23_seed_catalog.TOOL,
    m23_attest_compliance.TOOL,
    m23_create_api_key.TOOL,
    m23_rotate_api_key.TOOL,
    m23_revoke_api_key.TOOL,
    m23_set_quotas.TOOL,
    # ─────────────────────────────────────────────────────────────────
    # M23 — taxonomy curation tools (sub-package)
    # Gated by `EFE_FEATURE_M23_CURATION_ENABLED`.
    # ─────────────────────────────────────────────────────────────────
    *_CURATION_TOOLS,
    # ─────────────────────────────────────────────────────────────────
    # M24 — active learning tools (sub-package)
    # Gated by `EFE_FEATURE_M24_ACTIVE_LEARNING_ENABLED`.
    # ─────────────────────────────────────────────────────────────────
    *_ACTIVE_LEARNING_TOOLS,
    # ─────────────────────────────────────────────────────────────────
    # M25 — agent runtime tools (sub-package, EXPERIMENTAL)
    # Gated by `EFE_FEATURE_AGENT_RUNTIME_ENABLED` + per-tenant DPIA.
    # ─────────────────────────────────────────────────────────────────
    *_AGENT_RUNTIME_TOOLS,
)


def build_tool_registry(profile: Profile = Profile.CORE) -> dict[str, Tool]:
    """Return {tool_name → Tool} filtered by `profile` and feature flags.

    M20 addition : tools with a `feature_flag` attribute set are
    additionally gated by `EFE_FEATURE_<flag>` env-var (default false).
    Even if the profile would expose them, the env-var must be true.
    This is the OFF-by-default mechanism (cf. roadmap § F.7).
    """
    allowed = tools_for(profile)
    registry: dict[str, Tool] = {}
    for t in _ALL_TOOLS:
        if t.name not in allowed:
            continue
        # M20 feature flag gate
        if t.feature_flag is not None and not feature_enabled(t.feature_flag):
            continue
        if t.name in registry:
            raise RuntimeError(f"duplicate tool name in registry : {t.name!r}")
        registry[t.name] = t
    return registry


def all_known_tools() -> dict[str, Tool]:
    """Every tool, ungated (ignores both profile AND feature flags).
    Used by tests + admin diagnostics."""
    return {t.name: t for t in _ALL_TOOLS}


def feature_gated_tools() -> dict[str, Tool]:
    """Subset of all tools that are gated by a feature flag.
    Used by tests and `efe_list_active_features` (M21)."""
    return {t.name: t for t in _ALL_TOOLS if t.feature_flag is not None}
