"""Per-request budget enforcement.

Each call to :meth:`Bot.answer` is scoped by a :class:`Budget`. The bot
threads a :class:`BudgetLedger` through the planner / tool registry,
which increments counters as work is performed. Any increment that
would breach the budget raises :class:`BudgetExceeded` — which the bot
catches, records in the audit trail, and reports to the caller.

The design is intentionally simple: one ledger per request, no cross-
request leakage, and no shared mutable state.
"""

from __future__ import annotations

import time
from dataclasses import dataclass, field, replace

from efe_platform.errors import BudgetExceeded

# ---------------------------------------------------------------------------
#  Budget (immutable)
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class Budget:
    """Hard per-request ceilings."""

    max_wall_clock_ms: int = 30_000
    max_llm_tokens: int = 16_000
    max_usd: float = 0.25
    max_tool_calls: int = 8

    def __post_init__(self) -> None:
        if self.max_wall_clock_ms <= 0:
            raise ValueError("max_wall_clock_ms must be > 0")
        if self.max_llm_tokens < 0:
            raise ValueError("max_llm_tokens must be >= 0")
        if self.max_usd < 0:
            raise ValueError("max_usd must be >= 0")
        if self.max_tool_calls <= 0:
            raise ValueError("max_tool_calls must be > 0")

    def relaxed_by(self, factor: float) -> Budget:
        """Return a new budget with all ceilings multiplied by ``factor``."""
        if factor <= 0:
            raise ValueError("factor must be > 0")
        return replace(
            self,
            max_wall_clock_ms=int(self.max_wall_clock_ms * factor),
            max_llm_tokens=int(self.max_llm_tokens * factor),
            max_usd=self.max_usd * factor,
            max_tool_calls=int(self.max_tool_calls * factor),
        )


# ---------------------------------------------------------------------------
#  Ledger (mutable, scoped to one request)
# ---------------------------------------------------------------------------


@dataclass
class BudgetLedger:
    """Mutable running tally for a single request.

    The ledger is created by the bot and passed to callees. Callees
    must not hold a reference after the request completes.
    """

    budget: Budget
    start_monotonic: float = field(default_factory=time.monotonic)
    tokens_consumed: int = 0
    usd_consumed: float = 0.0
    tool_calls_made: int = 0

    # ------------------------------------------------------------ Queries

    @property
    def elapsed_ms(self) -> int:
        return int((time.monotonic() - self.start_monotonic) * 1000)

    def remaining_ms(self) -> int:
        return max(0, self.budget.max_wall_clock_ms - self.elapsed_ms)

    def remaining_tokens(self) -> int:
        return max(0, self.budget.max_llm_tokens - self.tokens_consumed)

    def remaining_usd(self) -> float:
        return max(0.0, self.budget.max_usd - self.usd_consumed)

    def remaining_tool_calls(self) -> int:
        return max(0, self.budget.max_tool_calls - self.tool_calls_made)

    def to_dict(self) -> dict[str, float | int]:
        return {
            "elapsed_ms": self.elapsed_ms,
            "tokens_consumed": self.tokens_consumed,
            "usd_consumed": round(self.usd_consumed, 6),
            "tool_calls_made": self.tool_calls_made,
            "remaining_ms": self.remaining_ms(),
            "remaining_tokens": self.remaining_tokens(),
            "remaining_usd": round(self.remaining_usd(), 6),
            "remaining_tool_calls": self.remaining_tool_calls(),
        }

    # -------------------------------------------------------- Mutations

    def charge_llm(self, *, tokens: int, usd: float) -> None:
        """Record LLM consumption; raise if it breaches the budget."""
        if tokens < 0 or usd < 0:
            raise ValueError("LLM usage must be non-negative")
        new_tokens = self.tokens_consumed + tokens
        new_usd = self.usd_consumed + usd
        if new_tokens > self.budget.max_llm_tokens:
            raise BudgetExceeded(
                f"llm tokens budget exceeded: {new_tokens} > {self.budget.max_llm_tokens}",
                detail={"dimension": "tokens", "attempted": new_tokens},
            )
        if new_usd > self.budget.max_usd:
            raise BudgetExceeded(
                f"llm usd budget exceeded: ${new_usd:.4f} > ${self.budget.max_usd:.4f}",
                detail={"dimension": "usd", "attempted": new_usd},
            )
        self._check_wall_clock()
        self.tokens_consumed = new_tokens
        self.usd_consumed = new_usd

    def charge_tool_call(self) -> None:
        """Record one tool invocation; raise if the call count is breached."""
        new_count = self.tool_calls_made + 1
        if new_count > self.budget.max_tool_calls:
            raise BudgetExceeded(
                f"tool calls budget exceeded: {new_count} > {self.budget.max_tool_calls}",
                detail={"dimension": "tool_calls", "attempted": new_count},
            )
        self._check_wall_clock()
        self.tool_calls_made = new_count

    def check_wall_clock(self) -> None:
        """Public checkpoint — useful between async phases."""
        self._check_wall_clock()

    # -------------------------------------------------------- Internals

    def _check_wall_clock(self) -> None:
        if self.elapsed_ms > self.budget.max_wall_clock_ms:
            raise BudgetExceeded(
                f"wall clock budget exceeded: {self.elapsed_ms}ms > "
                f"{self.budget.max_wall_clock_ms}ms",
                detail={"dimension": "wall_clock_ms", "attempted": self.elapsed_ms},
            )


__all__ = ["Budget", "BudgetExceeded", "BudgetLedger"]
