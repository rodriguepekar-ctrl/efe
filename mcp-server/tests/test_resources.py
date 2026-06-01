"""MCP Resources pin tests (M17 D44)."""
from __future__ import annotations

import pytest

from mcp_server.resources.registry import (
    InvalidResourceUri,
    parse_resource_uri,
)


def test_resources_list_includes_m17_baseline(advanced_dispatcher):
    """M17 baseline (6 catalog/audit/quotas types) must remain.

    D48 adds 4 bundle resources (10 total now). Future milestones may
    add more — this test asserts the M17 baseline is preserved without
    hard-coding a specific count.
    """
    resources = advanced_dispatcher.list_resources()
    uris = {r["uri"] for r in resources}
    for required in (
        "efe://catalogs/{name}",
        "efe://audit/recent",
        "efe://quotas",
    ):
        assert required in uris, f"M17 resource {required!r} missing"
    assert len(resources) >= 6


def test_resources_list_disabled_in_core_profile(dispatcher):
    # `dispatcher` fixture uses default Profile.CORE
    assert dispatcher.list_resources() == []


def test_parse_catalog_summary_uri():
    p = parse_resource_uri("efe://catalogs/support-routing")
    assert p.kind == "catalog_summary"
    assert p.name == "support-routing"


def test_parse_catalog_version_uri():
    p = parse_resource_uri("efe://catalogs/x/versions/7")
    assert p.kind == "catalog_version"
    assert p.name == "x"
    assert p.version == 7


def test_parse_catalog_class_uri():
    p = parse_resource_uri("efe://catalogs/x/classes/billing")
    assert p.kind == "catalog_class"
    assert p.class_name == "billing"


def test_parse_audit_uri_with_query():
    p = parse_resource_uri("efe://audit/recent?limit=20")
    assert p.kind == "audit_recent"
    assert p.query == {"limit": "20"}


def test_parse_quotas_uri():
    p = parse_resource_uri("efe://quotas")
    assert p.kind == "quotas"


def test_parse_traversal_rejected():
    with pytest.raises(InvalidResourceUri):
        parse_resource_uri("efe://catalogs/../../etc/passwd")


def test_parse_unknown_path_rejected():
    with pytest.raises(InvalidResourceUri):
        parse_resource_uri("efe://random/thing")


def test_parse_invalid_scheme():
    with pytest.raises(InvalidResourceUri):
        parse_resource_uri("https://catalogs/x")


def test_parse_invalid_query_key():
    with pytest.raises(InvalidResourceUri):
        parse_resource_uri("efe://audit/recent?evil=1")


@pytest.mark.asyncio
async def test_read_resource_quotas(advanced_dispatcher, fake_backend):
    fake_backend._catalogs.clear()
    res = await advanced_dispatcher.read_resource("efe://quotas")
    assert "uri" in res
    assert "limits" in res["text"]


@pytest.mark.asyncio
async def test_read_resource_unknown_returns_error(advanced_dispatcher):
    res = await advanced_dispatcher.read_resource("efe://catalogs/missing")
    assert "error" in res
