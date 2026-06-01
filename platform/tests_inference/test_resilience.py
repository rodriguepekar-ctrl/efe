"""Circuit breaker FSM + Retry + Timeout."""

from __future__ import annotations

import pytest

from efe_inference.clock import FakeClock
from efe_inference.config import (
    CircuitSettings,
    RetrySettings,
    TimeoutSettings,
)
from efe_inference.errors import (
    BackendAuthError,
    BackendUnavailable,
    CircuitOpen,
)
from efe_inference.resilience.circuit import (
    CircuitBreaker,
    CircuitBreakerRegistry,
    CircuitStateValue,
)
from efe_inference.resilience.retry import RetryExecutor
from efe_inference.resilience.timeout import TimeoutBudget


@pytest.mark.asyncio
async def test_circuit_transitions_closed_to_open():
    clock = FakeClock()
    cb = CircuitBreaker(
        settings=CircuitSettings(
            failure_threshold=2, window_s=60, cool_down_s=10,
        ),
        clock=clock,
    )
    assert cb.state == CircuitStateValue.CLOSED
    await cb.on_failure()
    assert cb.state == CircuitStateValue.CLOSED
    await cb.on_failure()
    assert cb.state == CircuitStateValue.OPEN


@pytest.mark.asyncio
async def test_circuit_refuses_when_open():
    clock = FakeClock()
    cb = CircuitBreaker(
        settings=CircuitSettings(
            failure_threshold=1, window_s=60, cool_down_s=10,
        ),
        clock=clock,
    )
    await cb.on_failure()
    with pytest.raises(CircuitOpen):
        await cb.allow()


@pytest.mark.asyncio
async def test_circuit_cool_down_moves_to_half_open():
    clock = FakeClock()
    cb = CircuitBreaker(
        settings=CircuitSettings(
            failure_threshold=1, window_s=60, cool_down_s=5,
        ),
        clock=clock,
    )
    await cb.on_failure()
    assert cb.state == CircuitStateValue.OPEN
    clock.advance(6.0)
    # allow() internally transitions to HALF_OPEN and permits one probe.
    await cb.allow()
    assert cb.state == CircuitStateValue.HALF_OPEN


@pytest.mark.asyncio
async def test_circuit_half_open_success_closes():
    clock = FakeClock()
    cb = CircuitBreaker(
        settings=CircuitSettings(
            failure_threshold=1, success_threshold=1,
            window_s=60, cool_down_s=5,
        ),
        clock=clock,
    )
    await cb.on_failure()
    clock.advance(6.0)
    await cb.allow()
    await cb.on_success()
    assert cb.state == CircuitStateValue.CLOSED


@pytest.mark.asyncio
async def test_circuit_registry_isolates_tenants():
    clock = FakeClock()
    reg = CircuitBreakerRegistry(
        settings=CircuitSettings(
            failure_threshold=1, window_s=60, cool_down_s=10,
        ),
        clock=clock,
    )
    a = reg.get("tenant-A", "backend-X")
    b = reg.get("tenant-B", "backend-X")
    await a.on_failure()
    assert a.state == CircuitStateValue.OPEN
    assert b.state == CircuitStateValue.CLOSED


@pytest.mark.asyncio
async def test_retry_executor_retries_on_retryable():
    calls = 0

    async def _flaky() -> str:
        nonlocal calls
        calls += 1
        if calls < 2:
            raise BackendUnavailable("flap")
        return "ok"

    rx = RetryExecutor(
        settings=RetrySettings(max_attempts=3, backoff_base_s=0.001),
    )
    out = await rx.run(_flaky)
    assert out == "ok"
    assert calls == 2


@pytest.mark.asyncio
async def test_retry_executor_gives_up_on_non_retryable():
    async def _bad() -> str:
        raise BackendAuthError("no")

    rx = RetryExecutor(settings=RetrySettings(max_attempts=5))
    with pytest.raises(BackendAuthError):
        await rx.run(_bad)


def test_timeout_budget_from_settings():
    s = TimeoutSettings(total_s=30.0)
    b = TimeoutBudget.from_settings(s)
    assert b.total_s == 30.0


def test_timeout_budget_clamped_by_total():
    s = TimeoutSettings(total_s=60.0, connect_s=5.0, first_byte_s=15.0)
    b = TimeoutBudget.from_settings(s).with_total(2.0)
    assert b.total_s == 2.0
    assert b.connect_s == 2.0
    assert b.first_byte_s == 2.0
