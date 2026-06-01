"""Pin tests for the per-provider CircuitBreaker."""
from __future__ import annotations

import time

from efe_byo_agent.llm.circuit_breaker import CircuitBreaker, CircuitState


def test_initial_state_closed():
    cb = CircuitBreaker()
    assert cb.state is CircuitState.CLOSED
    assert cb.is_closed()


def test_records_failure_below_threshold_stays_closed():
    cb = CircuitBreaker(threshold=3)
    cb.record_failure()
    cb.record_failure()
    assert cb.is_closed()
    assert cb.state is CircuitState.CLOSED


def test_opens_after_threshold():
    cb = CircuitBreaker(threshold=3)
    for _ in range(3):
        cb.record_failure()
    assert cb.state is CircuitState.OPEN
    assert not cb.is_closed()


def test_success_resets_consecutive_failures():
    cb = CircuitBreaker(threshold=3)
    cb.record_failure()
    cb.record_failure()
    cb.record_success()
    cb.record_failure()
    cb.record_failure()
    # After success, counter restarts ; 2 failures < 3 → still closed
    assert cb.is_closed()


def test_open_to_half_open_transition_after_duration():
    cb = CircuitBreaker(threshold=2, open_duration_s=0.05)
    cb.record_failure()
    cb.record_failure()
    assert cb.state is CircuitState.OPEN
    assert not cb.is_closed()
    time.sleep(0.06)
    # First is_closed() after duration → transitions to HALF_OPEN
    assert cb.is_closed()
    assert cb.state is CircuitState.HALF_OPEN


def test_half_open_success_returns_to_closed():
    cb = CircuitBreaker(threshold=2, open_duration_s=0.05)
    cb.record_failure()
    cb.record_failure()
    time.sleep(0.06)
    cb.is_closed()  # transition to HALF_OPEN
    cb.record_success()
    assert cb.state is CircuitState.CLOSED


def test_reset_returns_to_closed():
    cb = CircuitBreaker(threshold=2)
    cb.record_failure()
    cb.record_failure()
    cb.reset()
    assert cb.state is CircuitState.CLOSED
    assert cb.consecutive_failures == 0
