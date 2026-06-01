"""Prometheus metrics integration for Letter D.

Soft dependency on ``prometheus_client``: when the package is not
installed, every function in this module becomes a no-op. Hosts that
do export Prometheus automatically get the metrics registered on the
default registry.

Metric names (stable):

    <ns>_classify_total{catalog,label,outcome}           Counter
    <ns>_classify_latency_seconds{catalog}               Histogram
    <ns>_classify_errors_total{catalog,error_code}       Counter
    <ns>_facade_in_flight{catalog}                       Gauge
    <ns>_facade_preload_seconds                          Histogram
    <ns>_facade_ready                                    Gauge

``<ns>`` defaults to ``efe`` and is set by :attr:`EfeConfig.metrics_namespace`.
"""

from __future__ import annotations

import importlib.util
from typing import Any, Optional


_HAS_PROMETHEUS: bool = (
    importlib.util.find_spec("prometheus_client") is not None
)


class _NullMetric:
    """Drop-in that swallows every call when prometheus_client is absent."""

    def labels(self, **_kw: Any) -> "_NullMetric":
        return self

    def inc(self, *_a: Any, **_kw: Any) -> None:
        return None

    def dec(self, *_a: Any, **_kw: Any) -> None:
        return None

    def observe(self, *_a: Any, **_kw: Any) -> None:
        return None

    def set(self, *_a: Any, **_kw: Any) -> None:
        return None


class MetricsRegistry:
    """Holds the per-namespace metric handles. Safe to call when
    prometheus_client is missing: every method is a cheap no-op."""

    def __init__(
        self, namespace: str = "efe", registry: Optional[Any] = None,
    ) -> None:
        self._namespace = namespace
        self._enabled = _HAS_PROMETHEUS
        if not self._enabled:
            self.classify_total = _NullMetric()
            self.classify_latency = _NullMetric()
            self.classify_errors = _NullMetric()
            self.in_flight = _NullMetric()
            self.preload_seconds = _NullMetric()
            self.ready = _NullMetric()
            return

        from prometheus_client import Counter, Gauge, Histogram
        from prometheus_client.registry import REGISTRY

        reg = registry if registry is not None else REGISTRY

        # Wrap creation in try/except because pytest may re-import this
        # module under the same namespace → duplicate metric registration.
        def _counter(name: str, doc: str, labels: list[str]) -> Any:
            try:
                return Counter(name, doc, labels, registry=reg)
            except ValueError:
                return _find_existing(reg, name) or _NullMetric()

        def _histogram(
            name: str, doc: str, labels: list[str],
            buckets: Optional[tuple[float, ...]] = None,
        ) -> Any:
            try:
                kw = {"registry": reg}
                if buckets is not None:
                    kw["buckets"] = buckets
                return Histogram(name, doc, labels, **kw)
            except ValueError:
                return _find_existing(reg, name) or _NullMetric()

        def _gauge(name: str, doc: str, labels: list[str]) -> Any:
            try:
                return Gauge(name, doc, labels, registry=reg)
            except ValueError:
                return _find_existing(reg, name) or _NullMetric()

        ns = namespace
        self.classify_total = _counter(
            f"{ns}_classify_total",
            "Number of classify() calls by outcome.",
            ["catalog", "label", "outcome"],
        )
        # Buckets targeting the documented p95 budget (< 2 s).
        self.classify_latency = _histogram(
            f"{ns}_classify_latency_seconds",
            "End-to-end latency of classify() as measured client-side.",
            ["catalog"],
            buckets=(0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0),
        )
        self.classify_errors = _counter(
            f"{ns}_classify_errors_total",
            "Number of classify() calls that raised an EfeFacadeError.",
            ["catalog", "error_code"],
        )
        self.in_flight = _gauge(
            f"{ns}_facade_in_flight",
            "Number of classify() calls currently in flight.",
            ["catalog"],
        )
        self.preload_seconds = _histogram(
            f"{ns}_facade_preload_seconds",
            "Duration of preload() (embedder + catalogs + probe).",
            [],
            buckets=(0.1, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 15.0),
        )
        self.ready = _gauge(
            f"{ns}_facade_ready",
            "1 if the facade is ready to serve, 0 otherwise.",
            [],
        )

    # ------------------------------------------------------------------
    #  Recording helpers
    # ------------------------------------------------------------------

    def record_classify(
        self, *, catalog: str, label: str, outcome: str,
        elapsed_s: float,
    ) -> None:
        self.classify_total.labels(
            catalog=catalog, label=label, outcome=outcome,
        ).inc()
        self.classify_latency.labels(catalog=catalog).observe(elapsed_s)

    def record_error(self, *, catalog: str, error_code: str) -> None:
        self.classify_errors.labels(
            catalog=catalog, error_code=error_code,
        ).inc()

    def inflight_inc(self, *, catalog: str) -> None:
        self.in_flight.labels(catalog=catalog).inc()

    def inflight_dec(self, *, catalog: str) -> None:
        self.in_flight.labels(catalog=catalog).dec()

    def observe_preload(self, *, seconds: float) -> None:
        self.preload_seconds.observe(seconds)

    def set_ready(self, *, ready: bool) -> None:
        self.ready.set(1 if ready else 0)


def _find_existing(registry: Any, name: str) -> Optional[Any]:
    """Retrieve a metric already registered on ``registry`` (for tests
    that re-import this module)."""
    try:
        collectors = list(registry._collector_to_names.keys())  # type: ignore[attr-defined]
    except Exception:  # pragma: no cover
        return None
    for c in collectors:
        try:
            if getattr(c, "_name", None) == name:
                return c
        except Exception:  # pragma: no cover
            continue
    return None


__all__ = ["MetricsRegistry", "_HAS_PROMETHEUS"]
