"""efe_diff_bundles pin tests (M18 D47)."""
from __future__ import annotations

import json

import pytest


def _sbom_bytes(components: list[dict], vulns: list[dict] | None = None) -> bytes:
    return json.dumps({
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "components": components,
        "vulnerabilities": vulns or [],
    }).encode("utf-8")


@pytest.fixture
def two_bundles(fake_oci_registry, monkeypatch):
    monkeypatch.setenv("EFE_OCI_DEFAULT_REGISTRY", "ghcr.io/efe-project/bundles")
    sbom_v1 = _sbom_bytes(
        components=[
            {"name": "torch", "version": "2.0.1", "type": "library"},
            {"name": "old-lib", "version": "1.0.0", "type": "library"},
        ],
        vulns=[
            {
                "id": "CVE-2024-XXXX",
                "ratings": [{"severity": "MEDIUM"}],
                "affects": [{"ref": "pkg:pypi/old-lib@1.0.0"}],
            }
        ],
    )
    sbom_v2 = _sbom_bytes(
        components=[
            {"name": "torch", "version": "2.1.0", "type": "library"},
            {"name": "transformers", "version": "4.35.0", "type": "library"},
        ],
        vulns=[],
    )
    fake_oci_registry.add_bundle(
        repo="efe-project/bundles", slug="x", version="v1.0.0",
        annotations={
            "org.efe.bundle.framework": "sentence-transformers",
            "org.efe.bundle.dim": "384",
        },
        sbom_bytes=sbom_v1,
    )
    fake_oci_registry.add_bundle(
        repo="efe-project/bundles", slug="x", version="v2.0.0",
        annotations={
            "org.efe.bundle.framework": "sentence-transformers",
            "org.efe.bundle.dim": "384",
        },
        sbom_bytes=sbom_v2,
    )


@pytest.mark.asyncio
async def test_diff_returns_components_added_removed_upgraded(
    agent_native_dispatcher, two_bundles,
):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_diff_bundles",
        args={
            "slug_a": "x", "version_a": "v1.0.0",
            "slug_b": "x", "version_b": "v2.0.0",
        },
    )
    assert res.is_error is False, res.content
    diff = res.content["sbom_diff"]
    added_names = {c.get("name") for c in diff["components_added"]}
    removed_names = {c.get("name") for c in diff["components_removed"]}
    upgraded_names = {c.get("name") for c in diff["components_upgraded"]}
    assert "transformers" in added_names
    assert "old-lib" in removed_names
    assert "torch" in upgraded_names


@pytest.mark.asyncio
async def test_diff_detects_resolved_vulnerabilities(
    agent_native_dispatcher, two_bundles,
):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_diff_bundles",
        args={
            "slug_a": "x", "version_a": "v1.0.0",
            "slug_b": "x", "version_b": "v2.0.0",
        },
    )
    diff = res.content["sbom_diff"]
    resolved_ids = {v.get("cve") for v in diff["vulnerabilities_resolved"]}
    assert "CVE-2024-XXXX" in resolved_ids


@pytest.mark.asyncio
async def test_diff_detects_no_breaking_when_dim_unchanged(
    agent_native_dispatcher, two_bundles,
):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_diff_bundles",
        args={
            "slug_a": "x", "version_a": "v1.0.0",
            "slug_b": "x", "version_b": "v2.0.0",
        },
    )
    assert res.content["compatibility_breaking"] is False


@pytest.mark.asyncio
async def test_diff_detects_breaking_when_dim_changes(
    agent_native_dispatcher, fake_oci_registry, monkeypatch,
):
    monkeypatch.setenv("EFE_OCI_DEFAULT_REGISTRY", "ghcr.io/efe-project/bundles")
    sbom = _sbom_bytes([])
    fake_oci_registry.add_bundle(
        repo="efe-project/bundles", slug="y", version="v1.0.0",
        annotations={"org.efe.bundle.dim": "384"}, sbom_bytes=sbom,
    )
    fake_oci_registry.add_bundle(
        repo="efe-project/bundles", slug="y", version="v2.0.0",
        annotations={"org.efe.bundle.dim": "768"}, sbom_bytes=sbom,
    )
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_diff_bundles",
        args={
            "slug_a": "y", "version_a": "v1.0.0",
            "slug_b": "y", "version_b": "v2.0.0",
        },
    )
    assert res.content["compatibility_breaking"] is True
    assert res.content["manifest_diff"]["dim_changed"] is True


@pytest.mark.asyncio
async def test_diff_recommendation_safe_for_resolved_cve(
    agent_native_dispatcher, two_bundles,
):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_diff_bundles",
        args={
            "slug_a": "x", "version_a": "v1.0.0",
            "slug_b": "x", "version_b": "v2.0.0",
        },
    )
    rec = res.content["recommendation"]
    assert "Safe upgrade" in rec
    assert "1 CVE" in rec or "resolved" in rec.lower()


@pytest.mark.asyncio
async def test_diff_zero_changes_when_same_bundle(
    agent_native_dispatcher, two_bundles,
):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_diff_bundles",
        args={
            "slug_a": "x", "version_a": "v1.0.0",
            "slug_b": "x", "version_b": "v1.0.0",
        },
    )
    diff = res.content["sbom_diff"]
    assert diff["components_added"] == []
    assert diff["components_removed"] == []
    assert diff["components_upgraded"] == []
