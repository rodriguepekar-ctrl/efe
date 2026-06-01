"""Prometheus metrics — worker-side.

Catalog (per M04 §15):

* ``efe_worker_training_step_loss``          Histogram(method)
* ``efe_worker_training_step_duration_s``    Histogram(method)
* ``efe_worker_gpu_memory_used_gib``         Gauge(gpu_id)
* ``efe_worker_gpu_utilization_ratio``       Gauge(gpu_id)
* ``efe_worker_heartbeat_total``             Counter(outcome)
* ``efe_worker_checkpoint_save_duration_s``  Histogram(method)
* ``efe_worker_eval_metric_value``           Gauge(method, metric)
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
    training_step_loss: _Metric
    training_step_duration_s: _Metric
    gpu_memory_used_gib: _Metric
    gpu_utilization_ratio: _Metric
    heartbeat_total: _Metric
    checkpoint_save_duration_s: _Metric
    eval_metric_value: _Metric

    def snapshot(self) -> dict[str, Any]:
        out: dict[str, Any] = {}
        for name in self.__dataclass_fields__:
            m = getattr(self, name)
            if isinstance(m, _NoopMetric):
                out[m.name] = list(m.values)
        return out


def noop_registry() -> MetricsRegistry:
    return MetricsRegistry(
        training_step_loss=_NoopMetric("efe_worker_training_step_loss"),
        training_step_duration_s=_NoopMetric(
            "efe_worker_training_step_duration_seconds",
        ),
        gpu_memory_used_gib=_NoopMetric(
            "efe_worker_gpu_memory_used_gib",
        ),
        gpu_utilization_ratio=_NoopMetric(
            "efe_worker_gpu_utilization_ratio",
        ),
        heartbeat_total=_NoopMetric("efe_worker_heartbeat_total"),
        checkpoint_save_duration_s=_NoopMetric(
            "efe_worker_checkpoint_save_duration_seconds",
        ),
        eval_metric_value=_NoopMetric("efe_worker_eval_metric_value"),
    )


def build_registry(namespace: str = "efe_worker") -> MetricsRegistry:
    try:
        from prometheus_client import Counter, Gauge, Histogram
    except ImportError:
        return noop_registry()
    return MetricsRegistry(
        training_step_loss=Histogram(
            f"{namespace}_training_step_loss",
            "Per-step training loss.",
            labelnames=("method",),
            buckets=(0.01, 0.05, 0.1, 0.3, 1.0, 3.0, 10.0),
        ),
        training_step_duration_s=Histogram(
            f"{namespace}_training_step_duration_seconds",
            "Wall time per training step.",
            labelnames=("method",),
            buckets=(0.01, 0.05, 0.1, 0.3, 1.0, 3.0, 10.0, 30.0),
        ),
        gpu_memory_used_gib=Gauge(
            f"{namespace}_gpu_memory_used_gib",
            "GPU VRAM used (GiB).",
            labelnames=("gpu_id",),
        ),
        gpu_utilization_ratio=Gauge(
            f"{namespace}_gpu_utilization_ratio",
            "GPU utilisation ratio [0, 1].",
            labelnames=("gpu_id",),
        ),
        heartbeat_total=Counter(
            f"{namespace}_heartbeat_total",
            "Heartbeats attempted.",
            labelnames=("outcome",),
        ),
        checkpoint_save_duration_s=Histogram(
            f"{namespace}_checkpoint_save_duration_seconds",
            "Checkpoint save + upload.",
            labelnames=("method",),
            buckets=(0.1, 0.5, 1, 3, 10, 30, 90),
        ),
        eval_metric_value=Gauge(
            f"{namespace}_eval_metric_value",
            "Current eval metric value.",
            labelnames=("method", "metric"),
        ),
    )


__all__ = ["MetricsRegistry", "build_registry", "noop_registry"]
