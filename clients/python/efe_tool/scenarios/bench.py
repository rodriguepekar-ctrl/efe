"""Golden-case bench harness for scenarios.

Iterates a scenario's ``golden.json``, runs each case through
:func:`run_scenario`, and aggregates accuracy + cost + latency.

Usage (programmatic)::

    from efe_tool.scenarios import load_scenario, bench_scenario
    s = load_scenario("support_triage_bilingual")
    report = bench_scenario(s, n=1, anthropic_client=my_mock)
    print(report.accuracy, report.total_cost_usd)

Usage (CLI)::

    efe-admin scenario bench support_triage_bilingual --n 1

The bench is designed to be driven by a **mock** Anthropic client by
default in tests; the CLI wires the real one when the user opts in.
"""

from __future__ import annotations

import time
from dataclasses import dataclass, field
from typing import Any, Callable, Mapping, Optional

from ..bridges.claude import DEFAULT_MAX_ROUNDS, DEFAULT_MAX_TOKENS
from ..bridges.errors import LlmError
from .loader import GoldenCase, Scenario
from .runner import run_scenario


# ---------------------------------------------------------------------------
#  Report dataclasses
# ---------------------------------------------------------------------------


@dataclass
class BenchCaseOutcome:
    """One case's outcome: predicted label, latency, cost, pass/fail."""

    user: str
    expected_label: Optional[str]
    predicted_label: Optional[str]
    correct: bool
    rounds: int
    input_tokens: int
    output_tokens: int
    cost_usd: float
    wall_clock_ms: int
    error_code: Optional[str] = None
    error_message: Optional[str] = None


@dataclass
class BenchReport:
    """Aggregate bench report."""

    scenario: str
    model: str
    cases: list[BenchCaseOutcome] = field(default_factory=list)
    total_cost_usd: float = 0.0
    total_wall_clock_ms: int = 0

    @property
    def accuracy(self) -> Optional[float]:
        """Fraction of cases with ``correct == True``. None if no
        expected_label is defined on any case."""
        graded = [c for c in self.cases if c.expected_label is not None
                  and c.error_code is None]
        if not graded:
            return None
        hits = sum(1 for c in graded if c.correct)
        return round(hits / len(graded), 4)

    @property
    def passed(self) -> int:
        return sum(1 for c in self.cases if c.correct)

    @property
    def failed(self) -> int:
        return sum(1 for c in self.cases
                   if not c.correct and c.expected_label is not None)

    @property
    def errored(self) -> int:
        return sum(1 for c in self.cases if c.error_code is not None)

    def as_dict(self) -> dict[str, Any]:
        return {
            "scenario": self.scenario,
            "model": self.model,
            "case_count": len(self.cases),
            "passed": self.passed,
            "failed": self.failed,
            "errored": self.errored,
            "accuracy": self.accuracy,
            "total_cost_usd": round(self.total_cost_usd, 6),
            "total_wall_clock_ms": self.total_wall_clock_ms,
            "cases": [_outcome_as_dict(c) for c in self.cases],
        }

    def summary_line(self) -> str:
        acc = self.accuracy
        acc_str = f"{acc:.1%}" if acc is not None else "n/a"
        return (
            f"{self.scenario}: {self.passed}/{len(self.cases)} passed, "
            f"accuracy {acc_str}, cost ${self.total_cost_usd:.4f}, "
            f"{self.total_wall_clock_ms} ms"
        )


def _outcome_as_dict(c: BenchCaseOutcome) -> dict[str, Any]:
    return {
        "user": c.user,
        "expected_label": c.expected_label,
        "predicted_label": c.predicted_label,
        "correct": c.correct,
        "rounds": c.rounds,
        "input_tokens": c.input_tokens,
        "output_tokens": c.output_tokens,
        "cost_usd": round(c.cost_usd, 6),
        "wall_clock_ms": c.wall_clock_ms,
        "error_code": c.error_code,
        "error_message": c.error_message,
    }


# ---------------------------------------------------------------------------
#  Bench harness
# ---------------------------------------------------------------------------


def bench_scenario(
    scenario: Scenario,
    *,
    anthropic_client: Any,
    efe_client: Any,
    catalogs: Mapping[str, Any],
    model: Optional[str] = None,
    max_rounds: int = DEFAULT_MAX_ROUNDS,
    max_tokens: int = DEFAULT_MAX_TOKENS,
    cost_ceiling_usd: Optional[float] = None,
    on_event: Optional[Callable[[dict[str, Any]], None]] = None,
    stop_on_error: bool = False,
) -> BenchReport:
    """Run every golden case of ``scenario`` and return a report.

    The caller provides the Anthropic client (real or mock), the EFE
    client, and the preloaded catalog map. This separation lets tests
    script the Anthropic side deterministically via :class:`MockAnthropic`.
    """
    report = BenchReport(
        scenario=scenario.name,
        model=model or scenario.prompt.model,
    )
    for case in scenario.golden:
        outcome = _run_one_case(
            scenario, case,
            anthropic_client=anthropic_client,
            efe_client=efe_client,
            catalogs=catalogs,
            model=model,
            max_rounds=case.max_rounds or max_rounds,
            max_tokens=max_tokens,
            cost_ceiling_usd=cost_ceiling_usd,
            on_event=on_event,
        )
        report.cases.append(outcome)
        report.total_cost_usd += outcome.cost_usd
        report.total_wall_clock_ms += outcome.wall_clock_ms
        if stop_on_error and outcome.error_code is not None:
            break
    return report


def _run_one_case(
    scenario: Scenario,
    case: GoldenCase,
    *,
    anthropic_client: Any,
    efe_client: Any,
    catalogs: Mapping[str, Any],
    model: Optional[str],
    max_rounds: int,
    max_tokens: int,
    cost_ceiling_usd: Optional[float],
    on_event: Optional[Callable[[dict[str, Any]], None]],
) -> BenchCaseOutcome:
    t0 = time.perf_counter()
    try:
        run = run_scenario(
            scenario,
            user=case.user,
            anthropic_client=anthropic_client,
            efe_client=efe_client,
            catalogs=catalogs,
            model=model,
            max_rounds=max_rounds,
            max_tokens=max_tokens,
            cost_ceiling_usd=cost_ceiling_usd,
            on_event=on_event,
        )
    except LlmError as e:
        wall = int((time.perf_counter() - t0) * 1000)
        return BenchCaseOutcome(
            user=case.user,
            expected_label=case.expected_label,
            predicted_label=None,
            correct=False,
            rounds=0,
            input_tokens=0,
            output_tokens=0,
            cost_usd=0.0,
            wall_clock_ms=wall,
            error_code=e.error_code,
            error_message=e.message,
        )

    conv = run.conversation
    predicted = _predicted_label(conv)
    correct = (
        case.expected_label is not None
        and predicted is not None
        and predicted == case.expected_label
    )
    return BenchCaseOutcome(
        user=case.user,
        expected_label=case.expected_label,
        predicted_label=predicted,
        correct=correct,
        rounds=conv.stats.rounds,
        input_tokens=conv.stats.input_tokens,
        output_tokens=conv.stats.output_tokens,
        cost_usd=conv.stats.cost_usd,
        wall_clock_ms=conv.wall_clock_ms,
    )


def _predicted_label(conversation: Any) -> Optional[str]:
    """Return the label from the LAST tool call (the one Claude acted
    on). Returns ``None`` if no tool call succeeded."""
    for call in reversed(getattr(conversation, "tool_calls", []) or []):
        if not call.is_error and call.label:
            return str(call.label)
    return None


__all__ = [
    "BenchCaseOutcome",
    "BenchReport",
    "bench_scenario",
]
