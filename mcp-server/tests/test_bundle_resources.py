"""MCP `efe://bundles/...` resource pin tests (M18 D48)."""
from __future__ import annotations

import json

import pytest

from mcp_server.resources.registry import (
    InvalidResourceUri,
    parse_resource_uri,
)


# ─────────────────────────────────────────────────────────────────────
# URI grammar
# ─────────────────────────────────────────────────────────────────────


def test_parse_bundles_list_uri():
    p = parse_resource_uri("efe://bundles")
    assert p.kind == "bundles_list"


def test_parse_bundle_uri_full_form():
    p = parse_resource_uri("efe://bundles/support-routing/v3.2.0")
    assert p.kind == "bundle"
    assert p.bundle_slug == "support-routing"
    assert p.bundle_version == "v3.2.0"


def test_parse_bundle_sbom_subresource():
    p = parse_resource_uri("efe://bundles/x/v1.0.0/sbom")
    assert p.kind == "bundle_sbom"
    assert p.bundle_slug == "x"
    assert p.bundle_version == "v1.0.0"
    assert p.bundle_subresource == "sbom"


def test_parse_bundle_signature_subresource():
    p = parse_resource_uri("efe://bundles/x/v1.0.0/signature")
    assert p.kind == "bundle_signature"
    assert p.bundle_subresource == "signature"


def test_parse_bundle_invalid_slug_pattern_rejected():
    with pytest.raises(InvalidResourceUri):
        parse_resource_uri("efe://bundles/Invalid-Caps/v1.0.0")


def test_parse_bundle_invalid_version_pattern_rejected():
    with pytest.raises(InvalidResourceUri):
        parse_resource_uri("efe://bundles/x/1.0.0-no-v-prefix")


def test_parse_bundle_unknown_subresource_rejected():
    with pytest.raises(InvalidResourceUri):
        parse_resource_uri("efe://bundles/x/v1.0.0/secret")


def test_parse_bundle_traversal_rejected():
    with pytest.raises(InvalidResourceUri):
        parse_resource_uri("efe://bundles/../etc/passwd")


def test_parse_bundle_uri_accepts_registry_query():
    p = parse_resource_uri(
        "efe://bundles/x/v1.0.0?registry=ghcr.io%2Fefe-project%2Fbundles"
    )
    assert p.kind == "bundle"
    assert (p.query or {}).get("registry") == "ghcr.io/efe-project/bundles"


def test_parse_bundle_uri_with_pre_release_version():
    p = parse_resource_uri("efe://bundles/x/v1.0.0-rc1")
    assert p.kind == "bundle"
    assert p.bundle_version == "v1.0.0-rc1"


# ─────────────────────────────────────────────────────────────────────
# Resource list / advertising
# ─────────────────────────────────────────────────────────────────────


def test_resources_list_includes_4_bundle_descriptors(agent_native_dispatcher):
    resources = agent_native_dispatcher.list_resources()
    bundle_uris = {r["uri"] for r in resources if r["uri"].startswith("efe://bundles")}
    assert "efe://bundles" in bundle_uris
    assert "efe://bundles/{slug}/{version}" in bundle_uris
    assert "efe://bundles/{slug}/{version}/sbom" in bundle_uris
    assert "efe://bundles/{slug}/{version}/signature" in bundle_uris


# ─────────────────────────────────────────────────────────────────────
# Resource read (end-to-end via dispatcher)
# ─────────────────────────────────────────────────────────────────────


@pytest.fixture
def bundle_seeded(fake_oci_registry, ed25519_keypair, monkeypatch, slsa_attestation_factory):
    monkeypatch.setenv("EFE_OCI_DEFAULT_REGISTRY", "ghcr.io/efe-project/bundles")
    monkeypatch.setenv("EFE_OCI_PUBLIC_KEY", ed25519_keypair["public_pem"].decode("utf-8"))
    sbom = json.dumps({
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "components": [{"name": "torch", "version": "2.1.0", "type": "library"}],
        "vulnerabilities": [],
    }).encode("utf-8")
    # Compute the manifest sha first by adding without SLSA
    sha = fake_oci_registry.add_bundle(
        repo="efe-project/bundles",
        slug="resource-test",
        version="v1.0.0",
        sign_callback=ed25519_keypair["sign"],
        sbom_bytes=sbom,
        slsa_attestation_bytes=slsa_attestation_factory(
            manifest_sha256="placeholder",  # will be wrong on first call
        ),
        layer_sizes=[1_000_000],
    )
    # Re-attach a SLSA attestation that has the correct subject digest.
    # We do this by removing+re-inserting the SLSA referrer.
    referrers = fake_oci_registry._referrers.get(f"sha256:{sha}", [])
    real_attestation = slsa_attestation_factory(manifest_sha256=sha)
    new_digest = fake_oci_registry._store_blob(real_attestation)
    for ref in referrers:
        if ref.get("artifactType") == "application/vnd.in-toto+json":
            ref["layers"][0]["digest"] = new_digest
            ref["layers"][0]["size"] = len(real_attestation)
    return {"sha": sha}


@pytest.mark.asyncio
async def test_read_bundles_list_resource(agent_native_dispatcher, bundle_seeded):
    res = await agent_native_dispatcher.read_resource("efe://bundles")
    assert "uri" in res
    body = json.loads(res["text"])
    # Either backend or oci_direct source
    assert body["count"] >= 1


@pytest.mark.asyncio
async def test_read_bundle_metadata_resource(
    agent_native_dispatcher, bundle_seeded,
):
    res = await agent_native_dispatcher.read_resource(
        "efe://bundles/resource-test/v1.0.0"
    )
    assert "uri" in res
    body = json.loads(res["text"])
    assert body["slug"] == "resource-test"
    assert body["version"] == "v1.0.0"
    assert "manifest" in body
    assert body["manifest"]["total_layer_size"] == 1_000_000


@pytest.mark.asyncio
async def test_read_bundle_sbom_resource_lists_components(
    agent_native_dispatcher, bundle_seeded,
):
    res = await agent_native_dispatcher.read_resource(
        "efe://bundles/resource-test/v1.0.0/sbom"
    )
    body = json.loads(res["text"])
    assert body["sbom_present"] is True
    names = {c["name"] for c in body["components"]}
    assert "torch" in names


@pytest.mark.asyncio
async def test_read_bundle_signature_resource_verified(
    agent_native_dispatcher, bundle_seeded,
):
    res = await agent_native_dispatcher.read_resource(
        "efe://bundles/resource-test/v1.0.0/signature"
    )
    body = json.loads(res["text"])
    assert body["is_signed"] is True
    assert body["verified"] is True
    assert body["slsa_provenance"]["present"] is True
    assert body["slsa_provenance"]["subject_digest_match"] is True


@pytest.mark.asyncio
async def test_read_bundle_resource_unknown_returns_error(
    agent_native_dispatcher, monkeypatch,
):
    monkeypatch.setenv("EFE_OCI_DEFAULT_REGISTRY", "ghcr.io/efe-project/bundles")
    res = await agent_native_dispatcher.read_resource(
        "efe://bundles/nonexistent/v1.0.0"
    )
    assert "error" in res
    assert res["code"] in ("invalid_resource_uri", "not_found")


@pytest.mark.asyncio
async def test_read_bundle_resources_emit_audit_events(
    agent_native_dispatcher, bundle_seeded, audit_emitter,
):
    """M17-A5 carry-over : every resources/read emits a
    `mcp_resource_read` event."""
    audit_emitter.enabled = False  # local in-memory ring suffices for this test
    before = len(audit_emitter.recent(limit=500))
    await agent_native_dispatcher.read_resource(
        "efe://bundles/resource-test/v1.0.0"
    )
    after = len(audit_emitter.recent(limit=500))
    assert after == before + 1
    last = audit_emitter.recent(limit=1)[0]
    assert last.tool == "mcp_resource_read"
    assert last.ok is True


@pytest.mark.asyncio
async def test_read_bundle_traversal_rejected_via_dispatcher(
    agent_native_dispatcher,
):
    """The dispatcher's read_resource path must reject path traversal
    via the URI parser."""
    res = await agent_native_dispatcher.read_resource(
        "efe://bundles/../etc/passwd"
    )
    assert "error" in res
    assert res["code"] == "invalid_resource_uri"
