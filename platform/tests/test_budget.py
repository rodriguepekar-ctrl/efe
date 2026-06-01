"""Tests for :class:`Budget` + :class:`BudgetLedger`."""

from __future__ import annotations

import time

import pytest

from efe_platform.bot.budget import Budget, BudgetLedger
from efe_platform.errors import BudgetExceeded


def test_defaults() -> None:
    b = Budget()
    assert b.max_wall_clock_ms > 0
    assert b.max_llm_tokens > 0
    assert b.max_usd >= 0
    assert b.max_tool_calls > 0


def test_rejects_nonpositive() -> None:
    with pytest.raises(ValueError):
        Budget(max_wall_clock_ms=0)
    with pytest.raises(ValueError):
        Budget(max_tool_calls=0)


def test_relaxed_by_multiplies_all_ceilings() -> None:
    b = Budget(
        max_wall_clock_ms=1000, max_llm_tokens=100, max_usd=0.1, max_tool_calls=4
    )
    b2 = b.relaxed_by(2.0)
    assert b2.max_wall_clock_ms == 2000
    assert b2.max_llm_tokens == 200
    assert b2.max_usd == pytest.approx(0.2)
    assert b2.max_tool_calls == 8


def test_ledger_charges_llm() -> None:
    led = BudgetLedger(Budget(max_llm_tokens=100, max_usd=0.5))
    led.charge_llm(tokens=30, usd=0.1)
    assert led.tokens_consumed == 30
    assert led.usd_consumed == pytest.approx(0.1)
    assert led.remaining_tokens() == 70


def test_ledger_raises_on_token_exhaustion() -> None:
    led = BudgetLedger(Budget(max_llm_tokens=50, max_usd=10.0))
    led.charge_llm(tokens=40, usd=0)
    with pytest.raises(BudgetExceeded, match="llm tokens"):
        led.charge_llm(tokens=20, usd=0)


def test_ledger_raises_on_usd_exhaustion() -> None:
    led = BudgetLedger(Budget(max_llm_tokens=10_000, max_usd=0.05))
    with pytest.raises(BudgetExceeded, match="llm usd"):
        led.charge_llm(tokens=1, usd=0.1)


def test_ledger_tool_calls_counted() -> None:
    led = BudgetLedger(Budget(max_tool_calls=2))
    led.charge_tool_call()
    led.charge_tool_call()
    with pytest.raises(BudgetExceeded, match="tool calls"):
        led.charge_tool_call()


def test_ledger_wall_clock_detected() -> None:
    # Construct with a known start, then mutate the instance attribute to
    # simulate time passing. Cleaner than monkeypatching the stdlib.
    led = BudgetLedger(Budget(max_wall_clock_ms=100))
    led.start_monotonic = time.monotonic() - 1.0  # 1s ago → exceeds 100ms cap
    with pytest.raises(BudgetExceeded, match="wall clock"):
        led.check_wall_clock()


def test_ledger_to_dict_shape() -> None:
    led = BudgetLedger(Budget())
    led.charge_llm(tokens=5, usd=0.0)
    led.charge_tool_call()
    d = led.to_dict()
    assert d["tokens_consumed"] == 5
    assert d["tool_calls_made"] == 1
    assert d["remaining_tokens"] > 0


def test_ledger_rejects_negative() -> None:
    led = BudgetLedger(Budget())
    with pytest.raises(ValueError):
        led.charge_llm(tokens=-1, usd=0.0)
