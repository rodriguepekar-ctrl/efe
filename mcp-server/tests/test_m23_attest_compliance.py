"""Tests for `efe_attest_compliance` (D8)."""
from __future__ import annotations

from typing import Any

import pytest

from mcp_server.tools import m23_attest_compliance
from mcp_server.tools.types import ToolContext, ToolError


class _FakeAuth:
    pass


class _FakeBackend:
    def __init__(self) -> None:
        self.calls: list[dict[str, Any]] = []

    async def attest_compliance(self, *, tenant_id, ai_act_art14, gdpr_art22,
                                 signed_by, dpo_contact, auth):
        self.calls.append(dict(
            tenant_id=tenant_id, ai_act_art14=ai_act_art14,
            gdpr_art22=gdpr_art22, signed_by=signed_by, dpo_contact=dpo_contact,
        ))
        return {"attested_at": "2026-05-08T00:00:00Z"}


def _ctx(backend):
    return ToolContext(auth=_FakeAuth(), backend=backend)


@pytest.mark.asyncio
async def test_ai_act_only_happy():
    be = _FakeBackend()
    out = await m23_attest_compliance.handler(
        {
            "tenant_id": "acme",
            "ai_act_art14": True,
            "signed_by": "jane.doe@acme.com",
            "confirm": True,
        },
        _ctx(be),
    )
    assert out["ai_act_art14_acknowledged"] is True
    assert out["gdpr_art22_acknowledged"] is False
    assert be.calls[0]["ai_act_art14"] is True


@pytest.mark.asyncio
async def test_gdpr_requires_dpo_contact():
    be = _FakeBackend()
    with pytest.raises(ToolError) as exc:
        await m23_attest_compliance.handler(
            {
                "tenant_id": "acme",
                "gdpr_art22": True,
                "signed_by": "jane.doe@acme.com",
                "confirm": True,
            },
            _ctx(be),
        )
    assert exc.value.code == "invalid_input"
    assert "dpo_contact" in str(exc.value).lower()


@pytest.mark.asyncio
async def test_gdpr_with_dpo_happy():
    be = _FakeBackend()
    out = await m23_attest_compliance.handler(
        {
            "tenant_id": "acme",
            "gdpr_art22": True,
            "signed_by": "jane.doe@acme.com",
            "dpo_contact": "dpo@acme.com",
            "confirm": True,
        },
        _ctx(be),
    )
    assert out["gdpr_art22_acknowledged"] is True
    assert out["dpo_contact"] == "dpo@acme.com"


@pytest.mark.asyncio
async def test_no_attestation_at_all_rejected():
    be = _FakeBackend()
    with pytest.raises(ToolError) as exc:
        await m23_attest_compliance.handler(
            {
                "tenant_id": "acme",
                "signed_by": "jane.doe@acme.com",
                "confirm": True,
            },
            _ctx(be),
        )
    assert exc.value.code == "invalid_input"


@pytest.mark.asyncio
async def test_missing_confirm_rejected():
    be = _FakeBackend()
    with pytest.raises(ToolError) as exc:
        await m23_attest_compliance.handler(
            {
                "tenant_id": "acme",
                "ai_act_art14": True,
                "signed_by": "jane.doe@acme.com",
            },
            _ctx(be),
        )
    assert exc.value.code == "confirm_required"


@pytest.mark.asyncio
async def test_both_attestations_at_once():
    be = _FakeBackend()
    out = await m23_attest_compliance.handler(
        {
            "tenant_id": "acme",
            "ai_act_art14": True,
            "gdpr_art22": True,
            "signed_by": "jane.doe@acme.com",
            "dpo_contact": "dpo@acme.com",
            "confirm": True,
        },
        _ctx(be),
    )
    assert out["ai_act_art14_acknowledged"] is True
    assert out["gdpr_art22_acknowledged"] is True
