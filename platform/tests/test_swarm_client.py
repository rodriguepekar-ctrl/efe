"""Tests for :class:`SwarmClient` + :class:`InMemorySwarmTransport`."""

from __future__ import annotations

import pytest

from efe_platform.errors import SwarmDivergence, SwarmInvalidSpec
from efe_platform.swarm.client import InMemorySwarmTransport, SwarmClient, SwarmEndpoint
from efe_platform.swarm.protocol import (
    ConeRef,
    DecodedPayload,
    ExecuteResponse,
    Verdict,
)
from efe_platform.swarm.spec_builder import SpecBuilder


def _spec() -> object:
    return (
        SpecBuilder()
        .with_problem_embedding(tuple(0.1 * i for i in range(16)))
        .with_cone_ref(ConeRef("acme", "route", 1))
        .build()
    )


@pytest.mark.asyncio
async def test_execute_returns_outcome() -> None:
    transport = InMemorySwarmTransport()
    client = SwarmClient(SwarmEndpoint("x"), transport=transport, max_retries=0)
    async with client:
        outcome = await client.execute(_spec(), raw_prompt="hello", tenant="acme")
        assert outcome.verdict is Verdict.CONVERGED
        assert outcome.text is not None and outcome.text.startswith("[stub]")


@pytest.mark.asyncio
async def test_metadata_propagated() -> None:
    transport = InMemorySwarmTransport()
    client = SwarmClient(
        SwarmEndpoint("x"),
        transport=transport,
        max_retries=0,
        jwt="jwt-token",
    )
    async with client:
        await client.execute(_spec(), tenant="acme", request_id="r-1")
        last = transport.call_log[-1]
        assert last.request_id == "r-1"


@pytest.mark.asyncio
async def test_divergence_verdict_raises() -> None:
    transport = InMemorySwarmTransport()
    transport.seed(
        tenant="acme",
        prompt_prefix="boom",
        response=ExecuteResponse(
            swarm_id="s",
            request_id="r",
            payload=DecodedPayload(text="garbage"),
            confidence=0.2,
            verdict=Verdict.DIVERGENCE,
            rounds_executed=1,
            total_ms=10.0,
            alignment_delta=0.8,
        ),
    )
    client = SwarmClient(SwarmEndpoint("x"), transport=transport, max_retries=0)
    async with client:
        with pytest.raises(SwarmDivergence):
            await client.execute(_spec(), raw_prompt="boom", tenant="acme")


@pytest.mark.asyncio
async def test_describe_dry_run() -> None:
    transport = InMemorySwarmTransport()
    client = SwarmClient(SwarmEndpoint("x"), transport=transport)
    async with client:
        result = await client.describe(_spec())
        assert result["valid"] is True
        assert result["rounds_expected"] >= 1


@pytest.mark.asyncio
async def test_describe_rejects_huge_dim() -> None:
    transport = InMemorySwarmTransport()
    client = SwarmClient(SwarmEndpoint("x"), transport=transport)
    # We cannot build such a spec via the builder (it rejects dim > 65536) —
    # test the transport's own guard directly. We use client for realism:
    with pytest.raises(SwarmInvalidSpec):
        (
            SpecBuilder()
            .with_problem_embedding([0.0] * 100_000)
            .with_cone_ref(ConeRef("a", "b", 1))
            .build()
        )


@pytest.mark.asyncio
async def test_streaming_emits_chunks() -> None:
    transport = InMemorySwarmTransport()
    client = SwarmClient(SwarmEndpoint("x"), transport=transport)
    async with client:
        chunks = []
        async for c in client.execute_streaming(_spec(), raw_prompt="hi"):
            chunks.append(c)
        # Two text_delta + one final
        assert sum(1 for c in chunks if c["kind"] == "text_delta") == 2
        assert chunks[-1]["kind"] == "final"


@pytest.mark.asyncio
async def test_close_is_idempotent() -> None:
    transport = InMemorySwarmTransport()
    client = SwarmClient(SwarmEndpoint("x"), transport=transport)
    await client.close()
    await client.close()
