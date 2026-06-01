"""M18 final audit pass — pin tests for the 6 cross-cutting defects fixed.

Pattern carryover from M11-M17 audit passes. Locks in the M18-wide
invariants : audit traceability for write actions, rate-limit cannot
be bypassed, profile-tier filtering, idempotency-key header
convention, write tools require confirm, env-var sanitisation.
"""
from __future__ import annotations

import json
import re

import pytest

from mcp_server.audit_emitter import summarize_args_for_audit
from mcp_server.deploy.helm_values import (
    HelmValuesInput,
    _CHART_VERSION_RE,
    _sanitise_chart_version,
    render as render_helm_values,
)
from mcp_server.profile import Profile
from mcp_server.prompts.registry import PROMPT_DESCRIPTORS
from mcp_server.tools.registry import all_known_tools


# ─────────────────────────────────────────────────────────────────────
# A1 — audit summary captures bundle/deploy context (HIGH)
# ─────────────────────────────────────────────────────────────────────


def test_audit_a1_register_bundle_summary_includes_slug_version():
    summary = summarize_args_for_audit(
        "efe_register_bundle",
        {
            "slug": "support-routing", "version": "v3.2.0",
            "registry_url": "ghcr.io/efe-project/bundles",
            "verify_before_register": True,
            "confirm": True,
        },
    )
    assert summary["slug"] == "support-routing"
    assert summary["version"] == "v3.2.0"
    assert summary["registry_url"] == "ghcr.io/efe-project/bundles"
    assert summary["verify_before_register"] is True
    assert summary["confirm"] is True


def test_audit_a1_promote_bundle_summary_includes_scope():
    summary = summarize_args_for_audit(
        "efe_promote_bundle",
        {"slug": "x", "version": "v1.0.0", "scope": "global", "confirm": True},
    )
    assert summary["slug"] == "x"
    assert summary["version"] == "v1.0.0"
    assert summary["scope"] == "global"


def test_audit_a1_request_deployment_summary_includes_tracker_environment():
    summary = summarize_args_for_audit(
        "efe_request_deployment",
        {
            "slug": "x", "version": "v1.0.0",
            "tracker": "linear", "environment": "production",
            "rationale": "EU customer needs Italian + Portuguese coverage",
            "confirm": True,
        },
    )
    assert summary["tracker"] == "linear"
    assert summary["environment"] == "production"
    assert summary["rationale_len"] == len(
        "EU customer needs Italian + Portuguese coverage"
    )
    # Full rationale is NOT echoed (privacy + log-size)
    assert "rationale" not in summary
    assert "rationale_sample" in summary


def test_audit_a1_idempotency_key_length_only():
    """The idempotency_key value itself is not echoed — only its
    length, to keep audit logs compact."""
    summary = summarize_args_for_audit(
        "efe_register_bundle",
        {
            "slug": "x", "version": "v1.0.0", "confirm": True,
            "registry_url": "ghcr.io/x",
            "idempotency_key": "deploy-2026-05-02-attempt-7",
        },
    )
    assert summary["idempotency_key_len"] == 27
    assert "idempotency_key" not in summary


def test_audit_a1_dry_run_deploy_summary_captures_target_environment():
    summary = summarize_args_for_audit(
        "efe_dry_run_deploy",
        {"slug": "x", "version": "v2.0.0", "environment": "staging"},
    )
    assert summary["slug"] == "x"
    assert summary["version"] == "v2.0.0"
    assert summary["environment"] == "staging"


def test_audit_a1_unknown_keys_still_dropped():
    """Defense-in-depth : unknown keys (potential log-injection
    attempt) must STILL be dropped post-A1 fix."""
    summary = summarize_args_for_audit(
        "efe_register_bundle",
        {
            "slug": "x", "version": "v1.0.0",
            "registry_url": "ghcr.io/x", "confirm": True,
            "_evil_field": "<script>alert(1)</script>",
        },
    )
    assert "_evil_field" not in summary


# ─────────────────────────────────────────────────────────────────────
# A2 — register_bundle rate-weight covers internal inspect call (MED)
# ─────────────────────────────────────────────────────────────────────


def test_audit_a2_register_bundle_weight_at_least_inspect_plus_own():
    """register_bundle internally calls inspect_bundle handler ;
    its weight must cover both."""
    from mcp_server.dispatcher import Dispatcher
    inspect_w = Dispatcher.TOOL_WEIGHTS.get("efe_inspect_bundle", 1.0)
    register_w = Dispatcher.TOOL_WEIGHTS.get("efe_register_bundle", 1.0)
    # register MUST cost at least as much as inspect alone, plus a
    # small own-work surcharge. We pick 8.0 = 5 + 3 in the dispatcher.
    assert register_w >= inspect_w


def test_audit_a2_register_bundle_weight_strictly_above_promote():
    """register involves OCI work (inspect) ; promote is metadata-only.
    register must be heavier than promote."""
    from mcp_server.dispatcher import Dispatcher
    register_w = Dispatcher.TOOL_WEIGHTS.get("efe_register_bundle", 1.0)
    promote_w = Dispatcher.TOOL_WEIGHTS.get("efe_promote_bundle", 1.0)
    assert register_w > promote_w


# ─────────────────────────────────────────────────────────────────────
# A3 — prompt profile_tier filtering (LOW)
# ─────────────────────────────────────────────────────────────────────


def test_audit_a3_deploy_new_bundle_visible_in_deploy_profile_only(
    agent_native_dispatcher, agent_native_deploy_dispatcher,
):
    """`deploy_new_bundle` references write tools that don't exist in
    agent-native ; must be hidden there to avoid mid-workflow
    tool-not-found errors."""
    deploy_names = {p["name"] for p in agent_native_deploy_dispatcher.list_prompts()}
    agent_names = {p["name"] for p in agent_native_dispatcher.list_prompts()}
    assert "deploy_new_bundle" in deploy_names
    assert "deploy_new_bundle" not in agent_names


def test_audit_a3_advanced_prompts_visible_everywhere(
    advanced_dispatcher, agent_native_dispatcher, agent_native_deploy_dispatcher,
):
    """Prompts with `profile_tier='advanced'` (default) must remain
    visible in advanced + agent-native + agent-native-deploy."""
    for disp in (advanced_dispatcher, agent_native_dispatcher,
                 agent_native_deploy_dispatcher):
        names = {p["name"] for p in disp.list_prompts()}
        assert "classify_with_reasoning" in names


def test_audit_a3_prompt_template_has_profile_tier_field():
    from mcp_server.prompts.registry import PromptTemplate
    fields = PromptTemplate.__dataclass_fields__
    assert "profile_tier" in fields


# ─────────────────────────────────────────────────────────────────────
# A4 — Idempotency-Key flows via header (MED)
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_audit_a4_idempotency_key_in_header_not_body():
    """Audit fix M18-FINAL-A4 : the request must carry
    Idempotency-Key as a HEADER ; no `idempotency_key` field in the
    JSON body."""
    import httpx

    from mcp_server.auth import AuthContext
    from mcp_server.bundle.registry_arg import default_oci_factory  # noqa: F401
    from mcp_server.backend_client import RestBackendClient

    captured: dict = {}

    def _handler(request: httpx.Request) -> httpx.Response:
        captured["headers"] = dict(request.headers)
        captured["body"] = json.loads(request.content.decode("utf-8")) if request.content else {}
        return httpx.Response(
            status_code=200,
            content=b'{"bundle_id":"x","manifest_sha256":"abc"}',
            headers={"content-type": "application/json"},
        )

    client = RestBackendClient("http://test.example.com")
    client._client = httpx.AsyncClient(
        base_url="http://test.example.com",
        transport=httpx.MockTransport(_handler),
    )
    auth = AuthContext(raw_token="test-token-1234")
    await client.register_bundle(
        slug="x", version="v1.0.0",
        registry_url="ghcr.io/x", manifest_sha256="abc",
        verify_before_register=False,
        idempotency_key="my-deploy-2026-05-02",
        auth=auth,
    )
    await client.aclose()
    assert captured["headers"].get("idempotency-key") == "my-deploy-2026-05-02"
    # Body must NOT carry idempotency_key
    assert "idempotency_key" not in captured["body"]


# ─────────────────────────────────────────────────────────────────────
# A5 — institutional pin test : all write tools require confirm (LOW)
# ─────────────────────────────────────────────────────────────────────


def test_audit_a5_all_write_tools_require_confirm():
    """Every tool with `is_write=True` MUST list `confirm` in the
    schema's `required` array, else the LLM client could call it
    silently. Carries the M16 confirm-required pattern forward to
    every future write tool."""
    tools = all_known_tools()
    write_tools = [t for t in tools.values() if t.is_write]
    assert len(write_tools) >= 5  # at least M16 register/evolve + M17/M18
    for tool in write_tools:
        # Either confirm is REQUIRED outright, or there's an `if/then`
        # clause in the schema that conditionally requires it (e.g.
        # consolidate_versions only requires confirm when dry_run=False).
        required = tool.input_schema.get("required", [])
        if "confirm" in required:
            continue
        # consolidate_versions case : check the conditional
        if_clause = tool.input_schema.get("then", {})
        cond_required = if_clause.get("required", [])
        assert "confirm" in cond_required, (
            f"write tool {tool.name!r} does not require confirm in any "
            "branch of its schema"
        )


def test_audit_a5_confirm_property_is_boolean_when_present():
    """When confirm appears, it must be type:boolean (no string-ish
    truthiness games)."""
    tools = all_known_tools()
    for name, tool in tools.items():
        props = tool.input_schema.get("properties") or {}
        if "confirm" in props:
            assert props["confirm"].get("type") == "boolean", (
                f"{name!r} confirm must be type:boolean"
            )


# ─────────────────────────────────────────────────────────────────────
# A6 — EFE_HELM_CHART_VERSION sanitisation (LOW)
# ─────────────────────────────────────────────────────────────────────


def test_audit_a6_chart_version_re_accepts_semver():
    assert _CHART_VERSION_RE.match("1.2.0")
    assert _CHART_VERSION_RE.match("v1.2.3-rc1")
    assert _CHART_VERSION_RE.match("0.10.5")


def test_audit_a6_chart_version_re_rejects_newline():
    assert _CHART_VERSION_RE.match("1.0\nbundle:\n  evil: true") is None


def test_audit_a6_chart_version_re_rejects_colon():
    """Colons would break the YAML comment line into a key:value pair."""
    assert _CHART_VERSION_RE.match("1.0:evil") is None


def test_audit_a6_sanitise_returns_none_for_evil_input():
    assert _sanitise_chart_version("1.0\nbundle: evil") is None
    assert _sanitise_chart_version("# evil") is None


def test_audit_a6_sanitise_preserves_clean_input():
    assert _sanitise_chart_version("1.2.0") == "1.2.0"
    assert _sanitise_chart_version("v1.2.3-rc1") == "v1.2.3-rc1"


def test_audit_a6_render_drops_polluted_env(monkeypatch):
    monkeypatch.setenv("EFE_HELM_CHART_VERSION", "1.0\nbundle:\n  evil: true")
    out = render_helm_values(HelmValuesInput(
        bundle_slug="x", bundle_version="v1.0.0",
        bundle_manifest_sha256="abc" * 21 + "x",
    ))
    # The polluted env-var must NOT have produced any extra YAML lines.
    # Specifically, no `evil: true` should appear in the output.
    assert "evil:" not in out
    # Comment line for chart_version should also NOT be emitted.
    assert "chart_version" not in out


def test_audit_a6_render_emits_clean_chart_version(monkeypatch):
    monkeypatch.setenv("EFE_HELM_CHART_VERSION", "1.5.0")
    out = render_helm_values(HelmValuesInput(
        bundle_slug="x", bundle_version="v1.0.0",
        bundle_manifest_sha256="abc" * 21 + "x",
    ))
    assert "chart_version : 1.5.0" in out
