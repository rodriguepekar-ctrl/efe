"""Metric catalogue.

Each metric has a :class:`MetricDefinition` declaring:

* ``name`` — canonical identifier (matches SQL column value).
* ``role`` — default role (can be overridden per-rule).
* ``higher_is_better`` — direction.
* ``default_test`` — statistical test used by the comparator when a
  rule doesn't specify one.
* ``task_types`` — which task the metric is usable on (tags).

Extending: call :func:`register_metric` from a side-effect module.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Literal

from efe_gate.errors import UnknownMetric
from efe_gate.metrics.bundle import MetricRole

TestName = Literal["mcnemar", "bootstrap", "paired_t", "welch_t"]


@dataclass(frozen=True, slots=True)
class MetricDefinition:
    name: str
    role: MetricRole
    higher_is_better: bool
    default_test: TestName
    task_types: frozenset[str] = field(default_factory=frozenset)
    description: str = ""


CATALOG: dict[str, MetricDefinition] = {}


def register_metric(md: MetricDefinition) -> None:
    if not md.name:
        raise ValueError("metric name must be non-empty")
    CATALOG[md.name] = md


def resolve_metric(name: str) -> MetricDefinition:
    try:
        return CATALOG[name]
    except KeyError as exc:
        raise UnknownMetric(f"unknown metric {name!r}") from exc


def _bootstrap_register() -> None:
    reg = register_metric
    # Classification
    reg(MetricDefinition(
        name="accuracy", role=MetricRole.PRIMARY,
        higher_is_better=True, default_test="mcnemar",
        task_types=frozenset({"classif"}),
        description="fraction of predictions matching the gold label",
    ))
    reg(MetricDefinition(
        name="macro_f1", role=MetricRole.PRIMARY,
        higher_is_better=True, default_test="bootstrap",
        task_types=frozenset({"classif"}),
        description="mean per-class F1, robust to class imbalance",
    ))
    reg(MetricDefinition(
        name="calibration_ece", role=MetricRole.SECONDARY,
        higher_is_better=False, default_test="bootstrap",
        task_types=frozenset({"classif"}),
        description="Expected Calibration Error",
    ))
    # Retrieval / embeddings
    reg(MetricDefinition(
        name="recall@1", role=MetricRole.PRIMARY,
        higher_is_better=True, default_test="bootstrap",
        task_types=frozenset({"retrieval"}),
    ))
    reg(MetricDefinition(
        name="recall@5", role=MetricRole.PRIMARY,
        higher_is_better=True, default_test="bootstrap",
        task_types=frozenset({"retrieval"}),
    ))
    reg(MetricDefinition(
        name="recall@10", role=MetricRole.SECONDARY,
        higher_is_better=True, default_test="bootstrap",
        task_types=frozenset({"retrieval"}),
    ))
    reg(MetricDefinition(
        name="mrr", role=MetricRole.PRIMARY,
        higher_is_better=True, default_test="bootstrap",
        task_types=frozenset({"retrieval"}),
    ))
    reg(MetricDefinition(
        name="ndcg@10", role=MetricRole.SECONDARY,
        higher_is_better=True, default_test="bootstrap",
        task_types=frozenset({"retrieval"}),
    ))
    # Distill
    reg(MetricDefinition(
        name="student_accuracy", role=MetricRole.PRIMARY,
        higher_is_better=True, default_test="mcnemar",
        task_types=frozenset({"distill"}),
    ))
    reg(MetricDefinition(
        name="agreement_with_teacher", role=MetricRole.PRIMARY,
        higher_is_better=True, default_test="mcnemar",
        task_types=frozenset({"distill"}),
    ))
    reg(MetricDefinition(
        name="kl_divergence", role=MetricRole.SECONDARY,
        higher_is_better=False, default_test="paired_t",
        task_types=frozenset({"distill"}),
    ))
    # Budgets (shared)
    reg(MetricDefinition(
        name="p99_latency_ms", role=MetricRole.BUDGET,
        higher_is_better=False, default_test="welch_t",
        task_types=frozenset({"classif", "retrieval", "distill"}),
    ))
    reg(MetricDefinition(
        name="model_size_mb", role=MetricRole.BUDGET,
        higher_is_better=False, default_test="welch_t",
        task_types=frozenset({"classif", "retrieval", "distill"}),
    ))
    reg(MetricDefinition(
        name="peak_memory_gib", role=MetricRole.BUDGET,
        higher_is_better=False, default_test="welch_t",
        task_types=frozenset({"classif", "retrieval", "distill"}),
    ))
    reg(MetricDefinition(
        name="cold_start_s", role=MetricRole.BUDGET,
        higher_is_better=False, default_test="welch_t",
        task_types=frozenset({"classif", "retrieval", "distill"}),
    ))


_bootstrap_register()


__all__ = [
    "CATALOG",
    "MetricDefinition",
    "TestName",
    "register_metric",
    "resolve_metric",
]
