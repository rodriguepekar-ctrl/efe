"""SLO gateway — abstracts the M07 live-metrics provider.

The gateway returns a :class:`SloBreach` object; callers only care
whether ``breached`` is true, but the breakdown helps observability /
debug.

The production implementation queries Prometheus (via PromQL) through
an async HTTP client. The in-memory implementation here is used by
tests and scripted demos.
"""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, field
from typing import Protocol


@dataclass(frozen=True, slots=True)
class SloBreach:
    breached: bool
    latency_p95_ratio: float
    error_rate_ratio: float
    quality_delta: float
    details: Mapping[str, float] = field(default_factory=dict)


class SloGateway(Protocol):
    async def evaluate(
        self,
        *,
        tenant_id: str,
        task_id: str,
        candidate_artifact: str,
        baseline_artifact: str,
        window_s: int,
    ) -> SloBreach:  # pragma: no cover
        ...


@dataclass
class InMemorySloGateway:
    """Scripted gateway — returns pre-configured breaches."""

    breaches: dict[tuple[str, str], SloBreach] = field(default_factory=dict)
    default: SloBreach = field(
        default_factory=lambda: SloBreach(
            breached=False,
            latency_p95_ratio=1.0,
            error_rate_ratio=1.0,
            quality_delta=0.0,
        ),
    )
    calls: list[tuple[str, str, str, str, int]] = field(
        default_factory=list,
    )

    async def evaluate(
        self,
        *,
        tenant_id: str,
        task_id: str,
        candidate_artifact: str,
        baseline_artifact: str,
        window_s: int,
    ) -> SloBreach:
        self.calls.append((
            tenant_id, task_id, candidate_artifact,
            baseline_artifact, window_s,
        ))
        return self.breaches.get((tenant_id, task_id), self.default)

    # Test helpers
    def set_breach(
        self,
        *,
        tenant_id: str,
        task_id: str,
        latency_ratio: float = 2.0,
        error_ratio: float = 3.0,
        quality_delta: float = -0.05,
    ) -> None:
        self.breaches[(tenant_id, task_id)] = SloBreach(
            breached=True,
            latency_p95_ratio=latency_ratio,
            error_rate_ratio=error_ratio,
            quality_delta=quality_delta,
        )

    def clear(self, *, tenant_id: str, task_id: str) -> None:
        self.breaches.pop((tenant_id, task_id), None)


__all__ = [
    "InMemorySloGateway",
    "SloBreach",
    "SloGateway",
]
