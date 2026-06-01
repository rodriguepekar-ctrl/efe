"""D8 (GLOSS Sprint A) default-OFF compatibility tests.

Verify that without `EFE_FEATURE_D8_ADMIN_ENABLED=true`, the mcp-server
D8 build is bit-for-bit M22 GA. Cornerstone guarantee from
`docs/EFE_GO_LIVE_ONBOARDING_ARCHITECTURE.md` § 13.5 (« Compliance
preserved + 0 régression M16-M22 »).
"""
from __future__ import annotations

import pytest

from mcp_server.profile import Profile, feature_enabled
from mcp_server.tools.registry import (
    all_known_tools,
    build_tool_registry,
    feature_gated_tools,
)


D8_READ_TOOLS = (
    "efe_get_tenant_health",
    "efe_list_tenants",
    "efe_get_tenant",
    "efe_verify_compliance",
    "efe_list_api_keys",
    "efe_export_audit_log",
)
D8_WRITE_TOOLS = (
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
)
D8_TOOLS = D8_READ_TOOLS + D8_WRITE_TOOLS


def _disable_all_optional(monkeypatch):
    """Disable every optional feature flag for a clean baseline."""
    for flag in (
        "EFE_FEATURE_D8_ADMIN_ENABLED",
        "EFE_FEATURE_M22_MODE3_ENABLED",
        "EFE_FEATURE_M21_MODE2_ENABLED",
        "EFE_FEATURE_M20_HELPER_TOOLS_ENABLED",
    ):
        monkeypatch.delenv(flag, raising=False)


def test_default_off_no_d8_tools_listed(monkeypatch):
    """Without env-var, D8 tools NOT registered in agent-native."""
    _disable_all_optional(monkeypatch)
    registry = build_tool_registry(Profile.AGENT_NATIVE)
    for name in D8_TOOLS:
        assert name not in registry, (
            f"{name} must NOT be registered by default"
        )


def test_default_off_no_d8_tools_in_deploy(monkeypatch):
    """Same invariant in agent-native-deploy."""
    _disable_all_optional(monkeypatch)
    registry = build_tool_registry(Profile.AGENT_NATIVE_DEPLOY)
    for name in D8_TOOLS:
        assert name not in registry


def test_d8_feature_returns_false_by_default(monkeypatch):
    monkeypatch.delenv("EFE_FEATURE_D8_ADMIN_ENABLED", raising=False)
    assert feature_enabled("D8_ADMIN_ENABLED") is False


def test_d8_read_tool_appears_in_agent_native_when_enabled(monkeypatch):
    """Read tool exposed in `agent-native` profile."""
    _disable_all_optional(monkeypatch)
    monkeypatch.setenv("EFE_FEATURE_D8_ADMIN_ENABLED", "true")
    registry = build_tool_registry(Profile.AGENT_NATIVE)
    for name in D8_READ_TOOLS:
        assert name in registry, f"read tool {name} must appear when flag ON"


def test_d8_write_tools_NOT_in_agent_native_even_when_enabled(monkeypatch):
    """Write tools restricted to `agent-native-deploy` regardless of flag."""
    _disable_all_optional(monkeypatch)
    monkeypatch.setenv("EFE_FEATURE_D8_ADMIN_ENABLED", "true")
    registry = build_tool_registry(Profile.AGENT_NATIVE)
    for name in D8_WRITE_TOOLS:
        assert name not in registry, (
            f"write tool {name} must NOT be in agent-native (deploy-only)"
        )


def test_d8_write_tools_appear_in_deploy_when_enabled(monkeypatch):
    _disable_all_optional(monkeypatch)
    monkeypatch.setenv("EFE_FEATURE_D8_ADMIN_ENABLED", "true")
    registry = build_tool_registry(Profile.AGENT_NATIVE_DEPLOY)
    for name in D8_TOOLS:
        assert name in registry, f"{name} must appear in deploy when flag ON"


def test_d8_all_known_tools_includes_d8(monkeypatch):
    """`all_known_tools()` ignores feature flags ; should always include."""
    _disable_all_optional(monkeypatch)
    all_tools = all_known_tools()
    for name in D8_TOOLS:
        assert name in all_tools


def test_d8_feature_gated_tools_count():
    gated = feature_gated_tools()
    d8_in_gated = [n for n in D8_TOOLS if n in gated]
    assert len(d8_in_gated) == 16


def test_d8_read_write_split():
    assert len(D8_READ_TOOLS) == 6
    assert len(D8_WRITE_TOOLS) == 10
    assert len(D8_TOOLS) == 16


def test_d8_baseline_agent_native_count_preserved(monkeypatch):
    """With all optional flags OFF, agent-native registry == M19 baseline."""
    _disable_all_optional(monkeypatch)
    registry = build_tool_registry(Profile.AGENT_NATIVE)
    # Baseline = M16 (6) + M17 (8 advanced + 6 agent-native = 14) +
    # M18 read-only (5 + 1 + 5 = 11) = 31 tools.
    assert len(registry) == 31, (
        f"baseline broken — got {len(registry)} tools, expected 31"
    )


def test_d8_alone_enabled_tool_count_agent_native(monkeypatch):
    """With ONLY D8 enabled : 31 + 6 read tools = 37 in agent-native."""
    _disable_all_optional(monkeypatch)
    monkeypatch.setenv("EFE_FEATURE_D8_ADMIN_ENABLED", "true")
    registry = build_tool_registry(Profile.AGENT_NATIVE)
    assert len(registry) == 31 + 6, (
        f"agent-native + D8 broken — got {len(registry)}, expected {31 + 6}"
    )


def test_d8_alone_enabled_tool_count_deploy(monkeypatch):
    """With ONLY D8 enabled : 31 + M18 write (3) + 16 D8 = 50."""
    _disable_all_optional(monkeypatch)
    monkeypatch.setenv("EFE_FEATURE_D8_ADMIN_ENABLED", "true")
    registry = build_tool_registry(Profile.AGENT_NATIVE_DEPLOY)
    assert len(registry) == 31 + 3 + 16, (
        f"deploy + D8 broken — got {len(registry)}, expected {31 + 3 + 16}"
    )


def test_d8_write_tools_marked_is_write():
    """All D8 write tools have is_write=True."""
    all_tools = all_known_tools()
    for name in D8_WRITE_TOOLS:
        t = all_tools[name]
        assert t.is_write is True, f"{name} must have is_write=True"


def test_d8_read_tool_marked_is_write_false():
    all_tools = all_known_tools()
    t = all_tools["efe_get_tenant_health"]
    assert t.is_write is False


def test_d8_write_tools_require_confirm_in_schema():
    """All D8 write tools' input_schema requires confirm:true const."""
    all_tools = all_known_tools()
    for name in D8_WRITE_TOOLS:
        schema = all_tools[name].input_schema
        confirm = schema.get("properties", {}).get("confirm")
        assert confirm is not None, f"{name} missing confirm property"
        assert confirm.get("const") is True, (
            f"{name} confirm must have const:true"
        )
        required = schema.get("required", [])
        assert "confirm" in required, f"{name} confirm must be required"


def test_d8_tools_are_feature_flagged():
    """All 5 D8 tools have feature_flag set to D8_ADMIN_ENABLED."""
    all_tools = all_known_tools()
    for name in D8_TOOLS:
        t = all_tools[name]
        assert t.feature_flag == "D8_ADMIN_ENABLED", (
            f"{name} feature_flag mismatch: got {t.feature_flag!r}"
        )


def test_m16_to_m22_invariants_preserved(monkeypatch):
    """0 régression : enable all flags, check each milestone tool set
    still exposes the expected count."""
    monkeypatch.setenv("EFE_FEATURE_M20_HELPER_TOOLS_ENABLED", "true")
    monkeypatch.setenv("EFE_FEATURE_M21_MODE2_ENABLED", "true")
    monkeypatch.setenv("EFE_FEATURE_M22_MODE3_ENABLED", "true")
    monkeypatch.setenv("EFE_FEATURE_D8_ADMIN_ENABLED", "true")
    registry = build_tool_registry(Profile.AGENT_NATIVE_DEPLOY)
    # Expected sum :
    #   M16 (6) + M17 (14) + M18 read (11) + M18 write (3)
    # + M20 (5) + M21 read (4) + M21 write (1)
    # + M22 (5) + D8 (16, post-audit-pass extension) = 65.
    assert len(registry) == 65, (
        f"full-features registry got {len(registry)}, expected 65"
    )
