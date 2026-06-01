"""efe_get_deployment_status + efe_get_deployment_history pin tests (M18 D49)."""
from __future__ import annotations

import pytest


# ─────────────────────────────────────────────────────────────────────
# get_deployment_status
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_status_backend_path(agent_native_dispatcher, fake_backend):
    fake_backend.set_deployment_state({
        "deployed_at": "2026-04-15T10:30:00Z",
        "deployed_by": "ops@example.com",
        "bundle": {"slug": "x", "version": "v1.0.0", "manifest_sha256": "abc"},
        "helm": {"chart_version": "1.2.0", "release_name": "efe", "namespace": "efe-prod"},
        "replicas": {"desired": 3, "ready": 3, "available": 3},
        "health": "healthy",
        "uptime_seconds": 432000,
    })
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_get_deployment_status", args={},
    )
    assert res.is_error is False, res.content
    assert res.content["bundle"]["version"] == "v1.0.0"
    assert res.content["health"] == "healthy"
    assert res.content["source"] == "backend"


@pytest.mark.asyncio
async def test_status_fallback_when_backend_old(
    agent_native_dispatcher, fake_backend,
):
    """No deployment state set → fallback to /info compose."""
    fake_backend.set_deployment_state(None)
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_get_deployment_status", args={},
    )
    assert res.is_error is False
    assert res.content["source"] == "fallback_info_compose"
    assert any(
        "fallback" in w.lower() or "partial" in w.lower()
        for w in res.content["warnings"]
    )


@pytest.mark.asyncio
async def test_status_no_args_required(agent_native_dispatcher, fake_backend):
    fake_backend.set_deployment_state({"bundle": {"slug": "x", "version": "v1.0.0"}})
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_get_deployment_status", args={},
    )
    assert res.is_error is False


@pytest.mark.asyncio
async def test_status_extra_args_rejected_by_schema(agent_native_dispatcher):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_get_deployment_status",
        args={"limit": 5},   # unknown arg
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


# ─────────────────────────────────────────────────────────────────────
# get_deployment_history
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_history_returns_deployments(
    agent_native_dispatcher, fake_backend,
):
    fake_backend.set_deployment_history([
        {"deployed_at": "2026-04-15T10:30:00Z", "bundle_version": "v1.0.0",
         "outcome": "success"},
        {"deployed_at": "2026-04-10T08:00:00Z", "bundle_version": "v0.9.0",
         "outcome": "success"},
    ])
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_get_deployment_history", args={"limit": 5},
    )
    assert res.is_error is False
    assert res.content["count"] == 2


@pytest.mark.asyncio
async def test_history_default_limit_10(agent_native_dispatcher, fake_backend):
    fake_backend.set_deployment_history([{} for _ in range(15)])
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_get_deployment_history", args={},
    )
    assert res.content["limit"] == 10
    assert res.content["count"] == 10


@pytest.mark.asyncio
async def test_history_max_100_enforced(agent_native_dispatcher, fake_backend):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_get_deployment_history", args={"limit": 1000},
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_history_min_1_enforced(agent_native_dispatcher, fake_backend):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_get_deployment_history", args={"limit": 0},
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_history_unsupported_backend_returns_not_supported(
    agent_native_dispatcher, fake_backend,
):
    fake_backend.set_deployment_history(None)
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_get_deployment_history", args={"limit": 10},
    )
    assert res.is_error is True
    assert res.content["code"] == "not_supported"
