"""Tests for :class:`PolicyEngine` — plan + output guardrails."""

from __future__ import annotations

import pytest

from efe_platform.bot.budget import Budget, BudgetLedger
from efe_platform.bot.planner import PlannedCall
from efe_platform.bot.policy import PolicyEngine, PolicyViolation
from efe_platform.bot.prompts import SYSTEM_PLAN_FR
from efe_platform.bot.tool_registry import ToolRegistry, ToolSchema


def _registry_with(*names: str) -> ToolRegistry:
    r = ToolRegistry()
    for n in names:
        schema = ToolSchema(
            name=n,
            description=f"{n} test tool",
            input_schema={
                "type": "object",
                "properties": {"tenant": {"type": "string"}},
                "required": [],
            },
        )

        async def _ok(_args: dict) -> dict:  # type: ignore[no-untyped-def]
            return {"ok": True}

        r.register(schema, _ok)
    return r


# ---------------------------------------------------------------------------
#  Plan guardrails
# ---------------------------------------------------------------------------


def test_accepts_empty_plan() -> None:
    eng = PolicyEngine()
    results = eng.check_plan(
        planned_calls=(),
        registry=_registry_with("swarm.classify"),
        ledger=BudgetLedger(Budget()),
        expected_tenant="acme",
    )
    # No violation ≠ zero guardrails checked — we still ran three.
    assert {r.name for r in results} >= {"tool_whitelist", "budget_fit", "tenant_propagation"}
    assert all(r.passed for r in results)


def test_unknown_tool_rejected() -> None:
    eng = PolicyEngine()
    with pytest.raises(PolicyViolation, match="unknown tools"):
        eng.check_plan(
            planned_calls=(
                PlannedCall(tool_use_id="u", tool_name="ghost", arguments={}),
            ),
            registry=_registry_with("swarm.classify"),
            ledger=BudgetLedger(Budget()),
            expected_tenant="acme",
        )


def test_budget_fit_rejected() -> None:
    eng = PolicyEngine()
    # Cap 1 tool call, plan has 2.
    with pytest.raises(PolicyViolation, match="only 1 remain"):
        eng.check_plan(
            planned_calls=(
                PlannedCall(tool_use_id="u1", tool_name="t", arguments={}),
                PlannedCall(tool_use_id="u2", tool_name="t", arguments={}),
            ),
            registry=_registry_with("t"),
            ledger=BudgetLedger(Budget(max_tool_calls=1)),
            expected_tenant="acme",
        )


def test_tenant_mismatch_rejected() -> None:
    eng = PolicyEngine()
    with pytest.raises(PolicyViolation, match="tenant mismatch"):
        eng.check_plan(
            planned_calls=(
                PlannedCall(
                    tool_use_id="u",
                    tool_name="t",
                    arguments={"tenant": "other"},
                ),
            ),
            registry=_registry_with("t"),
            ledger=BudgetLedger(Budget()),
            expected_tenant="acme",
        )


def test_no_tenant_arg_is_accepted() -> None:
    """Tools without a tenant parameter in their call must not fail."""
    eng = PolicyEngine()
    results = eng.check_plan(
        planned_calls=(
            PlannedCall(tool_use_id="u", tool_name="t", arguments={"other": 1}),
        ),
        registry=_registry_with("t"),
        ledger=BudgetLedger(Budget()),
        expected_tenant="acme",
    )
    assert all(r.passed for r in results)


# ---------------------------------------------------------------------------
#  Output guardrails
# ---------------------------------------------------------------------------


def test_output_length_enforced() -> None:
    eng = PolicyEngine(max_output_chars=100)
    with pytest.raises(PolicyViolation, match="exceeds"):
        eng.check_output("x" * 500)


def test_prompt_leakage_detected() -> None:
    eng = PolicyEngine()
    # Echo a prefix of the system plan prompt — should trip the guard.
    leaked = SYSTEM_PLAN_FR[:60] + "... reste"
    with pytest.raises(PolicyViolation, match="leak"):
        eng.check_output(leaked)


def test_normal_output_passes() -> None:
    eng = PolicyEngine()
    results = eng.check_output("Réponse utilisateur normale, courte et sans fuite.")
    assert all(r.passed for r in results)
