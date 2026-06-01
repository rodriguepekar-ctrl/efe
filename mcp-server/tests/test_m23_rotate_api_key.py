"""Tests for `efe_rotate_api_key` (D8)."""
from __future__ import annotations

import pytest

from mcp_server.tools import m23_rotate_api_key
from mcp_server.tools.types import ToolContext, ToolError


class _FakeAuth:
    pass


class _FakeBackend:
    def __init__(self) -> None:
        self.raises_not_found = False

    class NotFoundError(Exception):
        pass

    async def rotate_api_key(self, *, tenant_id, key_id, auth):
        if self.raises_not_found:
            raise self.NotFoundError("not found")
        return {
            "new_key_id": "11111111-2222-3333-4444-555555555555",
            "new_plaintext": "efe_" + "b" * 64,
            "scope": "classify",
            "expires_at": "2026-12-08T00:00:00Z",
            "old_key_invalidated_at": "2026-05-09T00:00:00Z",
        }


def _ctx(b):
    return ToolContext(auth=_FakeAuth(), backend=b)


@pytest.mark.asyncio
async def test_happy_path_returns_new_plaintext():
    be = _FakeBackend()
    out = await m23_rotate_api_key.handler(
        {
            "tenant_id": "acme",
            "key_id": "00000000-0000-0000-0000-000000000000",
            "confirm": True,
        },
        _ctx(be),
    )
    assert out["new_plaintext"].startswith("efe_")
    assert out["scope"] == "classify"
    assert out["old_key_id"] == "00000000-0000-0000-0000-000000000000"


@pytest.mark.asyncio
async def test_missing_confirm_rejected():
    be = _FakeBackend()
    with pytest.raises(ToolError) as exc:
        await m23_rotate_api_key.handler(
            {
                "tenant_id": "acme",
                "key_id": "00000000-0000-0000-0000-000000000000",
            },
            _ctx(be),
        )
    assert exc.value.code == "confirm_required"


@pytest.mark.asyncio
async def test_not_found_returns_404():
    be = _FakeBackend()
    be.raises_not_found = True
    with pytest.raises(ToolError) as exc:
        await m23_rotate_api_key.handler(
            {
                "tenant_id": "acme",
                "key_id": "ffffffff-ffff-ffff-ffff-ffffffffffff",
                "confirm": True,
            },
            _ctx(be),
        )
    assert exc.value.code == "not_found"
    assert exc.value.status_code == 404
