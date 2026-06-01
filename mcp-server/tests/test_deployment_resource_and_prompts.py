"""efe://deployments/current resource + D49 prompt pin tests (M18 D49)."""
from __future__ import annotations

import json

import pytest

from mcp_server.prompts import PROMPT_DESCRIPTORS, render_prompt
from mcp_server.resources.registry import (
    InvalidResourceUri,
    parse_resource_uri,
)


# ─────────────────────────────────────────────────────────────────────
# URI grammar
# ─────────────────────────────────────────────────────────────────────


def test_parse_deployment_current_uri():
    p = parse_resource_uri("efe://deployments/current")
    assert p.kind == "deployment_current"


def test_parse_deployment_unknown_subpath_rejected():
    with pytest.raises(InvalidResourceUri):
        parse_resource_uri("efe://deployments/random")


def test_parse_deployment_no_subpath_rejected():
    with pytest.raises(InvalidResourceUri):
        parse_resource_uri("efe://deployments")


def test_resources_list_includes_deployment_descriptor(agent_native_dispatcher):
    resources = agent_native_dispatcher.list_resources()
    uris = {r["uri"] for r in resources}
    assert "efe://deployments/current" in uris


# ─────────────────────────────────────────────────────────────────────
# Read end-to-end
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_read_deployment_current_backend_path(
    agent_native_dispatcher, fake_backend,
):
    fake_backend.set_deployment_state({
        "bundle": {"slug": "x", "version": "v1.0.0"},
        "health": "healthy",
    })
    res = await agent_native_dispatcher.read_resource(
        "efe://deployments/current"
    )
    assert "uri" in res
    body = json.loads(res["text"])
    assert body["bundle"]["version"] == "v1.0.0"
    assert body["source"] == "backend"


@pytest.mark.asyncio
async def test_read_deployment_current_fallback_when_no_state(
    agent_native_dispatcher, fake_backend,
):
    fake_backend.set_deployment_state(None)
    res = await agent_native_dispatcher.read_resource(
        "efe://deployments/current"
    )
    body = json.loads(res["text"])
    assert body["source"] == "fallback_info_compose"


@pytest.mark.asyncio
async def test_read_deployment_emits_audit_event(
    agent_native_dispatcher, fake_backend, audit_emitter,
):
    """M17-A5 carry-over : every resources/read emits audit."""
    audit_emitter.enabled = False
    fake_backend.set_deployment_state({"bundle": {"slug": "x", "version": "v1"}})
    before = len(audit_emitter.recent(limit=500))
    await agent_native_dispatcher.read_resource("efe://deployments/current")
    after = len(audit_emitter.recent(limit=500))
    assert after == before + 1
    last = audit_emitter.recent(limit=1)[0]
    assert last.tool == "mcp_resource_read"


# ─────────────────────────────────────────────────────────────────────
# D49 prompts
# ─────────────────────────────────────────────────────────────────────


def test_prompts_list_includes_d49_templates():
    names = {p.name for p in PROMPT_DESCRIPTORS}
    assert "deploy_new_bundle" in names
    assert "rollback_to_previous_bundle" in names


def test_prompts_total_post_d49_at_least_10(agent_native_deploy_dispatcher):
    """M17 (5) + D48 (3) + D49 (2) = 10.

    Audit fix M18-FINAL-A3 : `deploy_new_bundle` is gated to
    agent-native-deploy profile (it references write tools the
    read-only profile lacks). Use the deploy dispatcher to see all
    10 prompts ; agent-native sees 9.
    """
    prompts = agent_native_deploy_dispatcher.list_prompts()
    assert len(prompts) >= 10


def test_prompts_total_in_agent_native_excludes_deploy_only(
    agent_native_dispatcher,
):
    """The deploy_new_bundle prompt MUST be hidden in the read-only
    profile so agents don't follow a workflow that references missing
    tools (mid-flight tool-not-found error)."""
    names = {p["name"] for p in agent_native_dispatcher.list_prompts()}
    assert "deploy_new_bundle" not in names


def test_deploy_new_bundle_substitutes_slug_version():
    text = render_prompt(
        "deploy_new_bundle",
        {"slug": "support-routing", "version": "v3.2.0"},
    )
    assert "support-routing" in text
    assert "v3.2.0" in text
    assert "efe_check_bundle_compatibility" in text
    assert "efe_dry_run_deploy" in text
    assert "efe_generate_helm_values" in text


def test_deploy_new_bundle_default_environment_production():
    text = render_prompt(
        "deploy_new_bundle",
        {"slug": "x", "version": "v1.0.0"},
    )
    assert "production" in text


def test_deploy_new_bundle_explicit_environment():
    text = render_prompt(
        "deploy_new_bundle",
        {"slug": "x", "version": "v1.0.0", "environment": "staging"},
    )
    assert "staging" in text


def test_deploy_new_bundle_emphasises_no_kubectl_helm_execution():
    text = render_prompt(
        "deploy_new_bundle",
        {"slug": "x", "version": "v1.0.0"},
    )
    assert "NEVER" in text or "never" in text


def test_deploy_new_bundle_missing_required_args_raise():
    with pytest.raises(ValueError):
        render_prompt("deploy_new_bundle", {"slug": "x"})


def test_rollback_to_previous_bundle_uses_reason():
    text = render_prompt(
        "rollback_to_previous_bundle",
        {"reason": "p99 latency spike"},
    )
    assert "p99 latency spike" in text
    assert "efe_get_deployment_history" in text
    assert "efe_generate_rollback_plan" in text


def test_rollback_default_environment_production():
    text = render_prompt(
        "rollback_to_previous_bundle",
        {"reason": "bug"},
    )
    assert "production" in text


def test_rollback_emphasises_no_kubectl_execution():
    text = render_prompt(
        "rollback_to_previous_bundle",
        {"reason": "bug"},
    )
    assert "NEVER" in text or "never" in text


def test_rollback_missing_reason_raises():
    with pytest.raises(ValueError):
        render_prompt("rollback_to_previous_bundle", {})
