"""Lua atomic_swap tests against the in-memory runner."""

from __future__ import annotations

import pytest

from efe_registry.router.lua_runner import InMemoryLuaRunner


@pytest.mark.asyncio
async def test_first_write_increments_version():
    lua = InMemoryLuaRunner()
    res = await lua.atomic_swap(
        version_key="routing:acme:sum:v",
        routing_key="routing:acme:sum",
        pubsub_channel="routing.updated",
        expected_version=0,
        new_version=1,
        entries=(("ep-1", 100, "champion"),),
    )
    assert res.ok
    assert res.new_version == 1
    assert lua.snapshot("routing:acme:sum") == {"ep-1": "100:champion"}


@pytest.mark.asyncio
async def test_version_mismatch_refuses():
    lua = InMemoryLuaRunner()
    await lua.atomic_swap(
        version_key="routing:acme:sum:v",
        routing_key="routing:acme:sum",
        pubsub_channel="routing.updated",
        expected_version=0,
        new_version=1,
        entries=(("ep-1", 100, "champion"),),
    )
    res2 = await lua.atomic_swap(
        version_key="routing:acme:sum:v",
        routing_key="routing:acme:sum",
        pubsub_channel="routing.updated",
        expected_version=0,  # stale
        new_version=1,
        entries=(("ep-1", 100, "champion"),),
    )
    assert not res2.ok
    assert "version_mismatch" in (res2.error or "")


@pytest.mark.asyncio
async def test_weights_not_100_refuses():
    lua = InMemoryLuaRunner()
    res = await lua.atomic_swap(
        version_key="routing:acme:sum:v",
        routing_key="routing:acme:sum",
        pubsub_channel="routing.updated",
        expected_version=0,
        new_version=1,
        entries=(
            ("ep-1", 70, "champion"),
            ("ep-2", 20, "candidate"),  # sum = 90, not 100
        ),
    )
    assert not res.ok
    assert "weights_not_100" in (res.error or "")


@pytest.mark.asyncio
async def test_non_sequential_version_refuses():
    lua = InMemoryLuaRunner()
    res = await lua.atomic_swap(
        version_key="routing:acme:sum:v",
        routing_key="routing:acme:sum",
        pubsub_channel="routing.updated",
        expected_version=0,
        new_version=5,
        entries=(("ep-1", 100, "champion"),),
    )
    assert not res.ok
    assert "sequential" in (res.error or "")


@pytest.mark.asyncio
async def test_publish_is_emitted_on_success():
    lua = InMemoryLuaRunner()
    await lua.atomic_swap(
        version_key="routing:acme:sum:v",
        routing_key="routing:acme:sum",
        pubsub_channel="routing.updated",
        expected_version=0,
        new_version=1,
        entries=(("ep-1", 100, "champion"),),
    )
    assert lua.published
    channel, payload = lua.published[0]
    assert channel == "routing.updated"
    assert "acme:sum" in payload
