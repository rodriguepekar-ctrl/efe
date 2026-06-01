"""efe_get_sbom tool pin tests (M18 D48)."""
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
def sbom_seeded(fake_oci_registry, monkeypatch):
    monkeypatch.setenv("EFE_OCI_DEFAULT_REGISTRY", "ghcr.io/efe-project/bundles")
    sbom = _sbom_bytes(
        components=[
            {"name": "torch", "version": "2.1.0", "type": "library", "purl": "pkg:pypi/torch@2.1.0"},
            {"name": "transformers", "version": "4.35.0", "type": "library"},
            {"name": "numpy", "version": "1.26.0", "type": "library"},
        ],
        vulns=[
            {
                "id": "CVE-2024-12345",
                "ratings": [{"severity": "HIGH"}],
                "affects": [{"ref": "pkg:pypi/torch@2.1.0"}],
            },
            {
                "id": "CVE-2024-67890",
                "ratings": [{"severity": "LOW"}],
                "affects": [{"ref": "pkg:pypi/numpy@1.26.0"}],
            },
        ],
    )
    fake_oci_registry.add_bundle(
        repo="efe-project/bundles", slug="sbom-test", version="v1.0.0",
        sbom_bytes=sbom,
    )


@pytest.mark.asyncio
async def test_get_sbom_returns_components(agent_native_dispatcher, sbom_seeded):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_get_sbom",
        args={"slug": "sbom-test", "version": "v1.0.0"},
    )
    assert res.is_error is False, res.content
    names = {c["name"] for c in res.content["components"]}
    assert names == {"torch", "transformers", "numpy"}


@pytest.mark.asyncio
async def test_get_sbom_returns_vulnerabilities(agent_native_dispatcher, sbom_seeded):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_get_sbom",
        args={"slug": "sbom-test", "version": "v1.0.0"},
    )
    ids = {v["id"] for v in res.content["vulnerabilities"]}
    assert "CVE-2024-12345" in ids
    assert "CVE-2024-67890" in ids


@pytest.mark.asyncio
async def test_get_sbom_filter_severity_high_excludes_low(
    agent_native_dispatcher, sbom_seeded,
):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_get_sbom",
        args={
            "slug": "sbom-test", "version": "v1.0.0",
            "filter_severity": "HIGH",
        },
    )
    severities = {v["severity"] for v in res.content["vulnerabilities"]}
    # LOW must be filtered out, HIGH kept
    assert "HIGH" in severities
    assert "LOW" not in severities


@pytest.mark.asyncio
async def test_get_sbom_max_components_truncates(
    agent_native_dispatcher, sbom_seeded,
):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_get_sbom",
        args={"slug": "sbom-test", "version": "v1.0.0", "max_components": 2},
    )
    assert len(res.content["components"]) == 2
    assert res.content["components_truncated"] is True
    assert res.content["components_total"] == 3


@pytest.mark.asyncio
async def test_get_sbom_summary_highest_severity_critical_first(
    agent_native_dispatcher, fake_oci_registry, monkeypatch,
):
    monkeypatch.setenv("EFE_OCI_DEFAULT_REGISTRY", "ghcr.io/efe-project/bundles")
    sbom = _sbom_bytes(
        components=[{"name": "x", "version": "1", "type": "library"}],
        vulns=[
            {"id": "CVE-1", "ratings": [{"severity": "CRITICAL"}], "affects": []},
            {"id": "CVE-2", "ratings": [{"severity": "MEDIUM"}], "affects": []},
        ],
    )
    fake_oci_registry.add_bundle(
        repo="efe-project/bundles", slug="crit", version="v1.0.0", sbom_bytes=sbom,
    )
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_get_sbom",
        args={"slug": "crit", "version": "v1.0.0"},
    )
    assert res.content["summary"]["highest_severity"] == "CRITICAL"


@pytest.mark.asyncio
async def test_get_sbom_no_sbom_returns_warning(
    agent_native_dispatcher, fake_oci_registry, monkeypatch,
):
    monkeypatch.setenv("EFE_OCI_DEFAULT_REGISTRY", "ghcr.io/efe-project/bundles")
    fake_oci_registry.add_bundle(
        repo="efe-project/bundles", slug="no-sbom", version="v1.0.0",
    )
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_get_sbom",
        args={"slug": "no-sbom", "version": "v1.0.0"},
    )
    assert res.is_error is False
    assert res.content["sbom_format"] is None
    assert "no SBOM" in res.content["warnings"][0]


@pytest.mark.asyncio
async def test_get_sbom_unknown_bundle_returns_not_found(
    agent_native_dispatcher, monkeypatch,
):
    monkeypatch.setenv("EFE_OCI_DEFAULT_REGISTRY", "ghcr.io/efe-project/bundles")
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_get_sbom",
        args={"slug": "missing", "version": "v9.9.9"},
    )
    assert res.is_error is True
    assert res.content["code"] == "not_found"


@pytest.mark.asyncio
async def test_get_sbom_components_with_vulns_count(
    agent_native_dispatcher, sbom_seeded,
):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_get_sbom",
        args={"slug": "sbom-test", "version": "v1.0.0"},
    )
    # 2 vulnerabilities affecting 2 distinct components
    assert res.content["summary"]["components_with_vulns"] == 2


@pytest.mark.asyncio
async def test_get_sbom_max_components_schema_max_5000(agent_native_dispatcher):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_get_sbom",
        args={"slug": "x", "version": "v1.0.0", "max_components": 10000},
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"
