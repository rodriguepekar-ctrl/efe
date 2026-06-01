"""Guardrails applied before executing a plan and before returning output.

The policy engine is the **one-way gate** between the LLM and side
effects. Every plan goes through :meth:`PolicyEngine.check_plan` before
any tool is invoked; every final answer goes through
:meth:`PolicyEngine.check_output`.

The MVP guardrails:

* **Tool whitelist** — the plan can only reference tools registered
  with the current :class:`ToolRegistry`.

* **Budget awareness** — the plan must fit under the remaining tool-call
  budget (refuse plans with N calls when only M < N remain).

* **Tenant propagation** — every planned call's ``tenant`` argument,
  if present, must equal the caller-supplied tenant.

* **Output sanity** — the final answer is checked for obvious prompt
  leakage (raw system prompt prefix reflected back) and for reasonable
  length.

Policy failures are **never silent** — they raise
:class:`PolicyViolation` with a machine-readable ``code`` so the bot
can decide whether to retry, fallback, or surface an error.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import TYPE_CHECKING

from efe_platform.bot.prompts import SYSTEM_PLAN_EN, SYSTEM_PLAN_FR
from efe_platform.errors import BotError

if TYPE_CHECKING:
    from efe_platform.bot.budget import BudgetLedger
    from efe_platform.bot.planner import PlannedCall
    from efe_platform.bot.tool_registry import ToolRegistry


# ---------------------------------------------------------------------------
#  Exception
# ---------------------------------------------------------------------------


class PolicyViolation(BotError):
    """Guardrail refused to let the plan / output through."""

    code = "policy_violation"


# ---------------------------------------------------------------------------
#  Guardrail result
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class GuardrailResult:
    """Outcome of one guardrail check."""

    name: str
    passed: bool
    reason: str = ""

    def to_dict(self) -> dict[str, object]:
        return {"name": self.name, "passed": self.passed, "reason": self.reason}


# ---------------------------------------------------------------------------
#  Policy engine
# ---------------------------------------------------------------------------


_SYSTEM_PROMPT_PREFIXES: tuple[str, ...] = (
    SYSTEM_PLAN_FR[:40],
    SYSTEM_PLAN_EN[:40],
)


@dataclass(frozen=True, slots=True)
class PolicyEngine:
    """Stateless policy evaluator."""

    max_output_chars: int = 10_000

    # ---------------------------------------------------------- Plan phase

    def check_plan(
        self,
        *,
        planned_calls: tuple[PlannedCall, ...],
        registry: ToolRegistry,
        ledger: BudgetLedger,
        expected_tenant: str,
    ) -> tuple[GuardrailResult, ...]:
        """Evaluate all plan-phase guardrails; raise if any fails."""
        results: list[GuardrailResult] = []

        # 1. Tool whitelist.
        unknown = [c for c in planned_calls if c.tool_name not in registry]
        if unknown:
            bad = ", ".join(sorted({c.tool_name for c in unknown}))
            raise PolicyViolation(
                f"plan references unknown tools: {bad}",
                detail={"unknown_tools": sorted({c.tool_name for c in unknown})},
            )
        results.append(GuardrailResult(name="tool_whitelist", passed=True))

        # 2. Budget fit.
        if len(planned_calls) > ledger.remaining_tool_calls():
            raise PolicyViolation(
                f"plan has {len(planned_calls)} calls but only "
                f"{ledger.remaining_tool_calls()} remain",
                detail={
                    "planned": len(planned_calls),
                    "remaining": ledger.remaining_tool_calls(),
                },
            )
        results.append(GuardrailResult(name="budget_fit", passed=True))

        # 3. Tenant propagation.
        for call in planned_calls:
            tenant_arg = call.arguments.get("tenant")
            if tenant_arg is not None and tenant_arg != expected_tenant:
                raise PolicyViolation(
                    f"tool call tenant mismatch: {tenant_arg!r} != "
                    f"{expected_tenant!r}",
                    detail={"tool": call.tool_name, "tenant": tenant_arg},
                )
        results.append(GuardrailResult(name="tenant_propagation", passed=True))

        return tuple(results)

    # --------------------------------------------------------- Output phase

    def check_output(self, text: str) -> tuple[GuardrailResult, ...]:
        """Evaluate final-output guardrails; raise if any fails."""
        results: list[GuardrailResult] = []

        # 1. Length.
        if len(text) > self.max_output_chars:
            raise PolicyViolation(
                f"output exceeds {self.max_output_chars} chars",
                detail={"length": len(text)},
            )
        results.append(GuardrailResult(name="output_length", passed=True))

        # 2. Prompt leakage.
        for prefix in _SYSTEM_PROMPT_PREFIXES:
            if prefix in text:
                raise PolicyViolation(
                    "final output appears to leak the system prompt",
                    detail={"matched_prefix": prefix[:20]},
                )
        results.append(GuardrailResult(name="prompt_leakage", passed=True))

        return tuple(results)


__all__ = ["GuardrailResult", "PolicyEngine", "PolicyViolation"]
