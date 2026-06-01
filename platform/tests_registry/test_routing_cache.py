"""Routing cache TTL + invalidation."""

from __future__ import annotations

import pytest

from efe_registry.clock import FakeClock
from efe_registry.core.artifact import ArtifactRole
from efe_registry.router.cache import RoutingCache
from efe_registry.router.routing_table import RoutingEntry, RoutingKey


@pytest.mark.asyncio
async def test_cache_hit_avoids_loader():
    calls = 0

    async def loader(key):
        nonlocal calls
        calls += 1
        return ((RoutingEntry("ep-1", 100, ArtifactRole.CHAMPION),), 1)

    clock = FakeClock()
    cache = RoutingCache(loader=loader, ttl_s=1.0, clock=clock)
    k = RoutingKey("acme", "sum")
    await cache.get(k)
    await cache.get(k)
    await cache.get(k)
    assert calls == 1


@pytest.mark.asyncio
async def test_cache_reloads_after_ttl():
    calls = 0

    async def loader(key):
        nonlocal calls
        calls += 1
        return ((RoutingEntry("ep-1", 100, ArtifactRole.CHAMPION),), calls)

    clock = FakeClock()
    cache = RoutingCache(loader=loader, ttl_s=1.0, clock=clock)
    k = RoutingKey("acme", "sum")
    await cache.get(k)
    clock.advance(2.0)
    await cache.get(k)
    assert calls == 2


@pytest.mark.asyncio
async def test_cache_invalidate_drops_entry():
    calls = 0

    async def loader(key):
        nonlocal calls
        calls += 1
        return ((RoutingEntry("ep-1", 100, ArtifactRole.CHAMPION),), 1)

    clock = FakeClock()
    cache = RoutingCache(loader=loader, ttl_s=60.0, clock=clock)
    k = RoutingKey("acme", "sum")
    await cache.get(k)
    dropped = cache.invalidate("routing:acme:sum")
    assert dropped == 1
    await cache.get(k)
    assert calls == 2


@pytest.mark.asyncio
async def test_cache_invalidate_bad_key_is_noop():
    async def loader(key):
        return ((RoutingEntry("ep-1", 100, ArtifactRole.CHAMPION),), 1)

    cache = RoutingCache(loader=loader, ttl_s=60.0)
    assert cache.invalidate("bogus") == 0
    assert cache.invalidate("routing:") == 0
