"""Public types the host application receives from :class:`EfeFacade`.

The hot-path return of :meth:`EfeFacade.classify` is a :class:`Verdict`.
The host never touches :class:`efe_tool.types.ClassifyResult` (internal).

See LETTER_D_ARCHITECTURE.md §5.1.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Mapping, Optional


# ---------------------------------------------------------------------------
#  Verdict
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class Verdict:
    """One classification verdict ready for host consumption.

    Attributes are a deliberate **reduction** of
    :class:`efe_tool.types.ClassifyResult` — only fields a business
    app legitimately needs appear.
    """

    label: str
    confidence: float
    reject: bool
    rounds: int
    alignment_delta: float
    catalog: str
    zone: str                                   # "trusted" | "warn" | "reject"
    elapsed_ms: int
    request_id: Optional[str] = None

    # ------------------------------------------------------------------
    #  Conveniences
    # ------------------------------------------------------------------

    @property
    def is_trusted(self) -> bool:
        return not self.reject and self.zone == "trusted"

    @property
    def is_warn(self) -> bool:
        return not self.reject and self.zone == "warn"

    @property
    def is_reject(self) -> bool:
        return self.reject or self.zone == "reject"

    def as_dict(self) -> dict[str, Any]:
        """JSON-serialisable projection, safe for HTTP responses."""
        return {
            "label": self.label,
            "confidence": round(self.confidence, 6),
            "reject": self.reject,
            "rounds": self.rounds,
            "alignment_delta": round(self.alignment_delta, 6),
            "catalog": self.catalog,
            "zone": self.zone,
            "elapsed_ms": self.elapsed_ms,
            "request_id": self.request_id,
        }


# ---------------------------------------------------------------------------
#  HealthStatus
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class HealthStatus:
    """Result of :meth:`EfeFacade.healthcheck`.

    Used by HTTP hosts to build ``/healthz`` / ``/readyz`` endpoints.
    """

    status: str                                 # "SERVING" | "NOT_SERVING" | "NOT_READY"
    latency_ms: int
    checks: Mapping[str, bool]
    reason: Optional[str] = None

    @property
    def serving(self) -> bool:
        return self.status == "SERVING"

    def as_dict(self) -> dict[str, Any]:
        return {
            "status": self.status,
            "latency_ms": self.latency_ms,
            "checks": dict(self.checks),
            "reason": self.reason,
        }


# ---------------------------------------------------------------------------
#  FacadeStats
# ---------------------------------------------------------------------------


@dataclass
class FacadeStats:
    """Running statistics snapshot — exposed via :meth:`EfeFacade.stats`."""

    classify_total: int = 0
    classify_failed: int = 0
    classify_rejected: int = 0
    in_flight: int = 0
    latency_sum_ms: float = 0.0
    latency_count: int = 0
    latency_p50_ms_last_window: float = 0.0
    latency_p95_ms_last_window: float = 0.0
    preload_ms: int = 0
    is_ready: bool = False
    is_closed: bool = False
    catalogs_loaded: tuple[str, ...] = ()
    model: str = ""
    endpoint: str = ""

    @property
    def avg_latency_ms(self) -> float:
        if self.latency_count == 0:
            return 0.0
        return round(self.latency_sum_ms / self.latency_count, 3)

    def as_dict(self) -> dict[str, Any]:
        return {
            "classify_total": self.classify_total,
            "classify_failed": self.classify_failed,
            "classify_rejected": self.classify_rejected,
            "in_flight": self.in_flight,
            "avg_latency_ms": self.avg_latency_ms,
            "latency_p50_ms": round(self.latency_p50_ms_last_window, 3),
            "latency_p95_ms": round(self.latency_p95_ms_last_window, 3),
            "preload_ms": self.preload_ms,
            "is_ready": self.is_ready,
            "is_closed": self.is_closed,
            "catalogs_loaded": list(self.catalogs_loaded),
            "model": self.model,
            "endpoint": self.endpoint,
        }


__all__ = ["Verdict", "HealthStatus", "FacadeStats"]
