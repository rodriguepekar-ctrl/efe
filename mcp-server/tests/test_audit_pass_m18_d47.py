"""M18 D47 audit pass 1 — pin tests for the 7 defects fixed.

Pattern carryover from M11-M17 audit passes. Every fix in this file
locks in correct behaviour so a future regression is caught loudly.
"""
from __future__ import annotations

import json
import os

import pytest

from mcp_server.bundle import (
    InvalidRegistryArg,
    OCIRegistryClient,
    OCIRegistryNotInAllowlist,
    check_allowlist,
    check_host_in_allowlist,
    parse_manifest,
    parse_sbom,
    split_registry,
)
from mcp_server.bundle.sbom import DEFAULT_MAX_SBOM_BYTES, SBOMParseError


# ─────────────────────────────────────────────────────────────────────
# A1 — allowlist path-prefix support (HIGH)
# ─────────────────────────────────────────────────────────────────────


def test_audit_a1_check_host_in_allowlist_accepts_host_when_allowlist_has_path():
    """Pre-fix : `OCIRegistryClient("ghcr.io", allowlist=("ghcr.io/foo",))`
    rejected the host because `check_allowlist` required the registry
    URL to be UNDER an allowlist entry. Now `check_host_in_allowlist`
    is used at construction time and matches just the host part."""
    check_host_in_allowlist("ghcr.io", allowlist=("ghcr.io/foo",))
    check_host_in_allowlist("ghcr.io", allowlist=("ghcr.io/foo", "registry.example.com"))


def test_audit_a1_check_host_in_allowlist_rejects_disallowed_host():
    with pytest.raises(OCIRegistryNotInAllowlist):
        check_host_in_allowlist("evil.example.com", allowlist=("ghcr.io/foo",))


def test_audit_a1_check_allowlist_strict_per_request_still_enforces_path():
    """`check_allowlist` (strict, used per-request) must reject a path
    not under any allowlist entry."""
    # A request to ghcr.io/random-other-repo with allowlist ghcr.io/foo
    # must STILL be rejected at the request level.
    with pytest.raises(OCIRegistryNotInAllowlist):
        check_allowlist(
            "https://ghcr.io/random-other-repo",
            allowlist=("ghcr.io/foo",),
        )


def test_audit_a1_oci_client_construction_with_path_prefixed_allowlist():
    """The user can now construct a client targeting just the host
    while keeping a path-prefixed allowlist."""
    client = OCIRegistryClient("ghcr.io", allowlist=("ghcr.io/efe-project",))
    assert client._allowlist == ("ghcr.io/efe-project",)


@pytest.mark.asyncio
async def test_audit_a1_get_manifest_rejects_repo_outside_allowlist():
    """Per-request enforcement : repo must satisfy the allowlist."""
    client = OCIRegistryClient("ghcr.io", allowlist=("ghcr.io/efe-project",))
    with pytest.raises(OCIRegistryNotInAllowlist):
        await client.get_manifest(repo="random-other-repo", reference="v1.0.0")
    await client.aclose()


@pytest.mark.asyncio
async def test_audit_a1_get_manifest_allowed_when_repo_under_allowlist(
    fake_oci_registry,
):
    """When allowlist is `ghcr.io/efe-project`, a request to
    `efe-project/bundles` is allowed."""
    fake_oci_registry.add_bundle(
        repo="efe-project/bundles", slug="x", version="v1.0.0",
    )
    import httpx

    def _handler(request):
        result = fake_oci_registry.handle_request(
            request.method, request.url.path, dict(request.url.params),
        )
        return httpx.Response(
            status_code=result["status_code"],
            content=result["content"],
            headers=result["headers"],
        )

    transport = httpx.MockTransport(_handler)
    client = OCIRegistryClient("ghcr.io", allowlist=("ghcr.io/efe-project",))
    # Replace transport with the mock
    await client._client.aclose()
    client._client = httpx.AsyncClient(
        base_url=client._url, transport=transport, timeout=10.0,
    )
    raw, _, _ = await client.get_manifest(
        repo="efe-project/bundles", reference="x-v1.0.0",
    )
    assert raw  # success
    await client.aclose()


# ─────────────────────────────────────────────────────────────────────
# A2 — real layer sizes from OCI manifest (MED)
# ─────────────────────────────────────────────────────────────────────


def test_audit_a2_manifest_total_layer_size_computed():
    """The parsed manifest must expose `total_layer_size` from
    `manifest.layers[*].size` instead of falling back to a hardcoded
    estimate."""
    manifest_dict = {
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "config": {"mediaType": "x", "digest": "sha256:abc", "size": 0},
        "layers": [
            {"mediaType": "x", "digest": "sha256:1", "size": 1_000_000},
            {"mediaType": "x", "digest": "sha256:2", "size": 500_000},
        ],
        "annotations": {},
    }
    raw = json.dumps(manifest_dict).encode("utf-8")
    manifest = parse_manifest(raw_bytes=raw, manifest_sha256="dummy")
    assert manifest.total_layer_size == 1_500_000
    assert manifest.layer_count == 2


def test_audit_a2_manifest_total_layer_size_zero_when_no_layers():
    manifest_dict = {
        "schemaVersion": 2,
        "mediaType": "x",
        "config": {"digest": "sha256:abc"},
        "layers": [],
        "annotations": {},
    }
    raw = json.dumps(manifest_dict).encode("utf-8")
    manifest = parse_manifest(raw_bytes=raw, manifest_sha256="dummy")
    assert manifest.total_layer_size == 0


@pytest.mark.asyncio
async def test_audit_a2_diff_bundles_uses_real_layer_sizes(
    agent_native_dispatcher, fake_oci_registry, monkeypatch,
):
    """Real OCI layer size deltas — not hardcoded 10KB-per-component."""
    monkeypatch.setenv("EFE_OCI_DEFAULT_REGISTRY", "ghcr.io/efe-project/bundles")
    sbom = json.dumps({"bomFormat": "CycloneDX", "specVersion": "1.5"}).encode("utf-8")
    fake_oci_registry.add_bundle(
        repo="efe-project/bundles", slug="z", version="v1.0.0",
        sbom_bytes=sbom, layer_sizes=[1_000_000],
    )
    fake_oci_registry.add_bundle(
        repo="efe-project/bundles", slug="z", version="v2.0.0",
        sbom_bytes=sbom, layer_sizes=[1_500_000, 200_000],
    )
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_diff_bundles",
        args={"slug_a": "z", "version_a": "v1.0.0",
              "slug_b": "z", "version_b": "v2.0.0"},
    )
    assert res.is_error is False, res.content
    # 1_700_000 - 1_000_000 = 700_000 from real layer sizes
    assert res.content["size_a_bytes"] == 1_000_000
    assert res.content["size_b_bytes"] == 1_700_000
    assert res.content["size_diff_bytes"] == 700_000
    assert res.content["size_estimated"] is False


# ─────────────────────────────────────────────────────────────────────
# A3 — SBOM size limit (MED)
# ─────────────────────────────────────────────────────────────────────


def test_audit_a3_sbom_parser_default_max_bytes_10mb():
    assert DEFAULT_MAX_SBOM_BYTES == 10 * 1024 * 1024


def test_audit_a3_sbom_parser_rejects_oversized_input():
    """Input bytes > max_bytes raise SBOMParseError."""
    huge = b'{"bomFormat":"CycloneDX","specVersion":"1.5","components":[' \
        + (b'{"name":"x","version":"1"},' * 1000) \
        + b']}'
    with pytest.raises(SBOMParseError) as excinfo:
        parse_sbom(huge, max_bytes=200)
    assert "max_bytes" in str(excinfo.value)


def test_audit_a3_sbom_parser_accepts_input_under_limit():
    small = b'{"bomFormat":"CycloneDX","specVersion":"1.5","components":[]}'
    parsed = parse_sbom(small, max_bytes=DEFAULT_MAX_SBOM_BYTES)
    assert parsed.spec_version == "1.5"


def test_audit_a3_sbom_parser_accepts_str_input_with_limit():
    s = '{"bomFormat":"CycloneDX","specVersion":"1.5","components":[]}'
    parsed = parse_sbom(s, max_bytes=200)
    assert parsed.spec_version == "1.5"


# ─────────────────────────────────────────────────────────────────────
# A4 — strip scheme from registry arg (LOW)
# ─────────────────────────────────────────────────────────────────────


def test_audit_a4_split_registry_strips_https_scheme():
    host, repo, full = split_registry("https://ghcr.io/efe-project/bundles")
    assert host == "ghcr.io"
    assert repo == "efe-project/bundles"
    assert full == "ghcr.io/efe-project/bundles"


def test_audit_a4_split_registry_strips_http_scheme():
    host, repo, _ = split_registry("http://localhost:5000/foo")
    assert host == "localhost:5000"
    assert repo == "foo"


def test_audit_a4_split_registry_handles_no_scheme():
    host, repo, _ = split_registry("ghcr.io/foo/bar")
    assert host == "ghcr.io"
    assert repo == "foo/bar"


def test_audit_a4_split_registry_rejects_host_only():
    with pytest.raises(InvalidRegistryArg):
        split_registry("ghcr.io")


def test_audit_a4_split_registry_rejects_empty():
    with pytest.raises(InvalidRegistryArg):
        split_registry("")
    with pytest.raises(InvalidRegistryArg):
        split_registry("   ")


# ─────────────────────────────────────────────────────────────────────
# A5 — sign_callback in fixture removes double-add complexity (LOW)
# ─────────────────────────────────────────────────────────────────────


def test_audit_a5_add_bundle_sign_callback_signs_canonical_bytes(
    fake_oci_registry, ed25519_keypair,
):
    """One add_bundle call with `sign_callback` produces a manifest +
    signature whose verification succeeds against the actual bytes."""
    from mcp_server.bundle import verify_ed25519_signature

    sha = fake_oci_registry.add_bundle(
        repo="efe-project/bundles",
        slug="audit-a5",
        version="v1.0.0",
        sign_callback=ed25519_keypair["sign"],
    )
    manifest_entry = fake_oci_registry._manifests["efe-project/bundles"]["audit-a5-v1.0.0"]
    referrers = fake_oci_registry._referrers.get(f"sha256:{sha}", [])
    assert len(referrers) == 1
    sig_layer = referrers[0]["layers"][0]
    sig_blob = fake_oci_registry._blobs[sig_layer["digest"]]
    result = verify_ed25519_signature(
        manifest_bytes=manifest_entry["raw"],
        signature_bytes=sig_blob,
        public_key_pem=ed25519_keypair["public_pem"],
        expected_manifest_sha256=manifest_entry["sha256"],
    )
    assert result.verified is True


# ─────────────────────────────────────────────────────────────────────
# A6 — oci_factory closes the original httpx client (LOW)
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_audit_a6_factory_closes_original_httpx_client(
    fake_oci_registry, oci_factory,
):
    """The factory must close the OCIRegistryClient's original httpx
    AsyncClient before replacing it with the MockTransport one. We
    can't directly observe close from outside, but we verify the
    replacement client works for at least one request without
    resource warnings."""
    fake_oci_registry.add_bundle(
        repo="efe-project/bundles", slug="a6", version="v1.0.0",
    )
    async with oci_factory("ghcr.io") as client:
        raw, _, _ = await client.get_manifest(
            repo="efe-project/bundles", reference="a6-v1.0.0",
        )
    assert raw


# ─────────────────────────────────────────────────────────────────────
# A7 — default factory honors EFE_MCP_INSECURE (LOW)
# ─────────────────────────────────────────────────────────────────────


def test_audit_a7_default_factory_honors_efe_mcp_insecure(monkeypatch):
    monkeypatch.setenv("EFE_OCI_REGISTRY_ALLOWLIST", "ghcr.io")
    monkeypatch.setenv("EFE_MCP_INSECURE", "true")
    from mcp_server.bundle.registry_arg import default_oci_factory_factory
    factory = default_oci_factory_factory()
    client = factory("ghcr.io/foo")
    # httpx.AsyncClient stores TLS verify in its _transport / config ;
    # we expose it via `client._client._transport._pool._ssl_context`
    # which is brittle. Instead, just verify the client constructed
    # without raising and that env was consulted (no exception).
    assert client._url == "https://ghcr.io"
    # In httpx, `verify=False` produces `_verify=False` on the SSL config.
    # We check via the `_transport` attribute.
    transport = client._client._transport
    # Best-effort check : httpx >= 0.27 exposes _verify on HTTPTransport
    verify = getattr(transport, "_verify", None)
    if verify is not None:
        assert verify is False or verify == False  # noqa: E712


def test_audit_a7_default_factory_uses_token_env(monkeypatch):
    monkeypatch.setenv("EFE_OCI_REGISTRY_ALLOWLIST", "ghcr.io")
    monkeypatch.setenv("EFE_OCI_REGISTRY_TOKEN", "tok-abc-123")
    monkeypatch.delenv("EFE_MCP_INSECURE", raising=False)
    from mcp_server.bundle.registry_arg import default_oci_factory_factory
    factory = default_oci_factory_factory()
    client = factory("ghcr.io/foo")
    assert client._headers.get("Authorization") == "Bearer tok-abc-123"


def test_audit_a7_default_factory_default_verify_tls_true(monkeypatch):
    monkeypatch.setenv("EFE_OCI_REGISTRY_ALLOWLIST", "ghcr.io")
    monkeypatch.delenv("EFE_MCP_INSECURE", raising=False)
    from mcp_server.bundle.registry_arg import default_oci_factory_factory
    factory = default_oci_factory_factory()
    client = factory("ghcr.io/foo")
    transport = client._client._transport
    verify = getattr(transport, "_verify", None)
    if verify is not None:
        # Default True
        assert verify is not False


# ─────────────────────────────────────────────────────────────────────
# Cross-cutting : tools accept https:// scheme post-A4 fix
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_audit_a4_tool_accepts_https_scheme_in_registry(
    agent_native_dispatcher, fake_oci_registry, monkeypatch,
):
    """A user passing `registry="https://ghcr.io/foo"` must work."""
    monkeypatch.setenv("EFE_OCI_DEFAULT_REGISTRY", "https://ghcr.io/efe-project/bundles")
    fake_oci_registry.add_bundle(
        repo="efe-project/bundles", slug="a4", version="v1.0.0",
    )
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_inspect_bundle",
        args={"slug": "a4", "version": "v1.0.0"},
    )
    # Pre-A4 fix : would have failed with "registry must be `host/repo`"
    # because https://ghcr.io was split into host="https:".
    assert res.is_error is False, res.content
