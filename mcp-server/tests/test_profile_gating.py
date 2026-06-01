"""Profile gating pin tests (M17 D43)."""
from __future__ import annotations

import pytest

from mcp_server.profile import (
    M16_CORE_TOOLS,
    M17_ADVANCED_TOOLS,
    M17_AGENT_NATIVE_TOOLS,
    Profile,
    load_profile_from_env,
    prompts_enabled,
    resources_enabled,
    sampling_enabled,
    streaming_enabled,
    tools_for,
)
from mcp_server.tools import build_tool_registry


def test_default_profile_is_core(monkeypatch):
    monkeypatch.delenv("EFE_MCP_TOOL_PROFILE", raising=False)
    assert load_profile_from_env() is Profile.CORE


def test_profile_advanced_includes_8_extra_tools():
    advanced = tools_for(Profile.ADVANCED)
    extra = advanced - M16_CORE_TOOLS
    assert extra == M17_ADVANCED_TOOLS
    assert len(extra) == 8


def test_profile_agent_native_includes_m17_extras_plus_m18_readonly():
    """M16 (6) + M17 advanced (8) + M17 agent-native (6) + M18 D47 (5)
    + M18 D48 (1) + M18 D49 (5) + M20 LLM-helpers (5).

    The AGENT_NATIVE profile expands across milestones. M20 helpers
    are listed in the gate (visible at profile-level) but their actual
    registration is gated additionally by env-var
    `EFE_FEATURE_M20_HELPER_TOOLS_ENABLED` (default OFF) — see the
    `test_default_off_m20.py` suite for that runtime gate.
    """
    from mcp_server.profile import (
        D8_ADMIN_READ_TOOLS,
        M18_BUNDLE_INTROSPECTION_TOOLS,
        M18_BUNDLE_SBOM_TOOLS,
        M18_DEPLOYMENT_HELPER_TOOLS,
        M20_LLM_HELPER_TOOLS,
        M21_MODE2_TOOLS,
        M22_MODE3_TOOLS,
        # Φ-D-OPS audit : M23 + M24 are now part of AGENT_NATIVE
        # (gated by their own feature flags but enumerated in the
        # profile membership). M25 stays operator-only — excluded.
        M23_CURATION_TOOLS,
        M24_ACTIVE_LEARNING_TOOLS,
    )

    agent = tools_for(Profile.AGENT_NATIVE)
    extra = agent - M16_CORE_TOOLS
    expected = (
        M17_ADVANCED_TOOLS
        | M17_AGENT_NATIVE_TOOLS
        | M18_BUNDLE_INTROSPECTION_TOOLS
        | M18_BUNDLE_SBOM_TOOLS
        | M18_DEPLOYMENT_HELPER_TOOLS
        | M20_LLM_HELPER_TOOLS
        | M21_MODE2_TOOLS  # M21 read-only Mode-2 tools (set_budget excluded)
        | M22_MODE3_TOOLS  # M22 Mode-3 multi-turn tools
        | D8_ADMIN_READ_TOOLS  # D8 admin read tools (writes excluded)
        | M23_CURATION_TOOLS   # gated by EFE_FEATURE_M23_CURATION_ENABLED
        | M24_ACTIVE_LEARNING_TOOLS  # gated by EFE_FEATURE_M24_ACTIVE_LEARNING_ENABLED
    )
    assert extra == expected, (
        f"AGENT_NATIVE drift : extra={extra - expected} missing={expected - extra}"
    )


def test_unknown_profile_raises(monkeypatch):
    monkeypatch.setenv("EFE_MCP_TOOL_PROFILE", "bogus")
    with pytest.raises(RuntimeError):
        load_profile_from_env()


def test_core_profile_six_tools_unchanged_from_m16():
    reg = build_tool_registry(Profile.CORE)
    assert set(reg.keys()) == M16_CORE_TOOLS
    assert len(reg) == 6


def test_profile_env_var_case_insensitive(monkeypatch):
    monkeypatch.setenv("EFE_MCP_TOOL_PROFILE", "ADVANCED")
    assert load_profile_from_env() is Profile.ADVANCED
    monkeypatch.setenv("EFE_MCP_TOOL_PROFILE", "Agent-Native")
    assert load_profile_from_env() is Profile.AGENT_NATIVE


def test_resources_enabled_only_advanced_and_above():
    assert not resources_enabled(Profile.CORE)
    assert resources_enabled(Profile.ADVANCED)
    assert resources_enabled(Profile.AGENT_NATIVE)


def test_prompts_enabled_only_advanced_and_above():
    assert not prompts_enabled(Profile.CORE)
    assert prompts_enabled(Profile.ADVANCED)
    assert prompts_enabled(Profile.AGENT_NATIVE)


def test_sampling_enabled_only_agent_native_and_above():
    assert not sampling_enabled(Profile.CORE)
    assert not sampling_enabled(Profile.ADVANCED)
    assert sampling_enabled(Profile.AGENT_NATIVE)


def test_streaming_enabled_only_agent_native_and_above():
    assert not streaming_enabled(Profile.CORE)
    assert not streaming_enabled(Profile.ADVANCED)
    assert streaming_enabled(Profile.AGENT_NATIVE)
