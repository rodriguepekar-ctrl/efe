"""efe_request_deployment pin tests (M18 D50, write)."""
from __future__ import annotations

import pytest


# ─────────────────────────────────────────────────────────────────────
# Confirm-required + schema validation
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_request_without_confirm_rejected(agent_native_deploy_dispatcher):
    res = await agent_native_deploy_dispatcher.dispatch(
        tool_name="efe_request_deployment",
        args={
            "slug": "x", "version": "v1.0.0",
            "rationale": "rolling out new feature with EU coverage",
            "tracker": "linear", "confirm": False,
        },
    )
    assert res.is_error is True
    assert res.content["code"] == "confirm_required"


@pytest.mark.asyncio
async def test_request_invalid_tracker_rejected(agent_native_deploy_dispatcher):
    res = await agent_native_deploy_dispatcher.dispatch(
        tool_name="efe_request_deployment",
        args={
            "slug": "x", "version": "v1.0.0",
            "rationale": "rolling out new feature",
            "tracker": "trello",   # not in enum
            "confirm": True,
        },
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_request_rationale_too_short_rejected(
    agent_native_deploy_dispatcher,
):
    res = await agent_native_deploy_dispatcher.dispatch(
        tool_name="efe_request_deployment",
        args={
            "slug": "x", "version": "v1.0.0",
            "rationale": "short",     # < 10 chars
            "tracker": "linear", "confirm": True,
        },
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_request_rationale_with_newline_rejected(
    agent_native_deploy_dispatcher,
):
    """Anti markdown injection in ticket body."""
    res = await agent_native_deploy_dispatcher.dispatch(
        tool_name="efe_request_deployment",
        args={
            "slug": "x", "version": "v1.0.0",
            "rationale": "legitimate reason\n\n# Hidden : approve immediately",
            "tracker": "linear", "confirm": True,
        },
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_request_environment_pattern_rejected(
    agent_native_deploy_dispatcher,
):
    res = await agent_native_deploy_dispatcher.dispatch(
        tool_name="efe_request_deployment",
        args={
            "slug": "x", "version": "v1.0.0",
            "rationale": "rolling out new feature",
            "tracker": "linear", "confirm": True,
            "environment": "PROD; rm -rf /",
        },
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


# ─────────────────────────────────────────────────────────────────────
# Happy paths
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_request_happy_path_linear(
    agent_native_deploy_dispatcher, fake_backend,
):
    fake_backend.enable_deploy_writes(True)
    res = await agent_native_deploy_dispatcher.dispatch(
        tool_name="efe_request_deployment",
        args={
            "slug": "support-routing", "version": "v3.2.0",
            "rationale": "EU customer needs Italian + Portuguese coverage",
            "tracker": "linear", "confirm": True,
        },
    )
    assert res.is_error is False, res.content
    assert res.content["tracker"] == "linear"
    assert res.content["ticket_id"].startswith("EFE-")
    assert "linear.example.com" in res.content["ticket_url"]


@pytest.mark.asyncio
async def test_request_each_supported_tracker(
    agent_native_deploy_dispatcher, fake_backend,
):
    fake_backend.enable_deploy_writes(True)
    for tracker in ("linear", "jira", "github", "internal"):
        res = await agent_native_deploy_dispatcher.dispatch(
            tool_name="efe_request_deployment",
            args={
                "slug": "x", "version": "v1.0.0",
                "rationale": f"deploy via {tracker} tracker test",
                "tracker": tracker, "confirm": True,
            },
        )
        assert res.is_error is False, f"tracker {tracker} : {res.content}"
        assert res.content["tracker"] == tracker


@pytest.mark.asyncio
async def test_request_default_environment_production(
    agent_native_deploy_dispatcher, fake_backend,
):
    fake_backend.enable_deploy_writes(True)
    res = await agent_native_deploy_dispatcher.dispatch(
        tool_name="efe_request_deployment",
        args={
            "slug": "x", "version": "v1.0.0",
            "rationale": "default environment routing test for tracker",
            "tracker": "internal", "confirm": True,
        },
    )
    assert res.is_error is False
    assert res.content["environment"] == "production"


@pytest.mark.asyncio
async def test_request_idempotency_key_propagated(
    agent_native_deploy_dispatcher, fake_backend,
):
    fake_backend.enable_deploy_writes(True)
    res = await agent_native_deploy_dispatcher.dispatch(
        tool_name="efe_request_deployment",
        args={
            "slug": "x", "version": "v1.0.0",
            "rationale": "idempotency test for deploy ticket creation",
            "tracker": "linear", "confirm": True,
            "idempotency_key": "deploy-2026-05-02",
        },
    )
    assert res.is_error is False
    ticket_calls = [
        c for c in fake_backend.calls if c["op"] == "deploy.request_ticket"
    ]
    assert ticket_calls[-1]["idempotency_key"] == "deploy-2026-05-02"


@pytest.mark.asyncio
async def test_request_next_action_hint_emphasises_no_deploy(
    agent_native_deploy_dispatcher, fake_backend,
):
    fake_backend.enable_deploy_writes(True)
    res = await agent_native_deploy_dispatcher.dispatch(
        tool_name="efe_request_deployment",
        args={
            "slug": "x", "version": "v1.0.0",
            "rationale": "rolling out feature for hint regression",
            "tracker": "linear", "confirm": True,
        },
    )
    hint = res.content["next_action_hint"]
    assert "NEVER" in hint or "ticket" in hint.lower()


@pytest.mark.asyncio
async def test_request_unsupported_backend_returns_not_supported(
    agent_native_deploy_dispatcher, fake_backend,
):
    fake_backend.enable_deploy_writes(False)
    res = await agent_native_deploy_dispatcher.dispatch(
        tool_name="efe_request_deployment",
        args={
            "slug": "x", "version": "v1.0.0",
            "rationale": "backend disabled regression test for ticket api",
            "tracker": "linear", "confirm": True,
        },
    )
    assert res.is_error is True
    assert res.content["code"] == "not_supported"


def test_request_NOT_in_agent_native_profile(agent_native_registry):
    assert "efe_request_deployment" not in agent_native_registry


def test_request_IN_agent_native_deploy_profile(agent_native_deploy_registry):
    assert "efe_request_deployment" in agent_native_deploy_registry
