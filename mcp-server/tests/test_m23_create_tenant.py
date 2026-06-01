"""Tests for `efe_create_tenant` (D8)."""
from __future__ import annotations

from typing import Any

import pytest

from mcp_server.tools import m23_create_tenant
from mcp_server.tools.types import ToolError


class _FakeAuth:
    pass


class _FakeBackend:
    """Minimal in-memory fake : records calls, returns canned response.

    Set `raises_conflict=True` to simulate the idempotency 409 path.
    """

    def __init__(self) -> None:
        self.calls: list[dict[str, Any]] = []
        self.raises_conflict = False

    class ConflictError(Exception):
        pass

    async def create_tenant(self, *, name, plan, max_rps, max_tokens_per_day,
                             data_residency, contact_email, auth):
        self.calls.append(dict(
            name=name, plan=plan, max_rps=max_rps,
            max_tokens_per_day=max_tokens_per_day,
            data_residency=data_residency, contact_email=contact_email,
        ))
        if self.raises_conflict:
            raise self.ConflictError(f"tenant '{name}' exists")
        return {
            "tenant_id": name,
            "plan": plan,
            "max_rps": max_rps,
            "data_residency": data_residency,
            "created_at": "2026-05-08T00:00:00Z",
            "initial_admin_token": "efe_" + "a" * 64,
        }


def _ctx(backend: _FakeBackend):
    from mcp_server.tools.types import ToolContext
    return ToolContext(auth=_FakeAuth(), backend=backend)


@pytest.mark.asyncio
async def test_happy_path_returns_token():
    be = _FakeBackend()
    out = await m23_create_tenant.handler(
        {"name": "acme", "plan": "enterprise", "max_rps": 1000, "confirm": True},
        _ctx(be),
    )
    assert out["tenant_id"] == "acme"
    assert out["plan"] == "enterprise"
    assert out["initial_admin_token"].startswith("efe_")
    assert len(be.calls) == 1
    assert be.calls[0]["name"] == "acme"


@pytest.mark.asyncio
async def test_missing_confirm_rejected():
    be = _FakeBackend()
    with pytest.raises(ToolError) as exc:
        await m23_create_tenant.handler({"name": "acme"}, _ctx(be))
    assert exc.value.code == "confirm_required"
    assert be.calls == []


@pytest.mark.asyncio
async def test_confirm_false_rejected():
    be = _FakeBackend()
    with pytest.raises(ToolError) as exc:
        await m23_create_tenant.handler(
            {"name": "acme", "confirm": False}, _ctx(be)
        )
    assert exc.value.code == "confirm_required"


@pytest.mark.asyncio
async def test_conflict_returns_409_code():
    be = _FakeBackend()
    be.raises_conflict = True
    with pytest.raises(ToolError) as exc:
        await m23_create_tenant.handler(
            {"name": "acme", "confirm": True}, _ctx(be)
        )
    assert exc.value.code == "conflict"
    assert exc.value.status_code == 409


@pytest.mark.asyncio
async def test_invalid_plan_rejected():
    be = _FakeBackend()
    with pytest.raises(ToolError) as exc:
        await m23_create_tenant.handler(
            {"name": "acme", "plan": "platinum", "confirm": True}, _ctx(be)
        )
    assert exc.value.code == "invalid_input"


@pytest.mark.asyncio
async def test_invalid_residency_rejected():
    be = _FakeBackend()
    with pytest.raises(ToolError) as exc:
        await m23_create_tenant.handler(
            {"name": "acme", "data_residency": "MARS_ONLY", "confirm": True},
            _ctx(be),
        )
    assert exc.value.code == "invalid_input"


@pytest.mark.asyncio
async def test_default_residency_is_any():
    be = _FakeBackend()
    out = await m23_create_tenant.handler(
        {"name": "acme", "confirm": True}, _ctx(be)
    )
    assert out["data_residency"] == "any"


def test_schema_requires_name_and_confirm():
    s = m23_create_tenant.INPUT_SCHEMA
    assert "name" in s["required"]
    assert "confirm" in s["required"]
    assert s["additionalProperties"] is False


def test_schema_name_pattern():
    pat = m23_create_tenant.INPUT_SCHEMA["properties"]["name"]["pattern"]
    import re
    rx = re.compile(pat)
    assert rx.match("acme")
    assert rx.match("acme-corp")
    assert not rx.match("Acme")
    assert not rx.match("ab")           # too short
    assert not rx.match("-acme")        # leading hyphen
    assert not rx.match("acme-")        # trailing hyphen
