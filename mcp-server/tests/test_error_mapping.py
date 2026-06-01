"""Error mapping pin tests (M16) — backend errors → MCP isError + code."""
from __future__ import annotations

import pytest

from mcp_server.backend_client import (
    BackendBadRequest,
    BackendForbidden,
    BackendNotFound,
    BackendQuotaExceeded,
    BackendUnavailable,
)


@pytest.mark.asyncio
async def test_backend_400_maps_to_invalid_arguments(dispatcher, fake_backend):
    fake_backend.next_failure = BackendBadRequest("bad input", status_code=400)
    res = await dispatcher.dispatch(
        tool_name="efe_list_catalogs", args={}
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_backend_403_maps_to_forbidden(dispatcher, fake_backend):
    fake_backend.next_failure = BackendForbidden("nope", status_code=403)
    res = await dispatcher.dispatch(
        tool_name="efe_list_catalogs", args={}
    )
    assert res.is_error is True
    assert res.content["code"] == "forbidden"


@pytest.mark.asyncio
async def test_backend_404_maps_to_not_found(dispatcher, fake_backend):
    fake_backend.next_failure = BackendNotFound("nope", status_code=404)
    res = await dispatcher.dispatch(
        tool_name="efe_get_audit_log", args={"limit": 10}
    )
    assert res.is_error is True
    assert res.content["code"] == "not_found"


@pytest.mark.asyncio
async def test_backend_429_maps_to_quota_exceeded(dispatcher, fake_backend):
    fake_backend.next_failure = BackendQuotaExceeded("over quota", status_code=429)
    res = await dispatcher.dispatch(
        tool_name="efe_list_catalogs", args={}
    )
    assert res.is_error is True
    assert res.content["code"] == "quota_exceeded"


@pytest.mark.asyncio
async def test_backend_503_maps_to_backend_unavailable(dispatcher, fake_backend):
    fake_backend.next_failure = BackendUnavailable("upstream down", status_code=503)
    res = await dispatcher.dispatch(
        tool_name="efe_list_catalogs", args={}
    )
    assert res.is_error is True
    assert res.content["code"] == "backend_unavailable"


@pytest.mark.asyncio
async def test_unhandled_exception_maps_to_internal_error(dispatcher, fake_backend):
    fake_backend.next_failure = RuntimeError("boom")
    res = await dispatcher.dispatch(
        tool_name="efe_list_catalogs", args={}
    )
    assert res.is_error is True
    assert res.content["code"] == "internal_error"


@pytest.mark.asyncio
async def test_rate_limit_returns_structured_error(rate_limiter, dispatcher):
    """Per-(token, tool) bucket exhausts → rate_limit_exceeded error code."""
    # Force a tiny bucket so the 2nd request is denied.
    rate_limiter.per_min = 1
    rate_limiter.reset()
    res1 = await dispatcher.dispatch(tool_name="efe_list_catalogs", args={})
    assert res1.is_error is False
    res2 = await dispatcher.dispatch(tool_name="efe_list_catalogs", args={})
    assert res2.is_error is True
    assert res2.content["code"] == "rate_limit_exceeded"
    assert "rate_limit" in res2.content
    assert res2.content["rate_limit"]["capacity"] == 1


@pytest.mark.asyncio
async def test_expired_token_blocks_dispatch(registry, fake_backend, audit_emitter, rate_limiter):
    """Dispatcher refuses calls when the JWT is expired (defense-in-depth)."""
    from mcp_server.auth import AuthContext
    from mcp_server.dispatcher import Dispatcher

    expired = AuthContext(raw_token="t" * 16, exp=1)  # epoch 1 = year 1970
    disp = Dispatcher(
        registry=registry,
        backend=fake_backend,
        auth=expired,
        audit=audit_emitter,
        rate_limiter=rate_limiter,
    )
    res = await disp.dispatch(tool_name="efe_list_catalogs", args={})
    assert res.is_error is True
    assert res.content["code"] == "token_expired"
