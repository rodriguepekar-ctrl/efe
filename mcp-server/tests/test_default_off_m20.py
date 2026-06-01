"""M20 default-OFF compatibility tests.

Verify that without `EFE_FEATURE_M20_HELPER_TOOLS_ENABLED=true`, an
mcp-server M20 build is bit-for-bit M19 GA. This is the cornerstone
guarantee from M20 architecture § 15 + roadmap § F.7.
"""
from __future__ import annotations

import pytest

from mcp_server.profile import Profile, feature_enabled
from mcp_server.tools.registry import (
    all_known_tools,
    build_tool_registry,
    feature_gated_tools,
)


# ─────────────────────────────────────────────────────────────────────


def test_default_off_no_helper_tools_listed(monkeypatch):
    """Without the env-var, M20 helper tools are NOT in agent-native registry."""
    monkeypatch.delenv("EFE_FEATURE_M20_HELPER_TOOLS_ENABLED", raising=False)
    registry = build_tool_registry(Profile.AGENT_NATIVE)
    for name in (
        "efe_summarize_classification",
        "efe_compare_bundles_quality",
        "efe_generate_test_sentences",
        "efe_analyze_confusion_matrix",
        "efe_draft_runbook",
    ):
        assert name not in registry, f"{name} must NOT be registered by default"


def test_default_off_feature_enabled_returns_false(monkeypatch):
    monkeypatch.delenv("EFE_FEATURE_M20_HELPER_TOOLS_ENABLED", raising=False)
    assert feature_enabled("M20_HELPER_TOOLS_ENABLED") is False


def test_default_off_helpers_appear_when_enabled(monkeypatch):
    monkeypatch.setenv("EFE_FEATURE_M20_HELPER_TOOLS_ENABLED", "true")
    registry = build_tool_registry(Profile.AGENT_NATIVE)
    for name in (
        "efe_summarize_classification",
        "efe_compare_bundles_quality",
        "efe_generate_test_sentences",
        "efe_analyze_confusion_matrix",
        "efe_draft_runbook",
    ):
        assert name in registry


def test_default_off_all_known_tools_includes_m20(monkeypatch):
    """`all_known_tools()` ignores feature flags ; should always include M20."""
    monkeypatch.delenv("EFE_FEATURE_M20_HELPER_TOOLS_ENABLED", raising=False)
    all_tools = all_known_tools()
    assert "efe_summarize_classification" in all_tools
    assert "efe_draft_runbook" in all_tools


def test_default_off_feature_gated_tools_count():
    """Feature-gated tools : 5 M20 + 5 M21 + 5 M22 + 16 D8 +
    7 M23 (taxonomy curation) + 4 M24 (active learning) +
    3 M25 (agent runtime). Audit-aligned post Φ-D-OPS.
    """
    gated = feature_gated_tools()
    names = set(gated)
    m20_expected = {
        "efe_summarize_classification",
        "efe_compare_bundles_quality",
        "efe_generate_test_sentences",
        "efe_analyze_confusion_matrix",
        "efe_draft_runbook",
    }
    m21_expected = {
        "efe_classify_mode2",
        "efe_get_llm_budget",
        "efe_set_llm_budget",
        "efe_list_llm_providers",
        "efe_invalidate_llm_cache",
    }
    m22_expected = {
        "efe_classify_mode3",
        "efe_get_mode3_reviews",
        "efe_approve_mode3_review",
        "efe_reject_mode3_review",
        "efe_get_concept_stack",
    }
    # GLOSS Sprint A (D8) — admin self-service tools (16 total :
    # 6 read + 10 write). Extended scope post audit pass GLOSS-A.
    d8_expected = {
        # read tools (6)
        "efe_get_tenant_health",
        "efe_list_tenants",
        "efe_get_tenant",
        "efe_verify_compliance",
        "efe_list_api_keys",
        "efe_export_audit_log",
        # write tools (10)
        "efe_create_tenant",
        "efe_update_tenant",
        "efe_suspend_tenant",
        "efe_delete_tenant",
        "efe_seed_catalog",
        "efe_attest_compliance",
        "efe_create_api_key",
        "efe_rotate_api_key",
        "efe_revoke_api_key",
        "efe_set_quotas",
    }
    # M23 — LLM-assisted taxonomy curation surface.
    m23_expected = {
        "efe_propose_taxonomy_split",
        "efe_suggest_class_merge",
        "efe_audit_bundle_vs_gold",
        "efe_explain_misclassification",
        "efe_generate_synthetic_examples",
        "efe_compare_bundles_quality",   # also in M20 (overlap)
        "efe_trigger_bundle_retune",
    }
    # M24 — Active learning loop write/read tools.
    m24_expected = {
        "efe_list_gold_label_queue",
        "efe_confirm_gold_label",
        "efe_request_gdpr_erasure",
        "efe_trigger_bundle_retune",     # also in M23 (overlap)
    }
    # M25 — Agent runtime EXPERIMENTAL.
    m25_expected = {
        "efe_create_agent_run",
        "efe_get_agent_run",
        "efe_approve_agent_step",
        "efe_terminate_agent_run",
    }
    assert m20_expected.issubset(names)
    assert m21_expected.issubset(names)
    assert m22_expected.issubset(names)
    assert d8_expected.issubset(names)
    assert m23_expected.issubset(names)
    assert m24_expected.issubset(names)
    assert m25_expected.issubset(names)
    # Total = union (overlaps collapse). Φ-D-OPS audit aligned the
    # canonical reference set with the post-M25 reality.
    expected = (
        m20_expected | m21_expected | m22_expected | d8_expected
        | m23_expected | m24_expected | m25_expected
    )
    assert names == expected, (
        f"feature-gated tool drift : "
        f"extra={names - expected} missing={expected - names}"
    )


def test_default_off_m19_baseline_tool_count_preserved(monkeypatch):
    """Without M20 enabled, agent-native registry size == M19 baseline."""
    monkeypatch.delenv("EFE_FEATURE_M20_HELPER_TOOLS_ENABLED", raising=False)
    registry = build_tool_registry(Profile.AGENT_NATIVE)
    # M19 baseline = 6 (M16 core) + 8 (M17 advanced) + 6 (M17 agent-native)
    # + 5 (M18 D47 introspection) + 1 (D48 SBOM) + 5 (D49 deploy helpers)
    # = 31 tools in agent-native (no M18 D50 write tools, no M20 helpers).
    assert len(registry) == 31


def test_default_off_m20_enabled_tool_count():
    """With M20 enabled in agent-native : 31 + 5 = 36."""
    import os
    os.environ["EFE_FEATURE_M20_HELPER_TOOLS_ENABLED"] = "true"
    try:
        registry = build_tool_registry(Profile.AGENT_NATIVE)
        assert len(registry) == 36
    finally:
        os.environ.pop("EFE_FEATURE_M20_HELPER_TOOLS_ENABLED", None)


def test_feature_enabled_recognises_truthy_values(monkeypatch):
    for v in ("true", "True", "TRUE", "1", "yes", "on"):
        monkeypatch.setenv("EFE_FEATURE_M20_HELPER_TOOLS_ENABLED", v)
        assert feature_enabled("M20_HELPER_TOOLS_ENABLED") is True


def test_feature_enabled_recognises_falsy_values(monkeypatch):
    for v in ("false", "False", "0", "no", "off", ""):
        monkeypatch.setenv("EFE_FEATURE_M20_HELPER_TOOLS_ENABLED", v)
        assert feature_enabled("M20_HELPER_TOOLS_ENABLED") is False


def test_feature_enabled_unknown_value_uses_default(monkeypatch):
    monkeypatch.setenv("EFE_FEATURE_M20_HELPER_TOOLS_ENABLED", "maybe")
    assert feature_enabled("M20_HELPER_TOOLS_ENABLED", default=False) is False
    assert feature_enabled("M20_HELPER_TOOLS_ENABLED", default=True) is True
