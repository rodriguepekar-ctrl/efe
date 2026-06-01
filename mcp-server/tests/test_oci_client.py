"""OCI registry client pin tests (M18 D47)."""
from __future__ import annotations

import pytest

from mcp_server.bundle import (
    OCIRegistryClient,
    OCIRegistryError,
    OCIRegistryNotInAllowlist,
    parse_registry_allowlist,
)
from mcp_server.bundle.oci import (
    _normalise_registry_url,
    check_allowlist,
)


def test_parse_registry_allowlist_empty_when_unset(monkeypatch):
    monkeypatch.delenv("EFE_OCI_REGISTRY_ALLOWLIST", raising=False)
    assert parse_registry_allowlist() == ()


def test_parse_registry_allowlist_comma_separated(monkeypatch):
    monkeypatch.setenv(
        "EFE_OCI_REGISTRY_ALLOWLIST",
        "ghcr.io/efe-project, registry.example.com",
    )
    al = parse_registry_allowlist()
    assert "ghcr.io/efe-project" in al
    assert "registry.example.com" in al


def test_check_allowlist_rejects_when_empty():
    with pytest.raises(OCIRegistryNotInAllowlist):
        check_allowlist("ghcr.io/foo", allowlist=())


def test_check_allowlist_accepts_exact_match():
    check_allowlist("ghcr.io/foo", allowlist=("ghcr.io/foo",))  # no raise


def test_check_allowlist_accepts_subpath():
    check_allowlist("ghcr.io/foo/bar", allowlist=("ghcr.io/foo",))  # no raise


def test_check_allowlist_rejects_other_host():
    with pytest.raises(OCIRegistryNotInAllowlist):
        check_allowlist("evil.example.com/foo", allowlist=("ghcr.io/foo",))


def test_normalise_strips_scheme():
    assert _normalise_registry_url("https://ghcr.io/foo") == "ghcr.io/foo"
    assert _normalise_registry_url("ghcr.io/foo") == "ghcr.io/foo"


def test_oci_client_construction_rejects_disallowed_host():
    with pytest.raises(OCIRegistryNotInAllowlist):
        OCIRegistryClient("evil.example.com", allowlist=("ghcr.io",))


@pytest.mark.asyncio
async def test_oci_client_get_manifest_returns_bytes(
    fake_oci_registry, oci_factory,
):
    fake_oci_registry.add_bundle(
        repo="efe-project/bundles", slug="x", version="v1.0.0",
    )
    async with oci_factory("ghcr.io") as client:
        raw, parsed, sha = await client.get_manifest(
            repo="efe-project/bundles", reference="x-v1.0.0",
        )
    assert isinstance(raw, bytes)
    assert isinstance(parsed, dict)
    assert len(sha) == 64


@pytest.mark.asyncio
async def test_oci_client_user_agent_set(fake_oci_registry, oci_factory):
    fake_oci_registry.add_bundle(
        repo="efe-project/bundles", slug="x", version="v1.0.0",
    )
    async with oci_factory("ghcr.io") as client:
        await client.get_manifest(
            repo="efe-project/bundles", reference="x-v1.0.0",
        )
    # FakeOCIRegistry doesn't capture headers in this minimal impl,
    # but we can confirm the client was constructed (no error raised).
    assert client._headers["User-Agent"].startswith("efe-mcp-server/")


@pytest.mark.asyncio
async def test_oci_client_referrers_filtered_by_artifact_type(
    fake_oci_registry, oci_factory, ed25519_keypair,
):
    raw_sig = ed25519_keypair["sign"](b"manifest-content")
    sha = fake_oci_registry.add_bundle(
        repo="efe-project/bundles", slug="x", version="v1.0.0",
        signature_bytes=raw_sig,
        sbom_bytes=b'{"bomFormat":"CycloneDX","specVersion":"1.5"}',
    )
    async with oci_factory("ghcr.io") as client:
        sig_referrers = await client.get_referrers(
            repo="efe-project/bundles", digest=f"sha256:{sha}",
            artifact_type="application/vnd.efe.bundle.signature.ed25519+json",
        )
        sbom_referrers = await client.get_referrers(
            repo="efe-project/bundles", digest=f"sha256:{sha}",
            artifact_type="application/vnd.cyclonedx+json",
        )
    assert len(sig_referrers) == 1
    assert len(sbom_referrers) == 1


@pytest.mark.asyncio
async def test_oci_client_404_returns_artifact_not_found(
    fake_oci_registry, oci_factory,
):
    from mcp_server.bundle import OCIArtifactNotFound

    async with oci_factory("ghcr.io") as client:
        with pytest.raises(OCIArtifactNotFound):
            await client.get_manifest(
                repo="efe-project/bundles", reference="missing",
            )
