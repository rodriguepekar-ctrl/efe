"""M21 default-OFF compatibility tests.

Verify that without `EFE_FEATURE_M21_MODE2_ENABLED=true`, an mcp-server
M21 build is bit-for-bit M20 GA. This is the cornerstone guarantee
from M21 architecture § 18 + roadmap § F.7.
"""
from __future__ import annotations

import pytest

from mcp_server.profile import Profile, feature_enabled
from mcp_server.tools.registry import (
    all_known_tools,
    build_tool_registry,
    feature_gated_tools,
)


M21_TOOLS = (
    "efe_classify_mode2",
    "efe_get_llm_budget",
    "efe_set_llm_budget",
    "efe_list_llm_providers",
    "efe_invalidate_llm_cache",
)


def test_default_off_no_m21_tools_listed(monkeypatch):
    """Without env-var, M21 tools NOT registered in agent-native."""
    monkeypatch.delenv("EFE_FEATURE_M21_MODE2_ENABLED", raising=False)
    monkeypatch.delenv("EFE_FEATURE_M20_HELPER_TOOLS_ENABLED", raising=False)
    registry = build_tool_registry(Profile.AGENT_NATIVE)
    for name in M21_TOOLS:
        assert name not in registry, (
            f"{name} must NOT be registered by default"
        )


def test_default_off_m21_feature_returns_false(monkeypatch):
    monkeypatch.delenv("EFE_FEATURE_M21_MODE2_ENABLED", raising=False)
    assert feature_enabled("M21_MODE2_ENABLED") is False


def test_m21_tools_appear_when_enabled(monkeypatch):
    monkeypatch.setenv("EFE_FEATURE_M21_MODE2_ENABLED", "true")
    registry = build_tool_registry(Profile.AGENT_NATIVE)
    # 4 of 5 in agent-native (set_llm_budget needs deploy)
    assert "efe_classify_mode2" in registry
    assert "efe_get_llm_budget" in registry
    assert "efe_list_llm_providers" in registry
    assert "efe_invalidate_llm_cache" in registry
    # set_llm_budget is agent-native-deploy only
    assert "efe_set_llm_budget" not in registry


def test_m21_set_budget_only_in_agent_native_deploy(monkeypatch):
    monkeypatch.setenv("EFE_FEATURE_M21_MODE2_ENABLED", "true")
    registry = build_tool_registry(Profile.AGENT_NATIVE_DEPLOY)
    assert "efe_set_llm_budget" in registry


def test_m21_all_known_tools_includes_m21(monkeypatch):
    """`all_known_tools()` ignores feature flags ; should always include M21."""
    monkeypatch.delenv("EFE_FEATURE_M21_MODE2_ENABLED", raising=False)
    all_tools = all_known_tools()
    for name in M21_TOOLS:
        assert name in all_tools


def test_m21_feature_gated_tools_count():
    """M21 adds 5 feature-gated tools to the registry."""
    gated = feature_gated_tools()
    m21_in_gated = [n for n in M21_TOOLS if n in gated]
    assert len(m21_in_gated) == 5


def test_m21_baseline_tool_count_preserved(monkeypatch):
    """Without M21 enabled, registry size = 31 (M19 baseline)."""
    monkeypatch.delenv("EFE_FEATURE_M21_MODE2_ENABLED", raising=False)
    monkeypatch.delenv("EFE_FEATURE_M20_HELPER_TOOLS_ENABLED", raising=False)
    registry = build_tool_registry(Profile.AGENT_NATIVE)
    assert len(registry) == 31


def test_m21_enabled_tool_count(monkeypatch):
    """With M21 (only) enabled : 31 + 4 = 35 in agent-native."""
    monkeypatch.delenv("EFE_FEATURE_M20_HELPER_TOOLS_ENABLED", raising=False)
    monkeypatch.setenv("EFE_FEATURE_M21_MODE2_ENABLED", "true")
    registry = build_tool_registry(Profile.AGENT_NATIVE)
    # 31 baseline + 4 M21 (set_llm_budget excluded — needs deploy profile)
    assert len(registry) == 35


def test_m21_plus_m20_enabled_tool_count(monkeypatch):
    """With both M20 + M21 enabled : 31 + 5 + 4 = 40."""
    monkeypatch.setenv("EFE_FEATURE_M20_HELPER_TOOLS_ENABLED", "true")
    monkeypatch.setenv("EFE_FEATURE_M21_MODE2_ENABLED", "true")
    registry = build_tool_registry(Profile.AGENT_NATIVE)
    assert len(registry) == 40


def test_m21_set_budget_requires_deploy_profile(monkeypatch):
    """efe_set_llm_budget is agent-native-deploy only (write tool)."""
    monkeypatch.setenv("EFE_FEATURE_M21_MODE2_ENABLED", "true")
    # Agent-native: NOT present
    reg_an = build_tool_registry(Profile.AGENT_NATIVE)
    assert "efe_set_llm_budget" not in reg_an
    # Agent-native-deploy: present
    reg_and = build_tool_registry(Profile.AGENT_NATIVE_DEPLOY)
    assert "efe_set_llm_budget" in reg_and
