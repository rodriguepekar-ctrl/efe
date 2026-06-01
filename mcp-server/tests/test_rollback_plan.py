"""efe_generate_rollback_plan + rollback.generate_plan pin tests (M18 D49)."""
from __future__ import annotations

import pytest

from mcp_server.deploy import generate_rollback_plan


# ─────────────────────────────────────────────────────────────────────
# Pure generate_plan unit tests
# ─────────────────────────────────────────────────────────────────────


def test_generate_plan_includes_pre_check_audit_chain():
    plan = generate_rollback_plan(
        from_version="v3.2.0", to_version="previous",
        reason="latency spike",
    )
    names = [c["name"] for c in plan.pre_checks]
    assert "verify_audit_chain_intact" in names


def test_generate_plan_includes_signature_verify_pre_check():
    plan = generate_rollback_plan(
        from_version="v3.2.0", to_version="v3.0.0",
        reason="bug",
    )
    names = [c["name"] for c in plan.pre_checks]
    assert "verify_target_bundle_signature" in names


def test_generate_plan_default_estimated_duration_300s():
    plan = generate_rollback_plan(
        from_version="v3.2.0", to_version="previous", reason="bug",
    )
    assert plan.estimated_duration_seconds == 300


def test_generate_plan_blast_radius_namespace_environment():
    plan = generate_rollback_plan(
        from_version="v3.2.0", to_version="previous",
        environment="staging", namespace="efe-staging",
        reason="testing rollback",
    )
    assert "efe-staging" in plan.blast_radius
    assert "staging" in plan.blast_radius


def test_generate_plan_commands_use_release_name_namespace():
    plan = generate_rollback_plan(
        from_version="v3.2.0", to_version="previous",
        namespace="efe-prod", release_name="efe",
        reason="bug",
    )
    cmds = "\n".join(plan.rollback_commands)
    assert "helm history efe -n efe-prod" in cmds
    assert "helm rollback efe <REV> -n efe-prod" in cmds


def test_generate_plan_includes_state_snapshot_pre_check():
    plan = generate_rollback_plan(
        from_version="v3.2.0", to_version="previous", reason="bug",
    )
    names = [c["name"] for c in plan.pre_checks]
    assert "snapshot_current_state" in names


def test_generate_plan_post_checks_reference_audit_chain():
    plan = generate_rollback_plan(
        from_version="v3.2.0", to_version="previous", reason="bug",
    )
    post = "\n".join(plan.post_checks)
    assert "audit_health_check" in post


# ─────────────────────────────────────────────────────────────────────
# Tool integration
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_generate_rollback_plan_tool_returns_runbook(
    agent_native_dispatcher, fake_backend,
):
    fake_backend.set_deployment_state({
        "bundle": {"slug": "x", "version": "v3.2.0", "manifest_sha256": "newsha"},
    })
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_generate_rollback_plan",
        args={"reason": "p99 latency spike", "to_version": "previous"},
    )
    assert res.is_error is False, res.content
    assert res.content["from_version"] == "v3.2.0"
    assert res.content["to_version"] == "previous"
    assert isinstance(res.content["rollback_commands"], list)
    assert len(res.content["rollback_commands"]) > 0
    assert any("NEVER" in w for w in res.content["warnings"])


@pytest.mark.asyncio
async def test_generate_rollback_plan_reason_required(agent_native_dispatcher):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_generate_rollback_plan", args={},
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_generate_rollback_plan_works_when_backend_no_current(
    agent_native_dispatcher, fake_backend,
):
    """If get_deployment_current is unavailable, from_version falls back
    to 'unknown' but the plan is still generated."""
    fake_backend.set_deployment_state(None)
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_generate_rollback_plan",
        args={"reason": "test"},
    )
    assert res.is_error is False
    assert res.content["from_version"] == "unknown"


@pytest.mark.asyncio
async def test_generate_rollback_plan_default_environment_production(
    agent_native_dispatcher, fake_backend,
):
    fake_backend.set_deployment_state({"bundle": {"slug": "x", "version": "v1"}})
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_generate_rollback_plan",
        args={"reason": "bug"},
    )
    assert res.content["environment"] == "production"


@pytest.mark.asyncio
async def test_generate_rollback_plan_reason_max_length_500(
    agent_native_dispatcher,
):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_generate_rollback_plan",
        args={"reason": "x" * 1000},
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"
