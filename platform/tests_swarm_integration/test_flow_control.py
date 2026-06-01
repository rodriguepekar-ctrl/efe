"""Bulkhead + Circuit breaker + retry tests."""

from __future__ import annotations

import asyncio

import pytest

from efe_swarm_integration.config import RetryConfig
from efe_swarm_integration.errors import BackpressureRejected, CircuitOpen
from efe_swarm_integration.flow_control.bulkhead import BulkheadRegistry
from efe_swarm_integration.flow_control.circuit_breaker import (
    CbState,
    CircuitBreaker,
    CircuitBreakerRegistry,
)
from efe_swarm_integration.flow_control.retry import (
    RetryClassifier,
    backoff_delay_ms,
)


def test_bulkhead_enforces_capacity() -> None:
    async def scenario() -> None:
        reg = BulkheadRegistry(capacity=1, queue_wait_ms=10)
        bh = await reg.for_tenant("t1")

        async def hold() -> None:
            async with bh:
                await asyncio.sleep(0.05)

        task = asyncio.create_task(hold())
        await asyncio.sleep(0.01)
        # Second acquire must fail with BackpressureRejected.
        with pytest.raises(BackpressureRejected):
            async with bh:
                pass
        await task

    asyncio.run(scenario())


def test_circuit_breaker_opens_on_error_burst() -> None:
    async def scenario() -> None:
        cb = CircuitBreaker(
            tenant_id="t1",
            error_rate_threshold=0.25,
            window_s=30.0,
            min_samples=4,
            open_cooldown_s=10.0,
        )
        # 4 failures straight.
        for _ in range(4):
            await cb.record(success=False)
        assert cb.state == CbState.open
        with pytest.raises(CircuitOpen):
            await cb.pre_call()

    asyncio.run(scenario())


def test_circuit_breaker_recovers_after_cooldown() -> None:
    async def scenario() -> None:
        cb = CircuitBreaker(
            tenant_id="t",
            error_rate_threshold=0.25,
            window_s=30.0,
            min_samples=1,
            open_cooldown_s=0.01,
        )
        await cb.record(success=False)  # open
        await asyncio.sleep(0.02)
        # Should move to half_open on pre_call.
        await cb.pre_call()
        assert cb.state == CbState.half_open
        await cb.record(success=True)
        assert cb.state == CbState.closed

    asyncio.run(scenario())


def test_retry_classifier() -> None:
    cfg = RetryConfig()
    c = RetryClassifier(cfg)
    assert c.should_retry(status="UNAVAILABLE", attempt=1, idempotent=False)
    assert not c.should_retry(
        status="UNAVAILABLE", attempt=cfg.max_attempts_unavailable + 1, idempotent=False
    )
    # DEADLINE_EXCEEDED retryable only when idempotent.
    assert not c.should_retry(status="DEADLINE_EXCEEDED", attempt=1, idempotent=False)
    assert c.should_retry(status="DEADLINE_EXCEEDED", attempt=1, idempotent=True)


def test_backoff_delay_monotonic_bound() -> None:
    cfg = RetryConfig()
    # The decorrelated-jitter backoff has a cap.
    values = [backoff_delay_ms(a, cfg=cfg) for a in range(1, 6)]
    assert all(v >= cfg.base_delay_ms // 2 for v in values)
    assert all(v <= cfg.max_delay_ms for v in values)
