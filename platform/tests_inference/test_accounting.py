"""Token counter + cost calculator + budget enforcer."""

from __future__ import annotations

from decimal import Decimal

import pytest

from efe_inference.accounting.budget import (
    BudgetEnforcer,
    InMemoryBudgetStore,
)
from efe_inference.accounting.cost import CostCalculator, CostRate
from efe_inference.accounting.tokenizer import (
    ApproxTokenCounter,
    TokenCounterRegistry,
)
from efe_inference.clock import FakeClock
from efe_inference.errors import BudgetExceeded, TokenizerUnavailable


def test_approx_counter_counts_words():
    c = ApproxTokenCounter()
    assert c.count("") == 0
    assert c.count("hello world") >= 2
    assert c.count("hello world" * 10) > c.count("hello world")


def test_token_registry_raises_on_unknown():
    reg = TokenCounterRegistry.default()
    with pytest.raises(TokenizerUnavailable):
        reg.count("ghost", "text")


def test_cost_calculator_exact():
    calc = CostCalculator((
        CostRate(
            backend_name="claude",
            prompt_usd_per_1k=Decimal("3"),
            completion_usd_per_1k=Decimal("15"),
        ),
    ))
    out = calc.calculate(
        "claude", prompt_tokens=1000, completion_tokens=200,
    )
    assert out == Decimal("6")


def test_cost_calculator_zero_unknown_backend():
    calc = CostCalculator()
    assert calc.calculate(
        "unknown", prompt_tokens=100, completion_tokens=100,
    ) == Decimal(0)


@pytest.mark.asyncio
async def test_budget_blocks_over_cap():
    store = InMemoryBudgetStore()
    clock = FakeClock()
    enforcer = BudgetEnforcer(
        store=store,
        default_daily_budget_usd=Decimal("1"),
        clock=clock,
    )
    await enforcer.record("t", Decimal("2"))
    with pytest.raises(BudgetExceeded):
        await enforcer.preflight("t")


@pytest.mark.asyncio
async def test_budget_independent_per_tenant():
    store = InMemoryBudgetStore()
    clock = FakeClock()
    enforcer = BudgetEnforcer(
        store=store,
        default_daily_budget_usd=Decimal("1"),
        clock=clock,
    )
    await enforcer.record("t-a", Decimal("5"))
    # Tenant b is untouched.
    await enforcer.preflight("t-b")
