"""Tests for `efe_get_tenant_health` (D8, read-only)."""
from __future__ import annotations

import pytest

from mcp_server.tools import m23_get_tenant_health
from mcp_server.tools.types import ToolContext, ToolError


class _FakeAuth:
    pass


class _FakeBackend:
    def __init__(self) -> None:
        self.raises_not_found = False
        self.payload: dict | None = None

    class NotFoundError(Exception):
        pass

    async def get_tenant_health(self, *, tenant_id, auth):
        if self.raises_not_found:
            raise self.NotFoundError("nope")
        return self.payload or {
            "status": "healthy",
            "checks": {
                "db": {"status": "ok", "details": None, "last_check": "2026-05-08T00:00:00Z"},
                "redis": {"status": "ok", "details": None, "last_check": "2026-05-08T00:00:00Z"},
                "llm_breaker": {"status": "closed", "details": None, "last_check": "2026-05-08T00:00:00Z"},
                "audit_chain": {"status": "ok", "details": None, "last_check": "2026-05-08T00:00:00Z"},
            },
            "metrics": {
                "rps_last_5m": 120,
                "p99_ms_last_5m": 87,
                "error_rate_last_5m": 0.001,
            },
            "compliance": {
                "ai_act_art14_acknowledged": True,
                "gdpr_art22_acknowledged": True,
                "expires_in_days": 365,
            },
            "pending_reviews": 0,
            "cost_today_usd": 1.23,
            "daily_cost_cap_usd": 100.0,
            "last_classify_at": "2026-05-08T00:00:00Z",
        }


def _ctx(b):
    return ToolContext(auth=_FakeAuth(), backend=b)


@pytest.mark.asyncio
async def test_happy_path_returns_full_payload():
    be = _FakeBackend()
    out = await m23_get_tenant_health.handler(
        {"tenant_id": "acme"}, _ctx(be)
    )
    assert out["status"] == "healthy"
    assert out["checks"]["db"]["status"] == "ok"
    assert out["metrics"]["rps_last_5m"] == 120
    assert out["compliance"]["ai_act_art14_acknowledged"] is True
    assert out["pending_reviews"] == 0


@pytest.mark.asyncio
async def test_no_confirm_required():
    """Read tools never require confirm — empty args should work."""
    be = _FakeBackend()
    # No `confirm` in input — must succeed.
    out = await m23_get_tenant_health.handler(
        {"tenant_id": "acme"}, _ctx(be)
    )
    assert "status" in out


@pytest.mark.asyncio
async def test_not_found_returns_404():
    be = _FakeBackend()
    be.raises_not_found = True
    with pytest.raises(ToolError) as exc:
        await m23_get_tenant_health.handler(
            {"tenant_id": "missing"}, _ctx(be)
        )
    assert exc.value.code == "not_found"
    assert exc.value.status_code == 404


@pytest.mark.asyncio
async def test_partial_payload_does_not_raise():
    """Backend may trim fields ; handler must fill defaults."""
    be = _FakeBackend()
    be.payload = {"status": "degraded"}
    out = await m23_get_tenant_health.handler(
        {"tenant_id": "acme"}, _ctx(be)
    )
    assert out["status"] == "degraded"
    assert out["checks"]["db"]["status"] == "unknown"
    assert out["pending_reviews"] == 0
