"""Metrics — thin abstraction with a Prometheus-compatible API.

If ``prometheus_client`` is installed we delegate; otherwise we keep
in-memory counters / histograms that tests can assert against.
"""

from __future__ import annotations

import math
from collections import defaultdict
from dataclasses import dataclass, field
from typing import Protocol


class Counter(Protocol):
    def inc(
        self, n: float = 1.0, **labels: str,
    ) -> None:  # pragma: no cover
        ...


class Gauge(Protocol):
    def set(self, value: float, **labels: str) -> None:  # pragma: no cover
        ...


class Histogram(Protocol):
    def observe(
        self, value: float, **labels: str,
    ) -> None:  # pragma: no cover
        ...


# ---------------------------------------------------------------------------
# In-memory implementation
# ---------------------------------------------------------------------------


def _labels_key(labels: dict[str, str]) -> tuple[tuple[str, str], ...]:
    return tuple(sorted(labels.items()))


@dataclass
class _InMemoryCounter:
    name: str
    values: dict[tuple[tuple[str, str], ...], float] = field(
        default_factory=dict,
    )

    def inc(self, n: float = 1.0, **labels: str) -> None:
        k = _labels_key(labels)
        self.values[k] = self.values.get(k, 0.0) + float(n)

    def value(self, **labels: str) -> float:
        return float(self.values.get(_labels_key(labels), 0.0))


@dataclass
class _InMemoryGauge:
    name: str
    values: dict[tuple[tuple[str, str], ...], float] = field(
        default_factory=dict,
    )

    def set(self, value: float, **labels: str) -> None:
        self.values[_labels_key(labels)] = float(value)

    def value(self, **labels: str) -> float:
        return float(self.values.get(_labels_key(labels), 0.0))


@dataclass
class _InMemoryHistogram:
    name: str
    buckets: tuple[float, ...] = (
        0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5,
        1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0,
    )
    counts: dict[tuple[tuple[str, str], ...], list[int]] = field(
        default_factory=lambda: defaultdict(
            lambda: [0] * 16,  # Overridden after init
        ),
    )
    sums: dict[tuple[tuple[str, str], ...], float] = field(
        default_factory=lambda: defaultdict(float),
    )

    def observe(self, value: float, **labels: str) -> None:
        key = _labels_key(labels)
        if key not in self.counts:
            self.counts[key] = [0] * (len(self.buckets) + 1)
        for i, b in enumerate(self.buckets):
            if value <= b:
                self.counts[key][i] += 1
        self.counts[key][-1] += 1  # +Inf bucket
        self.sums[key] = self.sums.get(key, 0.0) + float(value)

    def p(self, q: float, **labels: str) -> float:
        key = _labels_key(labels)
        buckets = self.counts.get(key)
        if not buckets:
            return float("nan")
        total = buckets[-1]
        if total == 0:
            return float("nan")
        target = q * total
        acc = 0
        for i, count in enumerate(buckets[:-1]):
            acc += count
            if acc >= target:
                return self.buckets[i]
        return self.buckets[-1]


# ---------------------------------------------------------------------------
# Catalog — all M06 metrics
# ---------------------------------------------------------------------------


@dataclass
class MetricsCatalog:
    publish_total: _InMemoryCounter = field(init=False)
    verify_latency_s: _InMemoryHistogram = field(init=False)
    verify_failures_total: _InMemoryCounter = field(init=False)
    router_requests_total: _InMemoryCounter = field(init=False)
    router_request_latency_s: _InMemoryHistogram = field(init=False)
    router_routing_table_version: _InMemoryGauge = field(init=False)
    router_cache_hits_total: _InMemoryCounter = field(init=False)
    router_cache_misses_total: _InMemoryCounter = field(init=False)
    router_quota_rejected_total: _InMemoryCounter = field(init=False)
    router_inflight_requests: _InMemoryGauge = field(init=False)
    router_dropped_requests_total: _InMemoryCounter = field(init=False)
    canary_active: _InMemoryGauge = field(init=False)
    canary_percent: _InMemoryGauge = field(init=False)
    canary_slo_breaches_total: _InMemoryCounter = field(init=False)
    canary_advance_total: _InMemoryCounter = field(init=False)
    canary_aborted_total: _InMemoryCounter = field(init=False)
    rollback_requested_total: _InMemoryCounter = field(init=False)
    rollback_completed_total: _InMemoryCounter = field(init=False)
    rollback_failed_total: _InMemoryCounter = field(init=False)
    rollback_latency_s: _InMemoryHistogram = field(init=False)
    rollback_phase_latency_s: _InMemoryHistogram = field(init=False)
    serving_loaded_artifacts: _InMemoryGauge = field(init=False)
    serving_tokens_generated_total: _InMemoryCounter = field(init=False)
    shadow_samples_total: _InMemoryCounter = field(init=False)
    shadow_cosine_mean: _InMemoryGauge = field(init=False)
    shadow_latency_delta_ms_mean: _InMemoryGauge = field(init=False)
    admission_reject_total: _InMemoryCounter = field(init=False)
    admission_pass_total: _InMemoryCounter = field(init=False)

    def __post_init__(self) -> None:
        self.publish_total = _InMemoryCounter("registry_publish_total")
        self.verify_latency_s = _InMemoryHistogram("verify_latency_s")
        self.verify_failures_total = _InMemoryCounter("verify_failures_total")
        self.router_requests_total = _InMemoryCounter("router_requests_total")
        self.router_request_latency_s = _InMemoryHistogram(
            "router_request_latency_s",
        )
        self.router_routing_table_version = _InMemoryGauge(
            "router_routing_table_version",
        )
        self.router_cache_hits_total = _InMemoryCounter(
            "router_cache_hits_total",
        )
        self.router_cache_misses_total = _InMemoryCounter(
            "router_cache_misses_total",
        )
        self.router_quota_rejected_total = _InMemoryCounter(
            "router_quota_rejected_total",
        )
        self.router_inflight_requests = _InMemoryGauge(
            "router_inflight_requests",
        )
        self.router_dropped_requests_total = _InMemoryCounter(
            "router_dropped_requests_total",
        )
        self.canary_active = _InMemoryGauge("canary_active")
        self.canary_percent = _InMemoryGauge("canary_percent")
        self.canary_slo_breaches_total = _InMemoryCounter(
            "canary_slo_breaches_total",
        )
        self.canary_advance_total = _InMemoryCounter("canary_advance_total")
        self.canary_aborted_total = _InMemoryCounter("canary_aborted_total")
        self.rollback_requested_total = _InMemoryCounter(
            "rollback_requested_total",
        )
        self.rollback_completed_total = _InMemoryCounter(
            "rollback_completed_total",
        )
        self.rollback_failed_total = _InMemoryCounter(
            "rollback_failed_total",
        )
        self.rollback_latency_s = _InMemoryHistogram("rollback_latency_s")
        self.rollback_phase_latency_s = _InMemoryHistogram(
            "rollback_phase_latency_s",
        )
        self.serving_loaded_artifacts = _InMemoryGauge(
            "serving_loaded_artifacts",
        )
        self.serving_tokens_generated_total = _InMemoryCounter(
            "serving_tokens_generated_total",
        )
        self.shadow_samples_total = _InMemoryCounter("shadow_samples_total")
        self.shadow_cosine_mean = _InMemoryGauge("shadow_cosine_mean")
        self.shadow_latency_delta_ms_mean = _InMemoryGauge(
            "shadow_latency_delta_ms_mean",
        )
        self.admission_reject_total = _InMemoryCounter(
            "admission_reject_total",
        )
        self.admission_pass_total = _InMemoryCounter("admission_pass_total")


_DEFAULT: MetricsCatalog | None = None


def get_catalog() -> MetricsCatalog:
    global _DEFAULT
    if _DEFAULT is None:
        _DEFAULT = MetricsCatalog()
    return _DEFAULT


def reset_catalog() -> None:
    global _DEFAULT
    _DEFAULT = MetricsCatalog()


def _sane(x: float) -> float:
    return 0.0 if math.isnan(x) else x


__all__ = [
    "Counter",
    "Gauge",
    "Histogram",
    "MetricsCatalog",
    "get_catalog",
    "reset_catalog",
]
