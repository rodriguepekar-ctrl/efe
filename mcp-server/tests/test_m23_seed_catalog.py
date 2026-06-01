"""Tests for `efe_seed_catalog` (D8)."""
from __future__ import annotations

from typing import Any

import pytest

from mcp_server.tools import m23_seed_catalog
from mcp_server.tools.types import ToolContext, ToolError


class _FakeAuth:
    pass


class _FakeBackend:
    def __init__(self) -> None:
        self.calls: list[dict[str, Any]] = []
        self.raises_conflict = False

    class ConflictError(Exception):
        pass

    async def seed_catalog(self, *, tenant_id, catalog, bundle_version, anchors, auth):
        self.calls.append(dict(
            tenant_id=tenant_id, catalog=catalog,
            bundle_version=bundle_version, n_anchors=len(anchors),
        ))
        if self.raises_conflict:
            raise self.ConflictError("dup")
        return {"anchors_seeded": len(anchors), "duration_ms": 1234}


def _ctx(backend):
    return ToolContext(auth=_FakeAuth(), backend=backend)


def _good_anchors(n: int = 2) -> list[dict[str, str]]:
    return [
        {"anchor_id": f"a{i:03}", "text": f"text {i}", "label": "billing"}
        for i in range(n)
    ]


@pytest.mark.asyncio
async def test_happy_path_returns_count():
    be = _FakeBackend()
    out = await m23_seed_catalog.handler(
        {
            "tenant_id": "acme",
            "bundle_version": "v1.0.0",
            "anchors": _good_anchors(50),
            "confirm": True,
        },
        _ctx(be),
    )
    assert out["anchors_seeded"] == 50
    assert out["duration_ms"] == 1234
    assert be.calls[0]["n_anchors"] == 50


@pytest.mark.asyncio
async def test_duplicate_anchor_id_rejected():
    be = _FakeBackend()
    anchors = [
        {"anchor_id": "x", "text": "a", "label": "billing"},
        {"anchor_id": "x", "text": "b", "label": "billing"},  # dup
    ]
    with pytest.raises(ToolError) as exc:
        await m23_seed_catalog.handler(
            {
                "tenant_id": "acme",
                "bundle_version": "v1.0.0",
                "anchors": anchors,
                "confirm": True,
            },
            _ctx(be),
        )
    assert exc.value.code == "invalid_input"
    assert "duplicate" in str(exc.value).lower()


@pytest.mark.asyncio
async def test_conflict_status_409():
    be = _FakeBackend()
    be.raises_conflict = True
    with pytest.raises(ToolError) as exc:
        await m23_seed_catalog.handler(
            {
                "tenant_id": "acme",
                "bundle_version": "v1.0.0",
                "anchors": _good_anchors(),
                "confirm": True,
            },
            _ctx(be),
        )
    assert exc.value.code == "conflict"
    assert exc.value.status_code == 409


@pytest.mark.asyncio
async def test_missing_confirm_rejected():
    be = _FakeBackend()
    with pytest.raises(ToolError) as exc:
        await m23_seed_catalog.handler(
            {
                "tenant_id": "acme",
                "bundle_version": "v1.0.0",
                "anchors": _good_anchors(),
            },
            _ctx(be),
        )
    assert exc.value.code == "confirm_required"


def test_schema_caps_max_anchors():
    cap = m23_seed_catalog.INPUT_SCHEMA["properties"]["anchors"]["maxItems"]
    assert cap == 50_000
