"""Minimal Prometheus-compatible metrics registry.

We avoid depending on ``prometheus_client`` directly so the platform
stays lightweight. Instead we ship a tiny in-memory registry that
renders to the Prometheus text format — good enough for the MVP
``/metrics`` endpoint. If a consumer wants ``prometheus_client``
features (pushgateway, labels beyond what we emit), they can always
write a thin adapter.

Emitted metrics
---------------

* ``efe_platform_requests_total{tenant,outcome}`` — counter.
* ``efe_platform_request_duration_seconds_sum{tenant}`` — counter (seconds).
* ``efe_platform_request_duration_seconds_count{tenant}`` — counter.
* ``efe_platform_tool_calls_total{tool,outcome}`` — counter.
* ``efe_platform_llm_tokens_total{phase}`` — counter.
* ``efe_platform_llm_usd_total`` — counter.
* ``efe_platform_budget_exceeded_total{dimension}`` — counter.
"""

from __future__ import annotations

import threading
from collections import defaultdict
from dataclasses import dataclass
from typing import Any

# ---------------------------------------------------------------------------
#  Immutable record
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class RequestMetrics:
    """Snapshot view of a single completed request's emitted counters."""

    tenant: str
    outcome: str  # "ok" | "error" | "budget" | ...
    duration_s: float
    tool_calls: int
    llm_tokens: int
    llm_usd: float


# ---------------------------------------------------------------------------
#  Registry
# ---------------------------------------------------------------------------


_COUNTER_HEADERS: dict[str, tuple[str, str]] = {
    "efe_platform_requests_total": ("counter", "Total requests by tenant and outcome."),
    "efe_platform_request_duration_seconds_sum": (
        "counter",
        "Sum of request durations in seconds.",
    ),
    "efe_platform_request_duration_seconds_count": (
        "counter",
        "Count of request durations.",
    ),
    "efe_platform_tool_calls_total": (
        "counter",
        "Tool invocations by name and outcome.",
    ),
    "efe_platform_llm_tokens_total": ("counter", "LLM tokens consumed by phase."),
    "efe_platform_llm_usd_total": ("counter", "LLM USD consumed."),
    "efe_platform_budget_exceeded_total": (
        "counter",
        "Budget ceiling breaches by dimension.",
    ),
}


class MetricsRegistry:
    """Thread-safe counter registry with text-format rendering."""

    def __init__(self) -> None:
        self._lock = threading.Lock()
        # dict[ metric_name ][ tuple(sorted labels) ] -> float
        self._counters: dict[str, dict[tuple[tuple[str, str], ...], float]] = (
            defaultdict(dict)
        )

    # ----------------------------------------------------------- Record API

    def record_request(self, snapshot: RequestMetrics) -> None:
        labels_tenant = (("tenant", snapshot.tenant), ("outcome", snapshot.outcome))
        self._add("efe_platform_requests_total", labels_tenant, 1.0)
        self._add(
            "efe_platform_request_duration_seconds_sum",
            (("tenant", snapshot.tenant),),
            snapshot.duration_s,
        )
        self._add(
            "efe_platform_request_duration_seconds_count",
            (("tenant", snapshot.tenant),),
            1.0,
        )
        self._add(
            "efe_platform_llm_tokens_total",
            (("phase", "total"),),
            float(snapshot.llm_tokens),
        )
        self._add("efe_platform_llm_usd_total", (), snapshot.llm_usd)

    def record_tool_call(self, *, tool: str, ok: bool) -> None:
        outcome = "ok" if ok else "error"
        self._add(
            "efe_platform_tool_calls_total",
            (("tool", tool), ("outcome", outcome)),
            1.0,
        )

    def record_budget_exceeded(self, *, dimension: str) -> None:
        self._add(
            "efe_platform_budget_exceeded_total",
            (("dimension", dimension),),
            1.0,
        )

    def _add(
        self,
        name: str,
        labels: tuple[tuple[str, str], ...],
        value: float,
    ) -> None:
        # Normalize label order for stable lookup.
        key = tuple(sorted(labels))
        with self._lock:
            bucket = self._counters[name]
            bucket[key] = bucket.get(key, 0.0) + value

    # ---------------------------------------------------------- Prometheus

    def render(self) -> str:
        """Return the Prometheus text exposition (UTF-8 text/plain)."""
        lines: list[str] = []
        with self._lock:
            for name, (mtype, doc) in _COUNTER_HEADERS.items():
                bucket = self._counters.get(name, {})
                lines.append(f"# HELP {name} {doc}")
                lines.append(f"# TYPE {name} {mtype}")
                if not bucket:
                    continue
                for labels, value in sorted(bucket.items()):
                    label_str = (
                        "{" + ",".join(f'{k}="{_escape(v)}"' for k, v in labels) + "}"
                        if labels
                        else ""
                    )
                    lines.append(f"{name}{label_str} {value}")
        return "\n".join(lines) + "\n"

    # ----------------------------------------------------------- Test help

    def snapshot(self) -> dict[str, dict[str, float]]:
        """Non-standard JSON-ish view — only for tests."""
        with self._lock:
            return {
                name: {_labels_to_str(labels): v for labels, v in bucket.items()}
                for name, bucket in self._counters.items()
            }


def _labels_to_str(labels: tuple[tuple[str, str], ...]) -> str:
    if not labels:
        return ""
    return ",".join(f"{k}={v}" for k, v in labels)


def _escape(v: Any) -> str:
    return str(v).replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n")


__all__ = ["MetricsRegistry", "RequestMetrics"]
