"""efe_generate_helm_values + helm_values.render pin tests (M18 D49)."""
from __future__ import annotations

import pytest
import yaml

from mcp_server.deploy import (
    ALLOWED_HELM_SECTIONS,
    HelmValuesInput,
    render_helm_values,
)


# ─────────────────────────────────────────────────────────────────────
# Pure helm_values.render unit tests
# ─────────────────────────────────────────────────────────────────────


def _input(**overrides):
    base = dict(
        bundle_slug="support-routing",
        bundle_version="v1.0.0",
        bundle_manifest_sha256="abc" * 21 + "x",
    )
    base.update(overrides)
    return HelmValuesInput(**base)


def test_render_includes_header_with_bundle_id():
    out = render_helm_values(_input())
    assert "support-routing:v1.0.0" in out
    assert "manifest_sha256: abc" in out


def test_render_includes_chart_version_when_env_set(monkeypatch):
    monkeypatch.setenv("EFE_HELM_CHART_VERSION", "1.2.0")
    out = render_helm_values(_input())
    assert "chart_version : 1.2.0" in out


def test_render_chart_version_omitted_when_env_unset(monkeypatch):
    monkeypatch.delenv("EFE_HELM_CHART_VERSION", raising=False)
    out = render_helm_values(_input())
    assert "chart_version" not in out


def test_render_yaml_body_parses_back():
    out = render_helm_values(_input())
    # Strip the comment header and parse the body
    body = "\n".join(line for line in out.splitlines() if not line.startswith("#"))
    parsed = yaml.safe_load(body)
    assert parsed["bundle"]["slug"] == "support-routing"
    assert parsed["bundle"]["version"] == "v1.0.0"
    assert parsed["replicaCount"] == 3


def test_render_default_replicas_3():
    out = render_helm_values(_input())
    body = "\n".join(line for line in out.splitlines() if not line.startswith("#"))
    parsed = yaml.safe_load(body)
    assert parsed["replicaCount"] == 3


def test_render_pdb_enabled_by_default():
    out = render_helm_values(_input())
    body = "\n".join(line for line in out.splitlines() if not line.startswith("#"))
    parsed = yaml.safe_load(body)
    assert parsed["podDisruptionBudget"]["enabled"] is True


def test_render_excludes_pdb_when_include_pdb_false():
    out = render_helm_values(_input(include_pdb=False))
    body = "\n".join(line for line in out.splitlines() if not line.startswith("#"))
    parsed = yaml.safe_load(body)
    assert "podDisruptionBudget" not in parsed


def test_render_excludes_resources_when_include_resources_false():
    out = render_helm_values(_input(include_resources=False))
    body = "\n".join(line for line in out.splitlines() if not line.startswith("#"))
    parsed = yaml.safe_load(body)
    assert "resources" not in parsed


def test_render_includes_autoscaling_when_enabled():
    out = render_helm_values(_input(autoscaling_enabled=True, autoscaling_min=2, autoscaling_max=20))
    body = "\n".join(line for line in out.splitlines() if not line.startswith("#"))
    parsed = yaml.safe_load(body)
    assert parsed["autoscaling"]["enabled"] is True
    assert parsed["autoscaling"]["min"] == 2
    assert parsed["autoscaling"]["max"] == 20


def test_render_omits_autoscaling_when_disabled_default():
    out = render_helm_values(_input())
    body = "\n".join(line for line in out.splitlines() if not line.startswith("#"))
    parsed = yaml.safe_load(body)
    assert "autoscaling" not in parsed


def test_render_only_emits_whitelisted_keys():
    """TM-M18-E-02 mitigation : the output may contain ONLY keys from
    ALLOWED_HELM_SECTIONS."""
    out = render_helm_values(_input(autoscaling_enabled=True))
    body = "\n".join(line for line in out.splitlines() if not line.startswith("#"))
    parsed = yaml.safe_load(body)
    for key in parsed.keys():
        assert key in ALLOWED_HELM_SECTIONS, (
            f"key {key!r} not in whitelist {sorted(ALLOWED_HELM_SECTIONS)}"
        )


def test_render_deterministic_output_for_diff_friendliness():
    a = render_helm_values(_input())
    b = render_helm_values(_input())
    assert a == b


# ─────────────────────────────────────────────────────────────────────
# Tool dispatch tests
# ─────────────────────────────────────────────────────────────────────


@pytest.fixture
def deploy_seeded_backend(fake_oci_registry, monkeypatch):
    monkeypatch.setenv("EFE_OCI_DEFAULT_REGISTRY", "ghcr.io/efe-project/bundles")
    fake_oci_registry.add_bundle(
        repo="efe-project/bundles", slug="x", version="v1.0.0",
        annotations={
            "org.efe.bundle.framework": "hash",
            "org.efe.bundle.dim": "384",
        },
        layer_sizes=[1_000_000],
    )


@pytest.mark.asyncio
async def test_generate_helm_values_returns_yaml(
    agent_native_dispatcher, deploy_seeded_backend,
):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_generate_helm_values",
        args={"slug": "x", "version": "v1.0.0"},
    )
    assert res.is_error is False, res.content
    assert "values_yaml" in res.content
    assert "bundle:" in res.content["values_yaml"]
    assert res.content["size_bytes"] > 0


@pytest.mark.asyncio
async def test_generate_helm_values_pins_manifest_sha(
    agent_native_dispatcher, deploy_seeded_backend,
):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_generate_helm_values",
        args={"slug": "x", "version": "v1.0.0"},
    )
    assert "manifest_sha256" in res.content
    assert "manifest_sha256:" in res.content["values_yaml"]


@pytest.mark.asyncio
async def test_generate_helm_values_replicas_param(
    agent_native_dispatcher, deploy_seeded_backend,
):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_generate_helm_values",
        args={"slug": "x", "version": "v1.0.0", "replicas": 7},
    )
    assert "replicaCount: 7" in res.content["values_yaml"]


@pytest.mark.asyncio
async def test_generate_helm_values_autoscaling_min_gt_max_rejected(
    agent_native_dispatcher, deploy_seeded_backend,
):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_generate_helm_values",
        args={
            "slug": "x", "version": "v1.0.0",
            "autoscaling_enabled": True,
            "autoscaling_min": 10, "autoscaling_max": 3,
        },
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_generate_helm_values_unknown_bundle_returns_not_found(
    agent_native_dispatcher, monkeypatch,
):
    monkeypatch.setenv("EFE_OCI_DEFAULT_REGISTRY", "ghcr.io/efe-project/bundles")
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_generate_helm_values",
        args={"slug": "missing", "version": "v9.9.9"},
    )
    assert res.is_error is True
    assert res.content["code"] == "not_found"


@pytest.mark.asyncio
async def test_generate_helm_values_invalid_slug_pattern_rejected(
    agent_native_dispatcher,
):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_generate_helm_values",
        args={"slug": "INVALID-CAPS", "version": "v1.0.0"},
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_generate_helm_values_next_action_hint_never_runs_helm(
    agent_native_dispatcher, deploy_seeded_backend,
):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_generate_helm_values",
        args={"slug": "x", "version": "v1.0.0"},
    )
    hint = res.content["next_action_hint"]
    # Must mention --dry-run + emphasise that the MCP server doesn't
    # execute helm itself (TM-M18-E-02 transparency requirement).
    assert "--dry-run" in hint
    assert "NEVER" in hint
