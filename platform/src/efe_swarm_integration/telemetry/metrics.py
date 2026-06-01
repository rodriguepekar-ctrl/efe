"""Prometheus-style metrics for M12 — with stub fallback."""

from __future__ import annotations

import threading
from dataclasses import dataclass, field
from typing import Any

try:  # pragma: no cover
    from prometheus_client import CollectorRegistry, Counter, Gauge, Histogram

    _PROM_AVAILABLE = True
except ImportError:  # pragma: no cover
    _PROM_AVAILABLE = False
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
        self.samples.append((dict(label_values), float(by)))

    def observe(self, value: float, **label_values: str) -> None:
        self.samples.append((dict(label_values), float(value)))

    def set(self, value: float, **label_values: str) -> None:
        self.gauge_values[self._label_key(label_values)] = float(value)
        self.samples.append((dict(label_values), float(value)))


class MetricsRegistry:
    """Facade over either ``prometheus_client`` or a stub recorder."""

    __slots__ = ("_lock", "_metrics", "_prom_registry")

    def __init__(self) -> None:
        self._metrics: dict[str, Any] = {}
        self._prom_registry = CollectorRegistry() if _PROM_AVAILABLE else None
        self._lock = threading.Lock()

    def counter(self, name: str, description: str, labels: tuple[str, ...] = ()) -> Any:
        return self._get_or_create(name, "counter", description, labels)

    def histogram(self, name: str, description: str, labels: tuple[str, ...] = ()) -> Any:
        return self._get_or_create(name, "histogram", description, labels)

    def gauge(self, name: str, description: str, labels: tuple[str, ...] = ()) -> Any:
        return self._get_or_create(name, "gauge", description, labels)

    def _get_or_create(
        self, name: str, kind: str, description: str, labels: tuple[str, ...]
    ) -> Any:
        with self._lock:
            if name in self._metrics:
                return self._metrics[name]
            if _PROM_AVAILABLE:
                assert self._prom_registry is not None
                if kind == "counter":
                    m = Counter(name, description, labels, registry=self._prom_registry)
                elif kind == "histogram":
                    m = Histogram(
                        name, description, labels, registry=self._prom_registry
                    )
                else:
                    m = Gauge(name, description, labels, registry=self._prom_registry)
            else:
                m = _StubMetric(name=name, kind=kind, labels=labels)
            self._metrics[name] = m
            return m

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
    reg.counter("efe_m12_rpc_total", "RPC count", ("tenant", "method", "status"))
    reg.histogram("efe_m12_rpc_latency_seconds", "RPC latency", ("method",))
    reg.gauge("efe_m12_rpc_inflight", "Inflight RPCs", ("tenant",))
    reg.gauge("efe_m12_cb_state", "Circuit breaker state (0 closed / 1 half / 2 open)", ("tenant",))
    reg.counter(
        "efe_m12_cb_transitions_total",
        "Circuit breaker transitions",
        ("tenant", "from_state", "to_state"),
    )
    reg.counter(
        "efe_m12_bulkhead_rejected_total",
        "Bulkhead rejections",
        ("tenant",),
    )
    reg.histogram(
        "efe_m12_hbm_estimate_bytes", "Ex-ante HBM estimate", ("tenant",)
    )
    reg.counter("efe_m12_verdict_total", "Verdict counter", ("verdict",))
    reg.histogram("efe_m12_alignment_delta", "Alignment delta", ())
    reg.counter(
        "efe_m12_streaming_chunks_total", "Stream chunks", ("kind",)
    )
    reg.gauge(
        "efe_m12_cone_binding_version", "Active cone version", ("feature_id",)
    )
    reg.counter(
        "efe_m12_artefact_sync_total", "Artefact syncs", ("status",)
    )
