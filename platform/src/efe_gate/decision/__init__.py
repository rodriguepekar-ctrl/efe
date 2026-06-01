"""Decision engine — 3-gate cascade."""

from __future__ import annotations

from efe_gate.decision.engine import Decision, DecisionEngine, DecisionOutcome
from efe_gate.decision.rule import (
    FairnessRuleSpec,
    MetricRuleSpec,
    PromotionRule,
    ReviewPolicy,
)

__all__ = [
    "Decision",
    "DecisionEngine",
    "DecisionOutcome",
    "FairnessRuleSpec",
    "MetricRuleSpec",
    "PromotionRule",
    "ReviewPolicy",
]
