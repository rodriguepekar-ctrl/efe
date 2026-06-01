"""Eval-job runner — build MetricsBundle from (candidate, champion, golden)."""

from __future__ import annotations

from efe_gate.runner.eval_job import (
    EvalInputs,
    EvalScorer,
    FakeScorer,
    run_evaluation,
)

__all__ = [
    "EvalInputs",
    "EvalScorer",
    "FakeScorer",
    "run_evaluation",
]
