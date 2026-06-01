"""AlertIncident — FSM + store.

States :

* ``firing``   — alert is actively breaching
* ``acked``    — operator acknowledged; suppress dispatch until
                  ``resolved``
* ``resolved`` — no breaches for the resolve window

Transitions (see M09 §5.2) :

* ``firing   → acked``    — operator ack via API / CLI
* ``firing   → resolved`` — scheduler detects window_passed
* ``acked    → resolved`` — same

Terminal state : ``resolved``.
"""

from __future__ import annotations

import asyncio
from dataclasses import dataclass, field, replace
from datetime import datetime
from typing import Any, Literal, Protocol

from efe_drift.clock import Clock, SystemClock
from efe_drift.detection.base import DriftResult, MultiScaleWindow
from efe_drift.detection.severity import Severity
from efe_drift.errors import NotFound, ValidationError
from efe_drift.ids import new_uuid

IncidentState = Literal["firing", "acked", "resolved"]

_TRANSITIONS: dict[tuple[IncidentState, IncidentState], str] = {
    ("firing", "acked"):     "ack",
    ("firing", "resolved"):  "resolve_auto",
    ("acked", "resolved"):   "resolve_auto",
}


class IncidentFsm:
    """Pure FSM — no I/O, no persistence."""

    TERMINAL: frozenset[IncidentState] = frozenset({"resolved"})

    @classmethod
    def is_terminal(cls, state: IncidentState) -> bool:
        return state in cls.TERMINAL

    @classmethod
    def validate(
        cls, from_state: IncidentState, to_state: IncidentState,
    ) -> None:
        if from_state == to_state:
            raise ValidationError(
                f"no-op transition {from_state!r} → {to_state!r}",
            )
        if cls.is_terminal(from_state):
            raise ValidationError(
                f"{from_state!r} is terminal — cannot transition",
            )
        if (from_state, to_state) not in _TRANSITIONS:
            raise ValidationError(
                f"{from_state!r} → {to_state!r} is not allowed",
            )


# ---------------------------------------------------------------------------
# AlertIncident
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class AlertIncident:
    incident_id: str
    rule_id: str
    tenant_id: str
    feature_id: str
    feature_name: str
    method: str
    severity: Severity
    score_at_fire: float
    fired_at: datetime
    state: IncidentState = "firing"
    acked_at: datetime | None = None
    acked_by: str | None = None
    resolved_at: datetime | None = None
    dispatch_results: dict[str, Any] = field(default_factory=dict)
    window_kind: MultiScaleWindow | None = None


# ---------------------------------------------------------------------------
# Store Protocol + in-memory impl
# ---------------------------------------------------------------------------


class AlertIncidentStore(Protocol):
    async def create(
        self, *, rule_id: str, result: DriftResult, now: datetime,
    ) -> AlertIncident:  # pragma: no cover
        ...

    async def get(
        self, incident_id: str,
    ) -> AlertIncident:  # pragma: no cover
        ...

    async def ack(
        self, incident_id: str, *, actor: str, now: datetime,
    ) -> AlertIncident:  # pragma: no cover
        ...

    async def resolve(
        self, incident_id: str, *, now: datetime,
    ) -> AlertIncident:  # pragma: no cover
        ...

    async def record_dispatch(
        self, incident_id: str, *, channel: str, result: dict[str, Any],
    ) -> AlertIncident:  # pragma: no cover
        ...

    async def list(
        self,
        *,
        tenant_id: str | None = None,
        state: IncidentState | None = None,
    ) -> tuple[AlertIncident, ...]:  # pragma: no cover
        ...

    async def find_active(
        self,
        *,
        rule_id: str,
        feature_id: str,
        severity: Severity,
    ) -> AlertIncident | None:  # pragma: no cover
        ...


class InMemoryAlertIncidentStore:
    __slots__ = ("_by_id", "_clock", "_lock")

    def __init__(self, *, clock: Clock | None = None) -> None:
        self._by_id: dict[str, AlertIncident] = {}
        self._clock = clock or SystemClock()
        self._lock = asyncio.Lock()

    async def create(
        self, *, rule_id: str, result: DriftResult, now: datetime,
    ) -> AlertIncident:
        async with self._lock:
            incident = AlertIncident(
                incident_id=new_uuid(),
                rule_id=rule_id,
                tenant_id=result.tenant_id,
                feature_id=result.feature_id,
                feature_name=result.feature_name,
                method=result.method,
                severity=result.severity,
                score_at_fire=result.score,
                fired_at=now,
                state="firing",
                window_kind=result.multi_scale_window,
            )
            self._by_id[incident.incident_id] = incident
            return incident

    async def get(self, incident_id: str) -> AlertIncident:
        incident = self._by_id.get(incident_id)
        if incident is None:
            raise NotFound(f"unknown incident_id {incident_id!r}")
        return incident

    async def ack(
        self, incident_id: str, *, actor: str, now: datetime,
    ) -> AlertIncident:
        async with self._lock:
            current = self._by_id.get(incident_id)
            if current is None:
                raise NotFound(f"unknown incident_id {incident_id!r}")
            IncidentFsm.validate(current.state, "acked")
            updated = replace(
                current,
                state="acked",
                acked_at=now,
                acked_by=actor,
            )
            self._by_id[incident_id] = updated
            return updated

    async def resolve(
        self, incident_id: str, *, now: datetime,
    ) -> AlertIncident:
        async with self._lock:
            current = self._by_id.get(incident_id)
            if current is None:
                raise NotFound(f"unknown incident_id {incident_id!r}")
            if current.state == "resolved":
                return current
            IncidentFsm.validate(current.state, "resolved")
            updated = replace(
                current,
                state="resolved",
                resolved_at=now,
            )
            self._by_id[incident_id] = updated
            return updated

    async def record_dispatch(
        self, incident_id: str, *, channel: str, result: dict[str, Any],
    ) -> AlertIncident:
        async with self._lock:
            current = self._by_id.get(incident_id)
            if current is None:
                raise NotFound(f"unknown incident_id {incident_id!r}")
            merged = dict(current.dispatch_results)
            merged[channel] = result
            updated = replace(current, dispatch_results=merged)
            self._by_id[incident_id] = updated
            return updated

    async def list(
        self,
        *,
        tenant_id: str | None = None,
        state: IncidentState | None = None,
    ) -> tuple[AlertIncident, ...]:
        rows = tuple(self._by_id.values())
        if tenant_id is not None:
            rows = tuple(i for i in rows if i.tenant_id == tenant_id)
        if state is not None:
            rows = tuple(i for i in rows if i.state == state)
        return rows

    async def find_active(
        self,
        *,
        rule_id: str,
        feature_id: str,
        severity: Severity,
    ) -> AlertIncident | None:
        for inc in self._by_id.values():
            if (
                inc.rule_id == rule_id
                and inc.feature_id == feature_id
                and inc.severity == severity
                and inc.state in ("firing", "acked")
            ):
                return inc
        return None


__all__ = [
    "AlertIncident",
    "AlertIncidentStore",
    "InMemoryAlertIncidentStore",
    "IncidentFsm",
    "IncidentState",
]
