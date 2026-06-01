"""Decision engine — the 3-gate cascade.

Given a :class:`MetricsBundle` for the candidate, a champion
:class:`MetricsBundle` (snapshot), and a :class:`PromotionRule`,
produce a :class:`Decision` (``PROMOTE`` | ``REJECT`` |
``HUMAN_REVIEW``) plus a structured reason trace.

Cascade (per §11 of the architecture doc):

    1. Budget caps — any breach → REJECT.
    2. Primary metrics via TOST — one FAIL → REJECT, any AMBIGUOUS
       → HUMAN_REVIEW.
    3. Fairness — per-group drop exceeds threshold → HUMAN_REVIEW.
    4. If all passed → PROMOTE.

The engine is **pure** — no DB, no transport. It is instantiated
per-request with its settings and called once.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
from typing import Any

from efe_gate.comparator import (
    ComparisonEngine,
    MetricComparison,
    MetricComparisonVerdict,
)
from efe_gate.decision.rule import PromotionRule
from efe_gate.metrics.bundle import MetricRole, MetricsBundle


class DecisionOutcome(str, Enum):
    PROMOTE = "promote"
    REJECT = "reject"
    HUMAN_REVIEW = "human_review"


@dataclass(frozen=True, slots=True)
class Decision:
    outcome: DecisionOutcome
    reason: str
    comparisons: tuple[MetricComparison, ...]
    fairness_violations: tuple[tuple[str, float], ...] = ()
    detail: dict[str, Any] = field(default_factory=dict)

    def as_dict(self) -> dict[str, Any]:
        return {
            "outcome": self.outcome.value,
            "reason": self.reason,
            "fairness_violations": [
                {"group": g, "drop": d} for g, d in self.fairness_violations
            ],
            "comparisons": [c.as_dict() for c in self.comparisons],
            "detail": dict(self.detail),
        }


@dataclass
class DecisionEngine:
    """3-gate cascade. Construct once per request."""

    comparator: ComparisonEngine

    def decide(
        self,
        *,
        champion: MetricsBundle,
        candidate: MetricsBundle,
        rule: PromotionRule,
    ) -> Decision:
        # Build per-metric overrides from the rule.
        margin_overrides: dict[str, float] = {}
        for m in rule.primary:
            if m.min_equivalence_margin is not None:
                margin_overrides[m.metric] = float(m.min_equivalence_margin)
        budget_caps: dict[str, float] = {}
        for m in rule.budget:
            if m.hard_cap is not None:
                budget_caps[m.metric] = float(m.hard_cap)

        comparisons = self.comparator.compare(
            champion=champion, candidate=candidate,
            margin_overrides=margin_overrides,
            budget_caps=budget_caps,
        )
        by_name = {c.metric_name: c for c in comparisons}

        # --- Gate 1 — Budgets --------------------------------------
        for b in rule.budget:
            cmp = by_name.get(b.metric)
            if cmp is None:
                continue
            if cmp.verdict is MetricComparisonVerdict.BUDGET_EXCEEDED:
                return Decision(
                    outcome=DecisionOutcome.REJECT,
                    reason=f"budget:{b.metric} breach "
                           f"(value={cmp.candidate_value}, "
                           f"cap={cmp.detail.get('cap')})",
                    comparisons=tuple(comparisons),
                    detail={"gate": "budget", "metric": b.metric},
                )
            if (
                cmp.role is MetricRole.BUDGET
                and cmp.verdict is MetricComparisonVerdict.FAIL
            ):
                return Decision(
                    outcome=DecisionOutcome.REJECT,
                    reason=f"budget:{b.metric} drift vs champion",
                    comparisons=tuple(comparisons),
                    detail={"gate": "budget", "metric": b.metric},
                )

        # --- Gate 2 — Primary metrics ------------------------------
        ambiguous: list[str] = []
        for p in rule.primary:
            cmp = by_name.get(p.metric)
            if cmp is None:
                return Decision(
                    outcome=DecisionOutcome.HUMAN_REVIEW,
                    reason=f"primary:{p.metric} not measured",
                    comparisons=tuple(comparisons),
                    detail={"gate": "primary", "metric": p.metric},
                )
            if cmp.verdict is MetricComparisonVerdict.FAIL:
                return Decision(
                    outcome=DecisionOutcome.REJECT,
                    reason=f"primary:{p.metric} regression proven",
                    comparisons=tuple(comparisons),
                    detail={"gate": "primary", "metric": p.metric},
                )
            if cmp.verdict is MetricComparisonVerdict.AMBIGUOUS:
                ambiguous.append(p.metric)

        if ambiguous:
            return Decision(
                outcome=DecisionOutcome.HUMAN_REVIEW,
                reason=(
                    "primary metrics ambiguous: "
                    + ", ".join(ambiguous)
                ),
                comparisons=tuple(comparisons),
                detail={"gate": "primary", "ambiguous": ambiguous},
            )

        # --- Gate 3 — Fairness -------------------------------------
        if rule.fairness is not None:
            violations: list[tuple[str, float]] = []
            metric = rule.fairness.metric
            max_drop = rule.fairness.max_per_group_drop
            min_samples = rule.fairness.min_samples_per_group
            groups_to_check = (
                list(rule.fairness.groups)
                if rule.fairness.groups
                else sorted(
                    set(candidate.by_group) & set(champion.by_group)
                )
            )
            for g in groups_to_check:
                cand_g = candidate.by_group.get(g, {}).get(metric)
                champ_g = champion.by_group.get(g, {}).get(metric)
                if cand_g is None or champ_g is None:
                    continue
                if cand_g.sample_size < min_samples:
                    continue
                drop = champ_g.value - cand_g.value
                if drop > max_drop:
                    violations.append((g, float(drop)))
            if violations:
                return Decision(
                    outcome=DecisionOutcome.HUMAN_REVIEW,
                    reason=(
                        f"fairness:{metric} per-group drop exceeded"
                    ),
                    comparisons=tuple(comparisons),
                    fairness_violations=tuple(violations),
                    detail={"gate": "fairness"},
                )

        # --- Pass ---
        return Decision(
            outcome=DecisionOutcome.PROMOTE,
            reason="all gates passed",
            comparisons=tuple(comparisons),
        )


__all__ = [
    "Decision",
    "DecisionEngine",
    "DecisionOutcome",
]
