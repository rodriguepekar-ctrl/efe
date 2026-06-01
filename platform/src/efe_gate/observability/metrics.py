"""Prometheus metrics — gate-side catalogue.

Metrics (M05 §15.2):

    efe_gate_eval_total                Counter(tenant, task, decision)
    efe_gate_eval_duration_seconds     Histogram(tenant, task)
    efe_gate_primary_metric_delta      Gauge(tenant, task, metric)
    efe_gate_budget_violations_total   Counter(tenant, task, budget)
    efe_gate_human_review_queue_size   Gauge(tenant)
    efe_gate_champion_changed_total    Counter(tenant, task)
    efe_gate_rollback_total            Counter(tenant, task, reason)
    efe_gate_audit_chain_integrity     Gauge(tenant)
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Protocol


class _Metric(Protocol):
    def inc(self, amount: float = 1.0, **labels: str) -> None:  # pragma: no cover
        ...

    def observe(self, value: float, **labels: str) -> None:  # pragma: no cover
        ...

    def set(self, value: float, **labels: str) -> None:  # pragma: no cover
        ...


@dataclass
class _NoopMetric:
    name: str
    values: list[tuple[str, float, dict[str, str]]] = field(
        default_factory=list,
    )

    def inc(self, amount: float = 1.0, **labels: str) -> None:
        self.values.append(("inc", float(amount), dict(labels)))

    def observe(self, value: float, **labels: str) -> None:
        self.values.append(("observe", float(value), dict(labels)))

    def set(self, value: float, **labels: str) -> None:
        self.values.append(("set", float(value), dict(labels)))


@dataclass
class MetricsRegistry:
    eval_total: _Metric
    eval_duration_seconds: _Metric
    primary_metric_delta: _Metric
    budget_violations_total: _Metric
    human_review_queue_size: _Metric
    champion_changed_total: _Metric
    rollback_total: _Metric
    audit_chain_integrity: _Metric

    def snapshot(self) -> dict[str, Any]:
        out: dict[str, Any] = {}
        for name in self.__dataclass_fields__:
            m = getattr(self, name)
            if isinstance(m, _NoopMetric):
                out[m.name] = list(m.values)
        return out


def noop_registry() -> MetricsRegistry:
    return MetricsRegistry(
        eval_total=_NoopMetric("efe_gate_eval_total"),
        eval_duration_seconds=_NoopMetric(
            "efe_gate_eval_duration_seconds",
        ),
        primary_metric_delta=_NoopMetric(
            "efe_gate_primary_metric_delta",
        ),
        budget_violations_total=_NoopMetric(
            "efe_gate_budget_violations_total",
        ),
        human_review_queue_size=_NoopMetric(
            "efe_gate_human_review_queue_size",
        ),
        champion_changed_total=_NoopMetric(
            "efe_gate_champion_changed_total",
        ),
        rollback_total=_NoopMetric("efe_gate_rollback_total"),
        audit_chain_integrity=_NoopMetric(
            "efe_gate_audit_chain_integrity",
        ),
    )


def build_registry(namespace: str = "efe_gate") -> MetricsRegistry:
    try:
        from prometheus_client import Counter, Gauge, Histogram
    except ImportError:
        return noop_registry()
    return MetricsRegistry(
        eval_total=Counter(
            f"{namespace}_eval_total", "Evaluations by decision.",
            labelnames=("tenant", "task", "decision"),
        ),
        eval_duration_seconds=Histogram(
            f"{namespace}_eval_duration_seconds",
            "End-to-end evaluation latency.",
            labelnames=("tenant", "task"),
            buckets=(10, 30, 60, 120, 300, 600, 1_800, 3_600),
        ),
        primary_metric_delta=Gauge(
            f"{namespace}_primary_metric_delta",
            "Candidate − Champion Δ.",
            labelnames=("tenant", "task", "metric"),
        ),
        budget_violations_total=Counter(
            f"{namespace}_budget_violations_total",
            "Budget cap breaches by budget.",
            labelnames=("tenant", "task", "budget"),
        ),
        human_review_queue_size=Gauge(
            f"{namespace}_human_review_queue_size",
            "Pending reviews by tenant.",
            labelnames=("tenant",),
        ),
        champion_changed_total=Counter(
            f"{namespace}_champion_changed_total",
            "Champion promotions by (tenant, task).",
            labelnames=("tenant", "task"),
        ),
        rollback_total=Counter(
            f"{namespace}_rollback_total",
            "Rollbacks performed.",
            labelnames=("tenant", "task", "reason"),
        ),
        audit_chain_integrity=Gauge(
            f"{namespace}_audit_chain_integrity",
            "1 = intact, 0 = broken.",
            labelnames=("tenant",),
        ),
    )


__all__ = ["MetricsRegistry", "build_registry", "noop_registry"]
