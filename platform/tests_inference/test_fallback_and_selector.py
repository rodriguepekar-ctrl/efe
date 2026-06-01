"""BackendSelector + FallbackChain."""

from __future__ import annotations

import pytest

from efe_inference.backends.mock import MockBackend
from efe_inference.backends.registry import BackendRegistry
from efe_inference.backends.selector import (
    BackendSelector,
    FallbackChain,
    FallbackEntry,
)
from efe_inference.clock import FakeClock
from efe_inference.config import CircuitSettings
from efe_inference.errors import AllBackendsDown
from efe_inference.resilience.circuit import CircuitBreakerRegistry


@pytest.fixture
def circuit_reg() -> CircuitBreakerRegistry:
    return CircuitBreakerRegistry(
        settings=CircuitSettings(
            failure_threshold=1, window_s=60, cool_down_s=60,
        ),
        clock=FakeClock(),
    )


@pytest.mark.asyncio
async def test_selector_returns_all_when_all_closed(circuit_reg):
    reg = BackendRegistry()
    reg.register(MockBackend(name="a"))
    reg.register(MockBackend(name="b"))
    chain = FallbackChain(entries=(
        FallbackEntry(backend_name="a"),
        FallbackEntry(backend_name="b"),
    ))
    sel = BackendSelector(
        registry=reg, circuit=circuit_reg, default_chain=chain,
    )
    result = await sel.select(tenant_id="t", task_id="q")
    assert [n for (_, a) in result for n in (a.name,)] == ["a", "b"]


@pytest.mark.asyncio
async def test_selector_skips_open_circuits(circuit_reg):
    reg = BackendRegistry()
    reg.register(MockBackend(name="a"))
    reg.register(MockBackend(name="b"))
    chain = FallbackChain(entries=(
        FallbackEntry(backend_name="a"),
        FallbackEntry(backend_name="b"),
    ))
    sel = BackendSelector(
        registry=reg, circuit=circuit_reg, default_chain=chain,
    )
    # Open "a"
    br = circuit_reg.get("t", "a")
    await br.on_failure()
    result = await sel.select(tenant_id="t", task_id="q")
    assert [a.name for (_, a) in result] == ["b"]


@pytest.mark.asyncio
async def test_selector_raises_when_all_open(circuit_reg):
    reg = BackendRegistry()
    reg.register(MockBackend(name="a"))
    chain = FallbackChain(entries=(FallbackEntry(backend_name="a"),))
    sel = BackendSelector(
        registry=reg, circuit=circuit_reg, default_chain=chain,
    )
    br = circuit_reg.get("t", "a")
    await br.on_failure()
    with pytest.raises(AllBackendsDown):
        await sel.select(tenant_id="t", task_id="q")


def test_fallback_chain_rejects_empty():
    with pytest.raises(ValueError):
        FallbackChain(entries=())


def test_resolve_chain_prefers_tenant_task():
    reg = BackendRegistry()
    reg.register(MockBackend(name="a"))
    reg.register(MockBackend(name="b"))
    default = FallbackChain(entries=(FallbackEntry(backend_name="a"),))
    per_tt = FallbackChain(entries=(FallbackEntry(backend_name="b"),))
    sel = BackendSelector(
        registry=reg,
        circuit=CircuitBreakerRegistry(
            settings=CircuitSettings(), clock=FakeClock(),
        ),
        default_chain=default,
        per_tenant_chains={("acme", "qa"): per_tt},
    )
    chain = sel.resolve_chain(tenant_id="acme", task_id="qa")
    assert chain.entries[0].backend_name == "b"
    chain2 = sel.resolve_chain(tenant_id="acme", task_id="other")
    assert chain2.entries[0].backend_name == "a"
