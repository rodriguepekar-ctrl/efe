"""efe_inspect_bundle + efe_verify_bundle_signature pin tests (M18 D47)."""
from __future__ import annotations

import json

import pytest


@pytest.fixture
def populated_registry(fake_oci_registry, ed25519_keypair, monkeypatch):
    """Populate the fake OCI registry with a signed bundle + SBOM.

    Audit fix M18-D47-A5 (LOW) : uses `sign_callback` so the signature
    is computed over the canonical manifest bytes in a single
    `add_bundle` call (no double-add+pop).
    """
    monkeypatch.setenv("EFE_OCI_PUBLIC_KEY", ed25519_keypair["public_pem"].decode("utf-8"))
    monkeypatch.setenv("EFE_OCI_DEFAULT_REGISTRY", "ghcr.io/efe-project/bundles")

    sbom = {
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "components": [
            {"name": "torch", "version": "2.1.0", "type": "library", "purl": "pkg:pypi/torch@2.1.0"},
            {"name": "transformers", "version": "4.35.0", "type": "library", "purl": "pkg:pypi/transformers@4.35.0"},
        ],
        "vulnerabilities": [],
    }
    sbom_bytes = json.dumps(sbom).encode("utf-8")

    sha = fake_oci_registry.add_bundle(
        repo="efe-project/bundles",
        slug="support-routing",
        version="v3.2.0",
        annotations={
            "org.efe.bundle.framework": "sentence-transformers",
            "org.efe.bundle.model_name": "all-MiniLM-L6-v2",
            "org.efe.bundle.dim": "384",
            "org.efe.bundle.is_dev_unsigned": "false",
            "org.efe.bundle.produced_at": "2026-04-15T10:30:00Z",
            "org.efe.bundle.produced_by": "github.com/efe-project/efe-bundles",
        },
        sign_callback=ed25519_keypair["sign"],
        sbom_bytes=sbom_bytes,
        layer_sizes=[1_400_000, 50_000],   # ~1.45 MB total
    )
    return {"sha256": sha}


@pytest.mark.asyncio
async def test_inspect_bundle_returns_full_metadata(
    agent_native_dispatcher, populated_registry,
):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_inspect_bundle",
        args={"slug": "support-routing", "version": "v3.2.0"},
    )
    assert res.is_error is False, res.content
    assert res.content["slug"] == "support-routing"
    assert res.content["version"] == "v3.2.0"
    assert "manifest" in res.content
    assert res.content["manifest"]["framework"] == "sentence-transformers"
    assert res.content["manifest"]["dim"] == 384


@pytest.mark.asyncio
async def test_inspect_bundle_signature_verified(
    agent_native_dispatcher, populated_registry,
):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_inspect_bundle",
        args={"slug": "support-routing", "version": "v3.2.0"},
    )
    sig = res.content["signature"]
    assert sig["is_signed"] is True
    assert sig["verified"] is True


@pytest.mark.asyncio
async def test_inspect_bundle_sbom_summary_components_count(
    agent_native_dispatcher, populated_registry,
):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_inspect_bundle",
        args={"slug": "support-routing", "version": "v3.2.0"},
    )
    sbom = res.content["sbom_summary"]
    assert sbom["components_count"] == 2
    assert sbom["vulnerabilities_count"] == 0


@pytest.mark.asyncio
async def test_inspect_bundle_with_full_sbom_includes_components_list(
    agent_native_dispatcher, populated_registry,
):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_inspect_bundle",
        args={
            "slug": "support-routing", "version": "v3.2.0",
            "include_full_sbom": True,
        },
    )
    assert "sbom" in res.content
    components = res.content["sbom"]["components"]
    names = {c["name"] for c in components}
    assert "torch" in names
    assert "transformers" in names


@pytest.mark.asyncio
async def test_inspect_bundle_unknown_slug_returns_not_found(
    agent_native_dispatcher, monkeypatch,
):
    monkeypatch.setenv("EFE_OCI_DEFAULT_REGISTRY", "ghcr.io/efe-project/bundles")
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_inspect_bundle",
        args={"slug": "missing", "version": "v1.0.0"},
    )
    assert res.is_error is True
    assert res.content["code"] == "not_found"


@pytest.mark.asyncio
async def test_inspect_bundle_invalid_slug_pattern_rejected(
    agent_native_dispatcher,
):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_inspect_bundle",
        args={"slug": "Invalid-Caps", "version": "v1.0.0"},
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_inspect_bundle_invalid_version_pattern_rejected(
    agent_native_dispatcher,
):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_inspect_bundle",
        args={"slug": "x", "version": "1.0.0-no-v-prefix"},
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_verify_bundle_signature_returns_verified_true(
    agent_native_dispatcher, populated_registry,
):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_verify_bundle_signature",
        args={"slug": "support-routing", "version": "v3.2.0"},
    )
    assert res.is_error is False, res.content
    assert res.content["verified"] is True
    assert res.content["details"]["manifest_sha256_matches"] is True


@pytest.mark.asyncio
async def test_verify_bundle_signature_unsigned_returns_false(
    agent_native_dispatcher, fake_oci_registry, ed25519_keypair, monkeypatch,
):
    monkeypatch.setenv("EFE_OCI_PUBLIC_KEY", ed25519_keypair["public_pem"].decode("utf-8"))
    monkeypatch.setenv("EFE_OCI_DEFAULT_REGISTRY", "ghcr.io/efe-project/bundles")
    fake_oci_registry.add_bundle(
        repo="efe-project/bundles", slug="unsigned", version="v1.0.0",
    )
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_verify_bundle_signature",
        args={"slug": "unsigned", "version": "v1.0.0"},
    )
    assert res.is_error is False
    assert res.content["verified"] is False
    assert res.content["error"] == "unsigned_bundle"


@pytest.mark.asyncio
async def test_verify_bundle_signature_no_trust_store_returns_unavailable(
    agent_native_dispatcher, populated_registry, monkeypatch,
):
    monkeypatch.delenv("EFE_OCI_PUBLIC_KEY", raising=False)
    monkeypatch.delenv("EFE_OCI_PUBLIC_KEY_FILE", raising=False)
    monkeypatch.setenv("EFE_OCI_DEFAULT_REGISTRY", "ghcr.io/efe-project/bundles")
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_verify_bundle_signature",
        args={"slug": "support-routing", "version": "v3.2.0"},
    )
    assert res.content.get("verification_unavailable") is True
    assert res.content["verified"] is False
