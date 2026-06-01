"""M18 D48+D49 audit pass — pin tests for the 7 defects fixed.

Pattern carryover from M11-M17 audit passes. Every fix in this file
locks in correct behaviour so a future regression is caught loudly.
"""
from __future__ import annotations

import json

import pytest

from mcp_server.bundle.slsa import (
    infer_slsa_level,
    parse_provenance,
)
from mcp_server.deploy.dry_run import compute_diff
from mcp_server.resources.registry import (
    InvalidResourceUri,
    parse_resource_uri,
)
from mcp_server.state import BundleCache


# ─────────────────────────────────────────────────────────────────────
# A1 — SLSA infer_slsa_level requires signature for level 3 (MED)
# ─────────────────────────────────────────────────────────────────────


_TRUSTED_BUILDER = "https://github.com/actions/runner/v2.319.0"
_SLSA_BUILD_TYPE = (
    "https://slsa-framework.github.io/github-actions-buildtypes/workflow/v1"
)


def _trusted_attestation():
    return {
        "_type": "https://in-toto.io/Statement/v1",
        "subject": [{"name": "x", "digest": {"sha256": "a" * 64}}],
        "predicateType": "https://slsa.dev/provenance/v1",
        "predicate": {
            "buildDefinition": {"buildType": _SLSA_BUILD_TYPE},
            "runDetails": {
                "builder": {"id": _TRUSTED_BUILDER},
                "metadata": {"invocationId": "x"},
            },
        },
    }


def test_audit_a1_level_caps_at_2_without_signature_verified():
    """Pre-fix : trusted builder alone returned level 3.
    Post-fix : level 3 requires signature_verified=True."""
    p = parse_provenance(_trusted_attestation())
    assert infer_slsa_level(p) == 2  # default signature_verified=False
    assert infer_slsa_level(p, signature_verified=False) == 2


def test_audit_a1_level_3_only_with_trusted_builder_and_signed():
    p = parse_provenance(_trusted_attestation())
    assert infer_slsa_level(p, signature_verified=True) == 3


def test_audit_a1_level_2_with_signed_but_untrusted_builder():
    body = _trusted_attestation()
    body["predicate"]["runDetails"]["builder"]["id"] = (
        "https://internal.example.com/jenkins"
    )
    p = parse_provenance(body)
    assert infer_slsa_level(p, signature_verified=True) == 2


# ─────────────────────────────────────────────────────────────────────
# A2 — bundle_cache namespace isolation (HIGH)
# ─────────────────────────────────────────────────────────────────────


def test_audit_a2_cache_keys_differ_by_namespace():
    cache = BundleCache(capacity=8, ttl_seconds=600.0)
    cache.put(
        tenant_id="t1", slug="x", version="v1", value={"shape": "tool"},
        namespace="inspect_tool",
    )
    cache.put(
        tenant_id="t1", slug="x", version="v1", value={"shape": "resource"},
        namespace="resource_get_bundle",
    )
    a = cache.get(tenant_id="t1", slug="x", version="v1", namespace="inspect_tool")
    b = cache.get(tenant_id="t1", slug="x", version="v1", namespace="resource_get_bundle")
    assert a == {"shape": "tool"}
    assert b == {"shape": "resource"}


def test_audit_a2_default_namespace_isolated_from_tagged():
    cache = BundleCache(capacity=8, ttl_seconds=600.0)
    cache.put(tenant_id="t1", slug="x", version="v1", value={"k": "default"})
    cache.put(
        tenant_id="t1", slug="x", version="v1", value={"k": "tagged"},
        namespace="inspect_tool",
    )
    assert cache.get(
        tenant_id="t1", slug="x", version="v1",
    ) == {"k": "default"}
    assert cache.get(
        tenant_id="t1", slug="x", version="v1", namespace="inspect_tool",
    ) == {"k": "tagged"}


@pytest.mark.asyncio
async def test_audit_a2_inspect_then_resource_no_cross_contamination(
    agent_native_dispatcher, fake_oci_registry, ed25519_keypair, monkeypatch,
):
    """End-to-end : calling efe_inspect_bundle then reading the bundle
    resource for the SAME slug:version must NOT yield the tool's shape
    from cache (different namespaces enforce shape isolation)."""
    monkeypatch.setenv("EFE_OCI_DEFAULT_REGISTRY", "ghcr.io/efe-project/bundles")
    monkeypatch.setenv(
        "EFE_OCI_PUBLIC_KEY",
        ed25519_keypair["public_pem"].decode("utf-8"),
    )
    fake_oci_registry.add_bundle(
        repo="efe-project/bundles", slug="cache-test", version="v1.0.0",
        sign_callback=ed25519_keypair["sign"],
        layer_sizes=[1_000_000],
    )
    # 1) Tool path puts a shape in the inspect_tool namespace
    tool_res = await agent_native_dispatcher.dispatch(
        tool_name="efe_inspect_bundle",
        args={"slug": "cache-test", "version": "v1.0.0"},
    )
    assert tool_res.is_error is False
    # The tool shape carries `compatibility` + `manifest.layer_count`
    assert "compatibility" in tool_res.content
    # 2) Resource path returns a different shape — must not collide
    res_payload = await agent_native_dispatcher.read_resource(
        "efe://bundles/cache-test/v1.0.0"
    )
    body = json.loads(res_payload["text"])
    # The resource shape lacks `compatibility`, has `cache_hit`
    assert "cache_hit" in body
    # If cache had collided, body would have had `compatibility` field
    # (from the tool's shape) instead of `cache_hit`.


# ─────────────────────────────────────────────────────────────────────
# A3 — deployment resource fallback hits /admin/bundle-info (LOW)
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_audit_a3_deployment_resource_uses_bundle_info_fallback(
    agent_native_dispatcher, fake_backend, monkeypatch,
):
    """Pre-fix : resource fallback returned bundle.slug='unknown' even
    when /admin/bundle-info had data. Post-fix : it tries the public
    `get_admin_bundle_info()` method first."""
    fake_backend.set_deployment_state(None)  # forces fallback path

    # Patch `get_admin_bundle_info` to return real data
    async def fake_admin_bundle_info(*, auth):
        return {
            "bundle_slug": "support-routing",
            "bundle_id": "support-routing-v3.2.0",
            "manifestSha256": "abc" * 21 + "x",
        }

    monkeypatch.setattr(
        fake_backend, "get_admin_bundle_info", fake_admin_bundle_info,
        raising=False,
    )
    res = await agent_native_dispatcher.read_resource(
        "efe://deployments/current"
    )
    body = json.loads(res["text"])
    assert body["bundle"]["slug"] == "support-routing"
    assert body["bundle"]["version"] == "support-routing-v3.2.0"
    assert body["bundle"]["manifest_sha256"] is not None


# ─────────────────────────────────────────────────────────────────────
# A4 — URI version regex matches tool schema (LOW)
# ─────────────────────────────────────────────────────────────────────


def test_audit_a4_uri_version_accepts_4_digit_part():
    p = parse_resource_uri("efe://bundles/x/v1234.0.0")
    assert p.bundle_version == "v1234.0.0"


def test_audit_a4_uri_version_accepts_5_digit_part():
    p = parse_resource_uri("efe://bundles/x/v99999.0.0")
    assert p.bundle_version == "v99999.0.0"


def test_audit_a4_uri_version_rejects_6_digit_part():
    """Reasonable upper bound at 5 digits per part."""
    with pytest.raises(InvalidResourceUri):
        parse_resource_uri("efe://bundles/x/v999999.0.0")


def test_audit_a4_uri_version_still_accepts_pre_release():
    p = parse_resource_uri("efe://bundles/x/v1.0.0-rc1")
    assert p.bundle_version == "v1.0.0-rc1"


# ─────────────────────────────────────────────────────────────────────
# A5 — rollback plan rejects shell metacharacters (HIGH)
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_audit_a5_namespace_with_shell_metachar_rejected(
    agent_native_dispatcher, fake_backend,
):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_generate_rollback_plan",
        args={
            "reason": "test",
            "namespace": "efe-prod; rm -rf /",
        },
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_audit_a5_release_name_with_backtick_rejected(
    agent_native_dispatcher, fake_backend,
):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_generate_rollback_plan",
        args={
            "reason": "test",
            "release_name": "efe`whoami`",
        },
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_audit_a5_to_version_with_pipe_rejected(
    agent_native_dispatcher, fake_backend,
):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_generate_rollback_plan",
        args={
            "reason": "test",
            "to_version": "v1.0.0 | tee /etc/passwd",
        },
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_audit_a5_environment_with_dollar_rejected(
    agent_native_dispatcher, fake_backend,
):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_generate_rollback_plan",
        args={
            "reason": "test",
            "environment": "prod$(curl evil)",
        },
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_audit_a5_reason_with_newline_rejected(
    agent_native_dispatcher, fake_backend,
):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_generate_rollback_plan",
        args={"reason": "legitimate\n# evil markdown injection"},
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_audit_a5_valid_namespace_release_still_accepted(
    agent_native_dispatcher, fake_backend,
):
    fake_backend.set_deployment_state({"bundle": {"slug": "x", "version": "v1"}})
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_generate_rollback_plan",
        args={
            "reason": "p99 latency spike post-deploy",
            "namespace": "efe-staging",
            "release_name": "efe-canary",
            "to_version": "v3.0.0",
            "environment": "staging",
        },
    )
    assert res.is_error is False, res.content


# ─────────────────────────────────────────────────────────────────────
# A6 — get_deployment_status uses public API (LOW)
# ─────────────────────────────────────────────────────────────────────


def test_audit_a6_backend_client_has_get_admin_bundle_info():
    from mcp_server.backend_client import BackendClient
    assert hasattr(BackendClient, "get_admin_bundle_info")


@pytest.mark.asyncio
async def test_audit_a6_status_no_longer_uses_private_request(
    agent_native_dispatcher, fake_backend,
):
    """Status fallback path should use the public `get_admin_bundle_info`
    method, not `_request()` directly."""
    fake_backend.set_deployment_state(None)
    # The fallback path runs and must NOT raise even when
    # get_admin_bundle_info is not implemented (raises NotImplementedError
    # which the tool catches gracefully).
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_get_deployment_status", args={},
    )
    assert res.is_error is False
    assert res.content["source"] == "fallback_info_compose"


# ─────────────────────────────────────────────────────────────────────
# A7 — dry_run tri-state signature handling (HIGH)
# ─────────────────────────────────────────────────────────────────────


def _target_manifest(**overrides):
    base = {
        "slug": "x", "version": "v1.0.0",
        "manifestSha256": "newsha", "framework": "hash",
        "model_name": "fake-model", "dim": 384, "is_dev_unsigned": False,
    }
    base.update(overrides)
    return base


def test_audit_a7_unverifiable_signature_is_medium_not_blocker():
    """Pre-fix : verified=False from missing trust store → BLOCKER.
    Post-fix : unverifiable=True → MEDIUM, not blocking."""
    diff = compute_diff(
        current=None,
        target_manifest=_target_manifest(),
        target_signature_verified=False,
        target_signature_unverifiable=True,
    )
    assert diff.has_blockers is False
    assert any(r.category == "signature_unverifiable" for r in diff.risks)
    assert all(r.category != "signature_unverified" for r in diff.risks)


def test_audit_a7_actual_failure_still_blocks():
    """When verified=False AND unverifiable=False, it's a real
    cryptographic failure → BLOCKER."""
    diff = compute_diff(
        current=None,
        target_manifest=_target_manifest(),
        target_signature_verified=False,
        target_signature_unverifiable=False,
    )
    assert diff.has_blockers is True
    assert any(r.category == "signature_unverified" for r in diff.risks)


def test_audit_a7_verified_true_no_signature_risk():
    diff = compute_diff(
        current=None,
        target_manifest=_target_manifest(),
        target_signature_verified=True,
    )
    assert all(
        not r.category.startswith("signature_") for r in diff.risks
    )


def test_audit_a7_unknown_signature_is_medium():
    """target_signature_verified=None → MEDIUM signature_unknown."""
    diff = compute_diff(
        current=None,
        target_manifest=_target_manifest(),
        target_signature_verified=None,
    )
    assert any(r.category == "signature_unknown" for r in diff.risks)
    assert diff.has_blockers is False


@pytest.mark.asyncio
async def test_audit_a7_dry_run_with_no_trust_store_not_blocked(
    agent_native_dispatcher, fake_oci_registry, fake_backend,
    ed25519_keypair, monkeypatch,
):
    """Real end-to-end : a SIGNED bundle with no trust store configured
    must NOT produce a BLOCKER (just a MEDIUM warning)."""
    monkeypatch.delenv("EFE_OCI_PUBLIC_KEY", raising=False)
    monkeypatch.delenv("EFE_OCI_PUBLIC_KEY_FILE", raising=False)
    monkeypatch.setenv("EFE_OCI_DEFAULT_REGISTRY", "ghcr.io/efe-project/bundles")
    fake_oci_registry.add_bundle(
        repo="efe-project/bundles", slug="x", version="v2.0.0",
        sign_callback=ed25519_keypair["sign"],
    )
    fake_backend.set_deployment_state({
        "bundle": {"slug": "x", "version": "v1.0.0", "manifest_sha256": "old"},
        "manifest": {"framework": "hash", "dim": 384},
    })
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_dry_run_deploy",
        args={"slug": "x", "version": "v2.0.0"},
    )
    assert res.is_error is False
    # Signature path : MEDIUM "signature_unverifiable", not BLOCKER.
    risks = res.content.get("risks", [])
    sig_risks = [r for r in risks if r["category"].startswith("signature_")]
    if sig_risks:
        # Allowed : MEDIUM signature_unverifiable. Forbidden : BLOCKER
        # signature_unverified (false-positive on un-configured trust store).
        for r in sig_risks:
            assert r["level"] != "BLOCKER" or r["category"] != "signature_unverified"
