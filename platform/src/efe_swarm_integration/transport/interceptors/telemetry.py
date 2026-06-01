"""TelemetryInterceptor — Prometheus counters + latency histogram."""

from __future__ import annotations

import time
from dataclasses import dataclass
from typing import Any, Protocol

__all__ = ["TelemetryInterceptor"]


class _Counter(Protocol):
    def inc(self, *args: Any, **kwargs: Any) -> None: ...  # pragma: no cover


class _Histogram(Protocol):
    def observe(self, *args: Any, **kwargs: Any) -> None: ...  # pragma: no cover


@dataclass(slots=True)
class TelemetryInterceptor:
    rpc_total: _Counter | None = None
    latency_seconds: _Histogram | None = None
    verdict_total: _Counter | None = None

    def record_start(self) -> float:
        return time.monotonic()

    def record_end(
        self,
        start_mono: float,
        *,
        tenant: str,
        method: str,
        status: str,
        verdict: str | None = None,
    ) -> float:
        latency = max(0.0, time.monotonic() - start_mono)
        if self.rpc_total is not None:
            try:
                self.rpc_total.inc(
                    tenant=tenant, method=method, status=status
                )
            except Exception:  # pragma: no cover
                pass
        if self.latency_seconds is not None:
            try:
                self.latency_seconds.observe(latency, method=method)
            except Exception:  # pragma: no cover
                pass
        if verdict is not None and self.verdict_total is not None:
            try:
                self.verdict_total.inc(verdict=verdict)
            except Exception:  # pragma: no cover
                pass
        return latency
