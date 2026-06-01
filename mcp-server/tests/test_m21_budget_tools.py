"""Pin tests for M21 budget tools (get/set + invalidate cache)."""
from __future__ import annotations

import pytest


@pytest.fixture
def fake_backend_with_budget(fake_backend):
    """Augment FakeBackend with budget + cache mocks."""

    async def get_llm_budget(*, auth):
        return {
            "monthly_cap_usd": 500.0,
            "spent_this_month_usd": 47.32,
            "pct_used": 9.46,
            "soft_threshold_pct": 80,
            "fail_open": False,
            "rollover_dom": 1,
            "last_rollover": "2026-12-01",
        }

    async def set_llm_budget(
        *, monthly_cap_usd, soft_threshold_pct, fail_open, rollover_dom, auth,
    ):
        return {
            "monthly_cap_usd": monthly_cap_usd,
            "soft_threshold_pct": soft_threshold_pct,
            "fail_open": fail_open,
            "rollover_dom": rollover_dom,
            "applied_at": "2026-12-15T10:00:00Z",
        }

    async def list_llm_providers(*, include_health, auth):
        return [
            {
                "name": "anthropic:claude-sonnet-4-5",
                "available": True,
                "circuit_state": "closed",
                "latency_p50_ms": 487,
            },
            {
                "name": "openai:gpt-5-mini",
                "available": True,
                "circuit_state": "closed",
                "latency_p50_ms": 312,
            },
        ]

    async def invalidate_llm_cache(*, scope, catalog, auth):
        return {
            "deleted_count": 42 if scope == "tenant" else 7,
            "applied_at": "2026-12-15T10:00:00Z",
        }

    fake_backend.get_llm_budget = get_llm_budget
    fake_backend.set_llm_budget = set_llm_budget
    fake_backend.list_llm_providers = list_llm_providers
    fake_backend.invalidate_llm_cache = invalidate_llm_cache
    return fake_backend


@pytest.fixture
def m21_dispatcher_with_budget(
    monkeypatch,
    fake_backend_with_budget,
    auth_ctx,
    audit_emitter,
    rate_limiter,
    sampling_client,
    embedding_cache,
    bundle_cache,
    oci_factory,
):
    from mcp_server.dispatcher import Dispatcher
    from mcp_server.profile import Profile
    from mcp_server.tools.registry import build_tool_registry

    monkeypatch.setenv("EFE_FEATURE_M21_MODE2_ENABLED", "true")
    registry = build_tool_registry(Profile.AGENT_NATIVE_DEPLOY)
    return Dispatcher(
        registry=registry,
        backend=fake_backend_with_budget,
        auth=auth_ctx,
        audit=audit_emitter,
        rate_limiter=rate_limiter,
        profile=Profile.AGENT_NATIVE_DEPLOY,
        sampling=sampling_client,
        embedding_cache=embedding_cache,
        bundle_cache=bundle_cache,
        oci_factory=oci_factory,
    )


# ─── efe_get_llm_budget ─────────────────────────────────────────────


@pytest.mark.asyncio
async def test_get_llm_budget_returns_state(m21_dispatcher_with_budget):
    result = await m21_dispatcher_with_budget.dispatch(
        tool_name="efe_get_llm_budget",
        args={},
    )
    assert result.is_error is False
    body = result.content
    assert body["monthly_cap_usd"] == 500.0
    assert body["spent_this_month_usd"] == 47.32
    assert body["pct_used"] == 9.46
    assert body["warning"] is False  # 9.46% < 80%


# ─── efe_set_llm_budget ────────────────────────────────────────────


@pytest.mark.asyncio
async def test_set_llm_budget_requires_confirm(m21_dispatcher_with_budget):
    result = await m21_dispatcher_with_budget.dispatch(
        tool_name="efe_set_llm_budget",
        args={"monthly_cap_usd": 100},  # missing confirm
    )
    assert result.is_error is True


@pytest.mark.asyncio
async def test_set_llm_budget_validates_cap_range(m21_dispatcher_with_budget):
    result = await m21_dispatcher_with_budget.dispatch(
        tool_name="efe_set_llm_budget",
        args={
            "monthly_cap_usd": 1_000_000,  # > max 100k
            "confirm": True,
        },
    )
    assert result.is_error is True


@pytest.mark.asyncio
async def test_set_llm_budget_basic_path(m21_dispatcher_with_budget):
    result = await m21_dispatcher_with_budget.dispatch(
        tool_name="efe_set_llm_budget",
        args={
            "monthly_cap_usd": 500,
            "soft_threshold_pct": 75,
            "fail_open": False,
            "rollover_dom": 1,
            "confirm": True,
        },
    )
    assert result.is_error is False
    body = result.content
    assert body["monthly_cap_usd"] == 500
    assert body["soft_threshold_pct"] == 75


@pytest.mark.asyncio
async def test_set_llm_budget_emits_audit(
    m21_dispatcher_with_budget, audit_emitter,
):
    audit_emitter.enabled = False
    before = len(audit_emitter.recent(limit=500))
    await m21_dispatcher_with_budget.dispatch(
        tool_name="efe_set_llm_budget",
        args={"monthly_cap_usd": 200, "confirm": True},
    )
    after = len(audit_emitter.recent(limit=500))
    assert after == before + 1


# ─── efe_list_llm_providers ────────────────────────────────────────


@pytest.mark.asyncio
async def test_list_llm_providers_returns_list(m21_dispatcher_with_budget):
    result = await m21_dispatcher_with_budget.dispatch(
        tool_name="efe_list_llm_providers",
        args={"include_health": True},
    )
    assert result.is_error is False
    body = result.content
    assert body["count"] == 2
    assert len(body["providers"]) == 2


# ─── efe_invalidate_llm_cache ──────────────────────────────────────


@pytest.mark.asyncio
async def test_invalidate_llm_cache_per_tenant(m21_dispatcher_with_budget):
    result = await m21_dispatcher_with_budget.dispatch(
        tool_name="efe_invalidate_llm_cache",
        args={"scope": "tenant", "confirm": True},
    )
    assert result.is_error is False
    assert result.content["deleted_count"] == 42


@pytest.mark.asyncio
async def test_invalidate_llm_cache_per_catalog(m21_dispatcher_with_budget):
    result = await m21_dispatcher_with_budget.dispatch(
        tool_name="efe_invalidate_llm_cache",
        args={
            "scope": "catalog",
            "catalog": "support-routing",
            "confirm": True,
        },
    )
    assert result.is_error is False
    assert result.content["deleted_count"] == 7


@pytest.mark.asyncio
async def test_invalidate_llm_cache_requires_confirm(m21_dispatcher_with_budget):
    result = await m21_dispatcher_with_budget.dispatch(
        tool_name="efe_invalidate_llm_cache",
        args={"scope": "tenant"},  # missing confirm
    )
    assert result.is_error is True


@pytest.mark.asyncio
async def test_invalidate_llm_cache_catalog_requires_catalog_arg(
    m21_dispatcher_with_budget,
):
    result = await m21_dispatcher_with_budget.dispatch(
        tool_name="efe_invalidate_llm_cache",
        args={"scope": "catalog", "confirm": True},  # catalog missing
    )
    assert result.is_error is True
