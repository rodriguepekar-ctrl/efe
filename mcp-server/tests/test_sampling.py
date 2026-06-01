"""Sampling client pin tests (M17 D45)."""
from __future__ import annotations

import asyncio

import pytest

from mcp_server.sampling import (
    SamplingBudgetExceeded,
    SamplingClient,
    SamplingTimeout,
    SamplingUnavailable,
)
from mcp_server.state import SamplingCache


@pytest.mark.asyncio
async def test_sampling_budget_default_5_rounds(sampling_client):
    for i in range(5):
        # Vary prompt to bypass cache
        await sampling_client.sample(tenant_id="t", prompt=f"q{i}")
    assert sampling_client.usage.rounds_used == 5


@pytest.mark.asyncio
async def test_sampling_budget_exceeded_raises_after_5(sampling_client):
    for i in range(5):
        await sampling_client.sample(tenant_id="t", prompt=f"q{i}")
    with pytest.raises(SamplingBudgetExceeded):
        await sampling_client.sample(tenant_id="t", prompt="q-extra")


@pytest.mark.asyncio
async def test_sampling_cache_avoids_round_trip(sampling_client):
    await sampling_client.sample(tenant_id="t", prompt="cached")
    assert sampling_client.usage.rounds_used == 1
    # Second call same prompt => cache hit, no new round used
    await sampling_client.sample(tenant_id="t", prompt="cached")
    assert sampling_client.usage.rounds_used == 1
    assert sampling_client.usage.cache_hits == 1


@pytest.mark.asyncio
async def test_sampling_unavailable_when_session_none():
    client = SamplingClient(None, client_supports_sampling=True)
    with pytest.raises(SamplingUnavailable):
        await client.sample(tenant_id="t", prompt="x")


@pytest.mark.asyncio
async def test_sampling_unavailable_when_client_unsupported(fake_sampling_session):
    client = SamplingClient(fake_sampling_session, client_supports_sampling=False)
    with pytest.raises(SamplingUnavailable):
        await client.sample(tenant_id="t", prompt="x")


@pytest.mark.asyncio
async def test_sampling_reset_usage(sampling_client):
    await sampling_client.sample(tenant_id="t", prompt="r1")
    assert sampling_client.usage.rounds_used == 1
    sampling_client.reset_usage()
    assert sampling_client.usage.rounds_used == 0


@pytest.mark.asyncio
async def test_sampling_caches_per_tenant(sampling_client):
    """TM-M17-I-01 — cache is keyed per tenant_id."""
    await sampling_client.sample(tenant_id="acme", prompt="shared")
    await sampling_client.sample(tenant_id="beta", prompt="shared")
    # Both consumed a fresh round (no cross-tenant leak)
    assert sampling_client.usage.rounds_used == 2
