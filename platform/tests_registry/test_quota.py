"""Quota enforcer tests."""

from __future__ import annotations

import pytest

from efe_registry.clock import FakeClock
from efe_registry.errors import QuotaExceeded
from efe_registry.router.lua_runner import InMemoryLuaRunner
from efe_registry.router.quota import QuotaEnforcer, QuotaSpec


@pytest.mark.asyncio
async def test_bucket_refills():
    clock = FakeClock()
    lua = InMemoryLuaRunner()
    enforcer = QuotaEnforcer(
        lua=lua,
        default_spec=QuotaSpec(capacity=5, refill_per_s=2, cost=1),
        clock=clock,
    )
    # Consume 5 tokens — all succeed.
    for _ in range(5):
        assert await enforcer.check(tenant="acme", api_key="k1")
    # 6th request — refuses (bucket empty).
    assert not await enforcer.check(tenant="acme", api_key="k1")
    # Advance 3s → refill fully back to capacity (3 * 2 = 6 tokens,
    # capped at 5).
    clock.advance(3.0)
    successes = 0
    for _ in range(5):
        if await enforcer.check(tenant="acme", api_key="k1"):
            successes += 1
    assert successes == 5
    # 6th after 5 successes — empty again.
    assert not await enforcer.check(tenant="acme", api_key="k1")


@pytest.mark.asyncio
async def test_tenant_isolation():
    clock = FakeClock()
    lua = InMemoryLuaRunner()
    enforcer = QuotaEnforcer(
        lua=lua,
        default_spec=QuotaSpec(capacity=1, refill_per_s=0, cost=1),
        clock=clock,
    )
    assert await enforcer.check(tenant="acme", api_key="k1")
    assert not await enforcer.check(tenant="acme", api_key="k1")
    # Different tenant: its bucket is independent.
    assert await enforcer.check(tenant="globex", api_key="k1")


@pytest.mark.asyncio
async def test_enforce_raises_on_exhaust():
    clock = FakeClock()
    lua = InMemoryLuaRunner()
    enforcer = QuotaEnforcer(
        lua=lua,
        default_spec=QuotaSpec(capacity=1, refill_per_s=0, cost=1),
        clock=clock,
    )
    await enforcer.enforce(tenant="acme", api_key="k")
    with pytest.raises(QuotaExceeded):
        await enforcer.enforce(tenant="acme", api_key="k")
