"""Pin tests for FallbackChain (multi-provider failover)."""
from __future__ import annotations

import asyncio

import pytest

from efe_byo_agent.exceptions import (
    AllProvidersFailedError,
    ProviderError,
    ProviderTimeoutError,
)
from efe_byo_agent.llm.base import LLMProvider
from efe_byo_agent.llm.fallback import FallbackChain
from efe_byo_agent.types import CostModel, LLMResponse, ProviderHealth


class _FakeProvider(LLMProvider):
    """Toy provider for tests : deterministic success/failure."""

    def __init__(self, *, name: str, fail: bool = False, latency_ms: float = 0):
        self._name = name
        self._fail = fail
        self._latency = latency_ms
        self.calls = 0

    @property
    def name(self) -> str:
        return self._name

    @property
    def supports_tools(self) -> bool:
        return True

    @property
    def supports_streaming(self) -> bool:
        return True

    @property
    def max_context_tokens(self) -> int:
        return 100_000

    @property
    def cost_model(self) -> CostModel:
        return CostModel(input_per_1m_tokens_usd=0, output_per_1m_tokens_usd=0)

    async def complete(self, **_kwargs) -> LLMResponse:
        self.calls += 1
        if self._latency:
            await asyncio.sleep(self._latency / 1000)
        if self._fail:
            raise ProviderError("simulated failure", provider_name=self._name)
        return LLMResponse(
            text=f"resp from {self._name}",
            tokens_in=10, tokens_out=5, cost_usd=0.0,
            finish_reason="stop",
            provider_name=self._name,
            model_name="fake",
        )

    async def stream(self, **kwargs):
        yield await self.complete(**kwargs)

    async def health(self) -> ProviderHealth:
        return ProviderHealth(available=not self._fail, last_check_ts=0.0)


@pytest.mark.asyncio
async def test_fallback_uses_primary_when_healthy():
    primary = _FakeProvider(name="p1")
    fallback = _FakeProvider(name="p2")
    chain = FallbackChain(primary=primary, fallbacks=[fallback])
    resp = await chain.complete(
        system_prompt=None, messages=[{"role": "user", "content": "hi"}],
    )
    assert resp.provider_name == "p1"
    assert primary.calls == 1
    assert fallback.calls == 0


@pytest.mark.asyncio
async def test_fallback_uses_fallback_when_primary_fails():
    primary = _FakeProvider(name="p1", fail=True)
    fallback = _FakeProvider(name="p2")
    chain = FallbackChain(primary=primary, fallbacks=[fallback])
    resp = await chain.complete(
        system_prompt=None, messages=[{"role": "user", "content": "hi"}],
    )
    assert resp.provider_name == "p2"


@pytest.mark.asyncio
async def test_fallback_all_fail_raises_aggregate():
    p1 = _FakeProvider(name="p1", fail=True)
    p2 = _FakeProvider(name="p2", fail=True)
    chain = FallbackChain(primary=p1, fallbacks=[p2])
    with pytest.raises(AllProvidersFailedError) as exc:
        await chain.complete(
            system_prompt=None, messages=[{"role": "user", "content": "hi"}],
        )
    assert len(exc.value.errors) == 2


@pytest.mark.asyncio
async def test_fallback_circuit_opens_after_threshold():
    p1 = _FakeProvider(name="p1", fail=True)
    p2 = _FakeProvider(name="p2")
    chain = FallbackChain(
        primary=p1, fallbacks=[p2],
        circuit_breaker_threshold=3,
    )
    # 3 failures should OPEN the circuit on p1.
    for _ in range(3):
        await chain.complete(
            system_prompt=None, messages=[{"role": "user", "content": "x"}],
        )
    states = chain.breaker_states()
    assert states["p1"] == "open"
    assert states["p2"] == "closed"


@pytest.mark.asyncio
async def test_fallback_skips_open_circuit_provider():
    p1 = _FakeProvider(name="p1", fail=True)
    p2 = _FakeProvider(name="p2")
    chain = FallbackChain(
        primary=p1, fallbacks=[p2], circuit_breaker_threshold=2,
    )
    # Trigger circuit open on p1.
    for _ in range(2):
        await chain.complete(
            system_prompt=None, messages=[{"role": "user", "content": "x"}],
        )
    p1_calls_before = p1.calls
    # Now p1 circuit is open ; next call should go directly to p2.
    await chain.complete(
        system_prompt=None, messages=[{"role": "user", "content": "x"}],
    )
    assert p1.calls == p1_calls_before  # p1 not retried


@pytest.mark.asyncio
async def test_fallback_timeout_treated_as_failure():
    slow = _FakeProvider(name="slow", latency_ms=200)
    fast = _FakeProvider(name="fast")
    chain = FallbackChain(
        primary=slow, fallbacks=[fast], per_provider_timeout_s=0.05,
    )
    resp = await chain.complete(
        system_prompt=None, messages=[{"role": "user", "content": "x"}],
    )
    assert resp.provider_name == "fast"


def test_fallback_requires_at_least_one_provider():
    with pytest.raises(ValueError):
        FallbackChain(primary=None, fallbacks=[])  # type: ignore[arg-type]


@pytest.mark.asyncio
async def test_fallback_cost_model_from_primary():
    p1 = _FakeProvider(name="p1")
    p2 = _FakeProvider(name="p2")
    chain = FallbackChain(primary=p1, fallbacks=[p2])
    cm = chain.cost_model
    assert cm.input_per_1m_tokens_usd == 0
