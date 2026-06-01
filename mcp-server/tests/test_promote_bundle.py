"""efe_promote_bundle pin tests (M18 D50, write)."""
from __future__ import annotations

import pytest


@pytest.mark.asyncio
async def test_promote_without_confirm_rejected(agent_native_deploy_dispatcher):
    res = await agent_native_deploy_dispatcher.dispatch(
        tool_name="efe_promote_bundle",
        args={
            "slug": "x", "version": "v1.0.0", "confirm": False,
        },
    )
    assert res.is_error is True
    assert res.content["code"] == "confirm_required"


@pytest.mark.asyncio
async def test_promote_missing_confirm_field_fails_schema(
    agent_native_deploy_dispatcher,
):
    res = await agent_native_deploy_dispatcher.dispatch(
        tool_name="efe_promote_bundle",
        args={"slug": "x", "version": "v1.0.0"},
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_promote_invalid_scope_rejected(agent_native_deploy_dispatcher):
    res = await agent_native_deploy_dispatcher.dispatch(
        tool_name="efe_promote_bundle",
        args={
            "slug": "x", "version": "v1.0.0",
            "scope": "all-clusters", "confirm": True,
        },
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_promote_default_scope_tenant(
    agent_native_deploy_dispatcher, fake_backend,
):
    fake_backend.enable_deploy_writes(True)
    res = await agent_native_deploy_dispatcher.dispatch(
        tool_name="efe_promote_bundle",
        args={"slug": "x", "version": "v1.0.0", "confirm": True},
    )
    assert res.is_error is False, res.content
    assert res.content["scope"] == "tenant"


@pytest.mark.asyncio
async def test_promote_global_scope(
    agent_native_deploy_dispatcher, fake_backend,
):
    fake_backend.enable_deploy_writes(True)
    res = await agent_native_deploy_dispatcher.dispatch(
        tool_name="efe_promote_bundle",
        args={
            "slug": "x", "version": "v1.0.0",
            "scope": "global", "confirm": True,
        },
    )
    assert res.is_error is False
    assert res.content["scope"] == "global"


@pytest.mark.asyncio
async def test_promote_returns_previous_current(
    agent_native_deploy_dispatcher, fake_backend,
):
    fake_backend.enable_deploy_writes(True)
    # First promote v1.0.0
    await agent_native_deploy_dispatcher.dispatch(
        tool_name="efe_promote_bundle",
        args={"slug": "x", "version": "v1.0.0", "confirm": True},
    )
    # Then promote v2.0.0 → previous_current_version should be v1.0.0
    res = await agent_native_deploy_dispatcher.dispatch(
        tool_name="efe_promote_bundle",
        args={"slug": "x", "version": "v2.0.0", "confirm": True},
    )
    assert res.content["previous_current_version"] == "v1.0.0"
    assert res.content["new_current_version"] == "v2.0.0"


@pytest.mark.asyncio
async def test_promote_next_action_hint_emphasises_no_helm(
    agent_native_deploy_dispatcher, fake_backend,
):
    fake_backend.enable_deploy_writes(True)
    res = await agent_native_deploy_dispatcher.dispatch(
        tool_name="efe_promote_bundle",
        args={"slug": "x", "version": "v1.0.0", "confirm": True},
    )
    hint = res.content["next_action_hint"]
    # Must inform the LLM/operator that this is metadata-only and that
    # the actual deploy must be done manually with helm.
    assert "NEVER" in hint or "metadata" in hint.lower()
    assert "helm upgrade" in hint


@pytest.mark.asyncio
async def test_promote_unsupported_backend_returns_not_supported(
    agent_native_deploy_dispatcher, fake_backend,
):
    fake_backend.enable_deploy_writes(False)
    res = await agent_native_deploy_dispatcher.dispatch(
        tool_name="efe_promote_bundle",
        args={"slug": "x", "version": "v1.0.0", "confirm": True},
    )
    assert res.is_error is True
    assert res.content["code"] == "not_supported"


def test_promote_bundle_NOT_in_agent_native_profile(agent_native_registry):
    assert "efe_promote_bundle" not in agent_native_registry


def test_promote_bundle_IN_agent_native_deploy_profile(
    agent_native_deploy_registry,
):
    assert "efe_promote_bundle" in agent_native_deploy_registry
