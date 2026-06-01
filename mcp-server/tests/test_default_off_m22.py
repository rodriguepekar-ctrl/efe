"""M22 default-OFF compatibility tests.

Verify that without `EFE_FEATURE_M22_MODE3_ENABLED=true`, the
mcp-server M22 build is bit-for-bit M21 GA. Cornerstone guarantee
from M22 architecture § 17 + roadmap § F.7.
"""
from __future__ import annotations

import pytest

from mcp_server.profile import Profile, feature_enabled
from mcp_server.tools.registry import (
    all_known_tools,
    build_tool_registry,
    feature_gated_tools,
)


M22_TOOLS = (
    "efe_classify_mode3",
    "efe_get_mode3_reviews",
    "efe_approve_mode3_review",
    "efe_reject_mode3_review",
    "efe_get_concept_stack",
)


def test_default_off_no_m22_tools_listed(monkeypatch):
    """Without env-var, M22 tools NOT registered in agent-native."""
    monkeypatch.delenv("EFE_FEATURE_M22_MODE3_ENABLED", raising=False)
    monkeypatch.delenv("EFE_FEATURE_M21_MODE2_ENABLED", raising=False)
    monkeypatch.delenv("EFE_FEATURE_M20_HELPER_TOOLS_ENABLED", raising=False)
    registry = build_tool_registry(Profile.AGENT_NATIVE)
    for name in M22_TOOLS:
        assert name not in registry, (
            f"{name} must NOT be registered by default"
        )


def test_default_off_m22_feature_returns_false(monkeypatch):
    monkeypatch.delenv("EFE_FEATURE_M22_MODE3_ENABLED", raising=False)
    assert feature_enabled("M22_MODE3_ENABLED") is False


def test_m22_tools_appear_when_enabled(monkeypatch):
    monkeypatch.setenv("EFE_FEATURE_M22_MODE3_ENABLED", "true")
    registry = build_tool_registry(Profile.AGENT_NATIVE)
    for name in M22_TOOLS:
        assert name in registry, f"{name} must appear when flag ON"


def test_m22_all_known_tools_includes_m22(monkeypatch):
    """`all_known_tools()` ignores feature flags ; should always include."""
    monkeypatch.delenv("EFE_FEATURE_M22_MODE3_ENABLED", raising=False)
    all_tools = all_known_tools()
    for name in M22_TOOLS:
        assert name in all_tools


def test_m22_feature_gated_tools_count():
    """M22 adds 5 feature-gated tools."""
    gated = feature_gated_tools()
    m22_in_gated = [n for n in M22_TOOLS if n in gated]
    assert len(m22_in_gated) == 5


def test_m22_baseline_tool_count_preserved(monkeypatch):
    """Without M22 enabled, registry size = 31 (M19 baseline)."""
    monkeypatch.delenv("EFE_FEATURE_M22_MODE3_ENABLED", raising=False)
    monkeypatch.delenv("EFE_FEATURE_M21_MODE2_ENABLED", raising=False)
    monkeypatch.delenv("EFE_FEATURE_M20_HELPER_TOOLS_ENABLED", raising=False)
    registry = build_tool_registry(Profile.AGENT_NATIVE)
    assert len(registry) == 31


def test_m22_enabled_alone_tool_count(monkeypatch):
    """With M22 only enabled : 31 + 5 = 36 in agent-native."""
    monkeypatch.delenv("EFE_FEATURE_M21_MODE2_ENABLED", raising=False)
    monkeypatch.delenv("EFE_FEATURE_M20_HELPER_TOOLS_ENABLED", raising=False)
    monkeypatch.setenv("EFE_FEATURE_M22_MODE3_ENABLED", "true")
    registry = build_tool_registry(Profile.AGENT_NATIVE)
    assert len(registry) == 36


def test_m22_all_milestones_enabled_count(monkeypatch):
    """All M20 + M21 + M22 enabled : 31 + 5 + 4 + 5 = 45 agent-native."""
    monkeypatch.setenv("EFE_FEATURE_M20_HELPER_TOOLS_ENABLED", "true")
    monkeypatch.setenv("EFE_FEATURE_M21_MODE2_ENABLED", "true")
    monkeypatch.setenv("EFE_FEATURE_M22_MODE3_ENABLED", "true")
    registry = build_tool_registry(Profile.AGENT_NATIVE)
    # 31 baseline + 5 M20 + 4 M21 (set_llm_budget excluded) + 5 M22
    assert len(registry) == 45


def test_m22_full_deploy_profile(monkeypatch):
    """agent-native-deploy with all M20+M21+M22 ON : 31 + 3 + 5 + 5 + 5 = 49."""
    monkeypatch.setenv("EFE_FEATURE_M20_HELPER_TOOLS_ENABLED", "true")
    monkeypatch.setenv("EFE_FEATURE_M21_MODE2_ENABLED", "true")
    monkeypatch.setenv("EFE_FEATURE_M22_MODE3_ENABLED", "true")
    registry = build_tool_registry(Profile.AGENT_NATIVE_DEPLOY)
    # Includes 3 M18 deploy write tools + 1 M21 write tool
    assert len(registry) == 49
