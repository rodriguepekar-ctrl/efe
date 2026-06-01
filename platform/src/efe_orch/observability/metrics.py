"""Prometheus metrics — orchestrator-wide counters, histograms, gauges.

We wrap the underlying ``prometheus_client`` types inside
:class:`MetricsRegistry` so:

* the rest of the service never imports ``prometheus_client`` directly,
* when the optional dep is missing we fall back to :func:`noop_registry`
  (which returns ``_NoopMetric`` instances that support the same
  method surface),
* tests can poke at :meth:`MetricsRegistry.snapshot` to assert on
  emitted values without scraping the HTTP endpoint.

Metrics catalogue (every one carries a ``pool`` label where sensible):

* ``efe_orch_runs_submitted_total``       Counter(tenant, pool)
* ``efe_orch_runs_transitioned_total``    Counter(from_state, to_state)
* ``efe_orch_run_duration_seconds``       Histogram(pool)
* ``efe_orch_runs_in_state``              Gauge(state)
* ``efe_orch_reaper_cycles_total``        Counter(outcome)
* ``efe_orch_retry_decisions_total``      Counter(error_class, retry)
* ``efe_orch_k8s_submit_total``           Counter(outcome)
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
    values: list[tuple[str, float, dict[str, str]]] = field(default_factory=list)

    def inc(self, amount: float = 1.0, **labels: str) -> None:
        self.values.append(("inc", float(amount), dict(labels)))

    def observe(self, value: float, **labels: str) -> None:
        self.values.append(("observe", float(value), dict(labels)))

    def set(self, value: float, **labels: str) -> None:
        self.values.append(("set", float(value), dict(labels)))


@dataclass
class MetricsRegistry:
    """In-process registry. Wraps prometheus_client when available."""

    runs_submitted: _Metric
    runs_transitioned: _Metric
    run_duration_seconds: _Metric
    runs_in_state: _Metric
    reaper_cycles: _Metric
    retry_decisions: _Metric
    k8s_submit: _Metric

    def snapshot(self) -> dict[str, Any]:
        """Best-effort machine-readable view — only meaningful for the noop impl."""
        out: dict[str, Any] = {}
        for name in self.__dataclass_fields__:
            m = getattr(self, name)
            if isinstance(m, _NoopMetric):
                out[m.name] = list(m.values)
        return out


def noop_registry() -> MetricsRegistry:
    """Return a registry whose metrics are :class:`_NoopMetric`."""
    return MetricsRegistry(
        runs_submitted=_NoopMetric("efe_orch_runs_submitted_total"),
        runs_transitioned=_NoopMetric("efe_orch_runs_transitioned_total"),
        run_duration_seconds=_NoopMetric("efe_orch_run_duration_seconds"),
        runs_in_state=_NoopMetric("efe_orch_runs_in_state"),
        reaper_cycles=_NoopMetric("efe_orch_reaper_cycles_total"),
        retry_decisions=_NoopMetric("efe_orch_retry_decisions_total"),
        k8s_submit=_NoopMetric("efe_orch_k8s_submit_total"),
    )


def build_registry(namespace: str = "efe_orch") -> MetricsRegistry:
    """Return a real prometheus registry if available, else noop.

    We intentionally don't *require* prometheus_client — the module
    importing this helper does not break if the dep is missing.
    """
    try:
        from prometheus_client import Counter, Gauge, Histogram
    except ImportError:
        return noop_registry()

    return MetricsRegistry(
        runs_submitted=Counter(
            f"{namespace}_runs_submitted_total",
            "Number of run submissions accepted.",
            labelnames=("tenant", "pool"),
        ),
        runs_transitioned=Counter(
            f"{namespace}_runs_transitioned_total",
            "Number of state transitions performed.",
            labelnames=("from_state", "to_state"),
        ),
        run_duration_seconds=Histogram(
            f"{namespace}_run_duration_seconds",
            "Wall-clock run duration (QUEUED→terminal), seconds.",
            labelnames=("pool",),
            buckets=(10, 30, 60, 120, 300, 600, 1_800, 3_600, 10_800),
        ),
        runs_in_state=Gauge(
            f"{namespace}_runs_in_state",
            "Current number of runs per state.",
            labelnames=("state",),
        ),
        reaper_cycles=Counter(
            f"{namespace}_reaper_cycles_total",
            "Number of heartbeat-reaper cycles executed.",
            labelnames=("outcome",),
        ),
        retry_decisions=Counter(
            f"{namespace}_retry_decisions_total",
            "Retry policy decisions emitted.",
            labelnames=("error_class", "retry"),
        ),
        k8s_submit=Counter(
            f"{namespace}_k8s_submit_total",
            "K8s Job submissions performed.",
            labelnames=("outcome",),
        ),
    )


__all__ = ["MetricsRegistry", "build_registry", "noop_registry"]
