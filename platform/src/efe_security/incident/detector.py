"""Lightweight counter-based detectors."""

from __future__ import annotations

import asyncio
from dataclasses import dataclass, field
from datetime import datetime, timedelta
from typing import Protocol

__all__ = ["Detector", "DetectorOutcome", "IncidentDraft", "SimpleCounterDetector"]


@dataclass(frozen=True, slots=True)
class IncidentDraft:
    category: str
    severity: str  # "low"|"medium"|"high"|"critical"
    summary: str
    tenant_id: str | None = None
    payload: dict = field(default_factory=dict)


@dataclass(frozen=True, slots=True)
class DetectorOutcome:
    incidents: list[IncidentDraft]


class Detector(Protocol):
    async def observe(
        self,
        *,
        kind: str,
        labels: dict[str, str],
        value: float,
        now: datetime,
    ) -> DetectorOutcome: ...  # pragma: no cover


@dataclass(slots=True)
class SimpleCounterDetector:
    """Counts occurrences of a ``kind`` keyed by labels and fires when the
    count over a rolling window crosses a threshold.
    """

    window_s: int
    threshold: int
    category: str
    severity: str
    summary_template: str = "{kind} count={count} threshold={threshold}"
    _events: dict[tuple[str, tuple[tuple[str, str], ...]], list[datetime]] = field(
        default_factory=dict
    )
    _lock: asyncio.Lock = field(default_factory=asyncio.Lock, repr=False)

    async def observe(
        self,
        *,
        kind: str,
        labels: dict[str, str],
        value: float,
        now: datetime,
    ) -> DetectorOutcome:
        del value  # Not used yet ; kept for API parity.
        key = (kind, tuple(sorted(labels.items())))
        async with self._lock:
            bucket = self._events.setdefault(key, [])
            bucket.append(now)
            horizon = now - timedelta(seconds=self.window_s)
            bucket[:] = [t for t in bucket if t >= horizon]
            count = len(bucket)
        if count < self.threshold:
            return DetectorOutcome(incidents=[])
        # Reset the counter to avoid a flood of incidents from the same window.
        async with self._lock:
            self._events[key].clear()
        return DetectorOutcome(
            incidents=[
                IncidentDraft(
                    category=self.category,
                    severity=self.severity,
                    summary=self.summary_template.format(
                        kind=kind, count=count, threshold=self.threshold
                    ),
                    tenant_id=labels.get("tenant_id"),
                    payload={"labels": dict(labels), "count": count},
                )
            ]
        )
