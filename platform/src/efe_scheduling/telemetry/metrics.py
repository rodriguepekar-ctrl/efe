"""Prometheus-style metrics registry.

If the ``prometheus_client`` package is available, we build real
Counter/Histogram/Gauge objects. Otherwise, we fall back to a tiny
in-memory recorder that the tests can inspect.
"""

from __future__ import annotations

import threading
from dataclasses import dataclass, field
from typing import Any

try:  # pragma: no cover - exercised only when the dep is installed
    from prometheus_client import CollectorRegistry, Counter, Gauge, Histogram

    _PROMETHEUS_AVAILABLE = True
except ImportError:  # pragma: no cover
    _PROMETHEUS_AVAILABLE = False
    CollectorRegistry = object  # type: ignore[assignment,misc]
    Counter = object  # type: ignore[assignment,misc]
    Gauge = object  # type: ignore[assignment,misc]
    Histogram = object  # type: ignore[assignment,misc]

__all__ = ["MetricsRegistry", "get_registry", "reset_registry"]


@dataclass(slots=True)
class _StubMetric:
    name: str
    kind: str
    labels: tuple[str, ...]
    samples: list[tuple[dict[str, str], float]] = field(default_factory=list)
    gauge_values: dict[tuple[str, ...], float] = field(default_factory=dict)

    def _label_key(self, label_values: dict[str, str]) -> tuple[str, ...]:
        return tuple(label_values.get(k, "") for k in self.labels)

    def inc(self, by: float = 1.0, **label_values: str) -> None:
        self.samples.append((dict(label_values), by))

    def observe(self, value: float, **label_values: str) -> None:
        self.samples.append((dict(label_values), value))

    def set(self, value: float, **label_values: str) -> None:
        self.gauge_values[self._label_key(label_values)] = value
        self.samples.append((dict(label_values), value))


class MetricsRegistry:
    """Facade over either ``prometheus_client`` or a stub recorder."""

    __slots__ = ("_metrics", "_prom_registry", "_lock")

    def __init__(self) -> None:
        self._metrics: dict[str, Any] = {}
        self._prom_registry = CollectorRegistry() if _PROMETHEUS_AVAILABLE else None
        self._lock = threading.Lock()

    # ------------------------------------------------------------------
    # Metric creation helpers
    # ------------------------------------------------------------------

    def counter(self, name: str, description: str, labels: tuple[str, ...] = ()) -> Any:
        return self._get_or_create(name, "counter", description, labels)

    def histogram(
        self, name: str, description: str, labels: tuple[str, ...] = ()
    ) -> Any:
        return self._get_or_create(name, "histogram", description, labels)

    def gauge(self, name: str, description: str, labels: tuple[str, ...] = ()) -> Any:
        return self._get_or_create(name, "gauge", description, labels)

    def _get_or_create(
        self, name: str, kind: str, description: str, labels: tuple[str, ...]
    ) -> Any:
        with self._lock:
            existing = self._metrics.get(name)
            if existing is not None:
                return existing
            if _PROMETHEUS_AVAILABLE:
                assert self._prom_registry is not None
                if kind == "counter":
                    m = Counter(name, description, labels, registry=self._prom_registry)
                elif kind == "histogram":
                    m = Histogram(name, description, labels, registry=self._prom_registry)
                else:
                    m = Gauge(name, description, labels, registry=self._prom_registry)
            else:
                m = _StubMetric(name=name, kind=kind, labels=labels)
            self._metrics[name] = m
            return m

    # ------------------------------------------------------------------
    # Inspection (used by tests)
    # ------------------------------------------------------------------

    def get(self, name: str) -> Any | None:
        return self._metrics.get(name)

    def names(self) -> list[str]:
        return sorted(self._metrics.keys())


_REGISTRY: MetricsRegistry | None = None


def get_registry() -> MetricsRegistry:
    global _REGISTRY
    if _REGISTRY is None:
        _REGISTRY = MetricsRegistry()
        _install_default_metrics(_REGISTRY)
    return _REGISTRY


def reset_registry() -> None:
    global _REGISTRY
    _REGISTRY = None


def _install_default_metrics(reg: MetricsRegistry) -> None:
    """Pre-register the metrics enumerated in §14.1 of the architecture."""
    reg.counter(
        "efe_sched_fire_total", "Fires enqueued", ("tenant", "kind")
    )
    reg.counter(
        "efe_sched_fire_duplicate_total",
        "Fires rejected as duplicate",
        ("tenant", "kind"),
    )
    reg.counter(
        "efe_sched_fire_dropped_total",
        "Fires dropped (catch-up truncation, ...)",
        ("tenant", "reason"),
    )
    reg.histogram(
        "efe_sched_tick_duration_seconds",
        "Duration of one scheduler tick",
        (),
    )
    reg.histogram(
        "efe_sched_job_wait_seconds",
        "Queued → running latency",
        ("kind",),
    )
    reg.histogram(
        "efe_sched_job_run_seconds",
        "Running → terminal latency",
        ("kind",),
    )
    reg.counter(
        "efe_sched_job_state_total",
        "Job state transitions",
        ("kind", "to_state"),
    )
    reg.counter(
        "efe_sched_retry_total", "Retries scheduled", ("kind", "reason")
    )
    reg.counter(
        "efe_sched_dead_letter_total", "Dead-lettered jobs", ("tenant", "kind")
    )
    reg.gauge(
        "efe_sched_worker_lease_gauge",
        "Jobs currently leased by worker",
        ("worker_id",),
    )
    reg.counter(
        "efe_sched_trigger_fire_total",
        "Triggers fired",
        ("tenant", "trigger_kind"),
    )
    reg.counter(
        "efe_sched_learning_decision_total",
        "Learning policy decisions",
        ("policy", "decision"),
    )
