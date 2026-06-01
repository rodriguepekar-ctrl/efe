"""Prometheus-compatible metrics (in-memory fallback for tests)."""

from __future__ import annotations

from collections import defaultdict
from dataclasses import dataclass, field


def _lkey(labels: dict[str, str]) -> tuple[tuple[str, str], ...]:
    return tuple(sorted(labels.items()))


@dataclass
class _Counter:
    name: str
    values: dict[tuple[tuple[str, str], ...], float] = field(
        default_factory=dict,
    )

    def inc(self, n: float = 1.0, **labels: str) -> None:
        k = _lkey(labels)
        self.values[k] = self.values.get(k, 0.0) + float(n)

    def value(self, **labels: str) -> float:
        return float(self.values.get(_lkey(labels), 0.0))


@dataclass
class _Gauge:
    name: str
    values: dict[tuple[tuple[str, str], ...], float] = field(
        default_factory=dict,
    )

    def set(self, v: float, **labels: str) -> None:
        self.values[_lkey(labels)] = float(v)

    def value(self, **labels: str) -> float:
        return float(self.values.get(_lkey(labels), 0.0))


@dataclass
class _Histogram:
    name: str
    buckets: tuple[float, ...] = (
        0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5,
        1.0, 2.5, 5.0, 10.0, 30.0, 60.0,
    )
    counts: dict[tuple[tuple[str, str], ...], list[int]] = field(
        default_factory=lambda: defaultdict(list),
    )
    sums: dict[tuple[tuple[str, str], ...], float] = field(
        default_factory=lambda: defaultdict(float),
    )

    def observe(self, v: float, **labels: str) -> None:
        key = _lkey(labels)
        if key not in self.counts:
            self.counts[key] = [0] * (len(self.buckets) + 1)
        for i, b in enumerate(self.buckets):
            if v <= b:
                self.counts[key][i] += 1
        self.counts[key][-1] += 1
        self.sums[key] = self.sums.get(key, 0.0) + float(v)


@dataclass
class MetricsCatalog:
    requests_total: _Counter = field(init=False)
    chat_latency_seconds: _Histogram = field(init=False)
    ttft_seconds: _Histogram = field(init=False)
    adapter_overhead_seconds: _Histogram = field(init=False)
    prompt_tokens_total: _Counter = field(init=False)
    completion_tokens_total: _Counter = field(init=False)
    cost_usd_total: _Counter = field(init=False)
    cost_micro_cents_total: _Counter = field(init=False)
    budget_rejections_total: _Counter = field(init=False)
    cache_hits_total: _Counter = field(init=False)
    cache_misses_total: _Counter = field(init=False)
    safety_block_total: _Counter = field(init=False)
    safety_scrub_total: _Counter = field(init=False)
    retry_total: _Counter = field(init=False)
    circuit_state: _Gauge = field(init=False)
    fallback_used_total: _Counter = field(init=False)
    backend_errors_total: _Counter = field(init=False)
    dlq_entries_total: _Counter = field(init=False)
    tool_repair_total: _Counter = field(init=False)

    def __post_init__(self) -> None:
        self.requests_total = _Counter("requests_total")
        self.chat_latency_seconds = _Histogram("chat_latency_seconds")
        self.ttft_seconds = _Histogram("ttft_seconds")
        self.adapter_overhead_seconds = _Histogram(
            "adapter_overhead_seconds",
        )
        self.prompt_tokens_total = _Counter("prompt_tokens_total")
        self.completion_tokens_total = _Counter("completion_tokens_total")
        self.cost_usd_total = _Counter("cost_usd_total")
        self.cost_micro_cents_total = _Counter("cost_micro_cents_total")
        self.budget_rejections_total = _Counter("budget_rejections_total")
        self.cache_hits_total = _Counter("cache_hits_total")
        self.cache_misses_total = _Counter("cache_misses_total")
        self.safety_block_total = _Counter("safety_block_total")
        self.safety_scrub_total = _Counter("safety_scrub_total")
        self.retry_total = _Counter("retry_total")
        self.circuit_state = _Gauge("circuit_state")
        self.fallback_used_total = _Counter("fallback_used_total")
        self.backend_errors_total = _Counter("backend_errors_total")
        self.dlq_entries_total = _Counter("dlq_entries_total")
        self.tool_repair_total = _Counter("tool_repair_total")


_DEFAULT: MetricsCatalog | None = None


def get_catalog() -> MetricsCatalog:
    global _DEFAULT
    if _DEFAULT is None:
        _DEFAULT = MetricsCatalog()
    return _DEFAULT


def reset_catalog() -> None:
    global _DEFAULT
    _DEFAULT = MetricsCatalog()


__all__ = ["MetricsCatalog", "get_catalog", "reset_catalog"]
