"""efe_register_bundle pin tests (M18 D50, write)."""
from __future__ import annotations

import pytest


@pytest.fixture
def register_seeded(fake_oci_registry, ed25519_keypair, fake_backend, monkeypatch):
    """Populate registry with a signed bundle + enable backend writes."""
    monkeypatch.setenv(
        "EFE_OCI_PUBLIC_KEY",
        ed25519_keypair["public_pem"].decode("utf-8"),
    )
    fake_oci_registry.add_bundle(
        repo="efe-project/bundles", slug="x", version="v1.0.0",
        sign_callback=ed25519_keypair["sign"],
    )
    fake_backend.enable_deploy_writes(True)


# ─────────────────────────────────────────────────────────────────────
# Confirm-required pattern (M16 carry-over)
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_register_without_confirm_rejected(agent_native_deploy_dispatcher):
    res = await agent_native_deploy_dispatcher.dispatch(
        tool_name="efe_register_bundle",
        args={
            "slug": "x", "version": "v1.0.0",
            "registry_url": "ghcr.io/efe-project/bundles",
            "confirm": False,
        },
    )
    assert res.is_error is True
    assert res.content["code"] == "confirm_required"


@pytest.mark.asyncio
async def test_register_missing_confirm_field_fails_schema(
    agent_native_deploy_dispatcher,
):
    res = await agent_native_deploy_dispatcher.dispatch(
        tool_name="efe_register_bundle",
        args={
            "slug": "x", "version": "v1.0.0",
            "registry_url": "ghcr.io/efe-project/bundles",
        },
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


# ─────────────────────────────────────────────────────────────────────
# Schema validation
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_register_invalid_slug_pattern_rejected(
    agent_native_deploy_dispatcher,
):
    res = await agent_native_deploy_dispatcher.dispatch(
        tool_name="efe_register_bundle",
        args={
            "slug": "INVALID", "version": "v1.0.0",
            "registry_url": "ghcr.io/x", "confirm": True,
        },
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_register_invalid_version_pattern_rejected(
    agent_native_deploy_dispatcher,
):
    res = await agent_native_deploy_dispatcher.dispatch(
        tool_name="efe_register_bundle",
        args={
            "slug": "x", "version": "1.0.0-no-prefix",
            "registry_url": "ghcr.io/x", "confirm": True,
        },
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_register_invalid_idempotency_key_rejected(
    agent_native_deploy_dispatcher,
):
    res = await agent_native_deploy_dispatcher.dispatch(
        tool_name="efe_register_bundle",
        args={
            "slug": "x", "version": "v1.0.0",
            "registry_url": "ghcr.io/x", "confirm": True,
            # Includes a `;` which the pattern forbids
            "idempotency_key": "key;rm-rf",
        },
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


# ─────────────────────────────────────────────────────────────────────
# Happy paths
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_register_happy_path_with_verify(
    agent_native_deploy_dispatcher, register_seeded, monkeypatch,
):
    monkeypatch.setenv("EFE_OCI_DEFAULT_REGISTRY", "ghcr.io/efe-project/bundles")
    res = await agent_native_deploy_dispatcher.dispatch(
        tool_name="efe_register_bundle",
        args={
            "slug": "x", "version": "v1.0.0",
            "registry_url": "ghcr.io/efe-project/bundles",
            "confirm": True,
        },
    )
    assert res.is_error is False, res.content
    assert res.content["registered"] is True
    assert res.content["slug"] == "x"
    assert res.content["version"] == "v1.0.0"
    # verify_before_register=True default → manifest_sha256 is set
    assert res.content["manifest_sha256"] is not None
    # verified_at_register reflects local Ed25519 verification
    assert res.content["verified_at_register"] is True


@pytest.mark.asyncio
async def test_register_skip_verify(
    agent_native_deploy_dispatcher, register_seeded, monkeypatch,
):
    monkeypatch.setenv("EFE_OCI_DEFAULT_REGISTRY", "ghcr.io/efe-project/bundles")
    res = await agent_native_deploy_dispatcher.dispatch(
        tool_name="efe_register_bundle",
        args={
            "slug": "x", "version": "v1.0.0",
            "registry_url": "ghcr.io/efe-project/bundles",
            "verify_before_register": False,
            "confirm": True,
        },
    )
    assert res.is_error is False
    assert res.content["verified_at_register"] is None
    assert res.content["verify_summary"] is None


@pytest.mark.asyncio
async def test_register_propagates_idempotency_key(
    agent_native_deploy_dispatcher, register_seeded, fake_backend, monkeypatch,
):
    monkeypatch.setenv("EFE_OCI_DEFAULT_REGISTRY", "ghcr.io/efe-project/bundles")
    res = await agent_native_deploy_dispatcher.dispatch(
        tool_name="efe_register_bundle",
        args={
            "slug": "x", "version": "v1.0.0",
            "registry_url": "ghcr.io/efe-project/bundles",
            "idempotency_key": "deploy-2026-05-02-attempt-1",
            "confirm": True,
        },
    )
    assert res.is_error is False
    register_calls = [
        c for c in fake_backend.calls if c["op"] == "deploy.register_bundle"
    ]
    assert register_calls
    assert register_calls[-1]["idempotency_key"] == "deploy-2026-05-02-attempt-1"


@pytest.mark.asyncio
async def test_register_unsupported_backend_returns_not_supported(
    agent_native_deploy_dispatcher, fake_backend, monkeypatch,
):
    monkeypatch.setenv("EFE_OCI_DEFAULT_REGISTRY", "ghcr.io/efe-project/bundles")
    fake_backend.enable_deploy_writes(False)
    res = await agent_native_deploy_dispatcher.dispatch(
        tool_name="efe_register_bundle",
        args={
            "slug": "x", "version": "v1.0.0",
            "registry_url": "ghcr.io/efe-project/bundles",
            "verify_before_register": False,
            "confirm": True,
        },
    )
    assert res.is_error is True
    assert res.content["code"] == "not_supported"


# ─────────────────────────────────────────────────────────────────────
# Profile gating — write tool MUST be hidden in agent-native (read-only)
# ─────────────────────────────────────────────────────────────────────


def test_register_bundle_NOT_in_agent_native_profile(agent_native_registry):
    """The READ-ONLY profile must NOT expose write tools."""
    assert "efe_register_bundle" not in agent_native_registry


def test_register_bundle_IN_agent_native_deploy_profile(
    agent_native_deploy_registry,
):
    assert "efe_register_bundle" in agent_native_deploy_registry
