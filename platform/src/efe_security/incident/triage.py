"""Incident record, state machine and triage service."""

from __future__ import annotations

import asyncio
import enum
import uuid
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any

from efe_security.crypto import DOMAIN_TAG_INCIDENT_SEAL, hmac_seal
from efe_security.errors import SecurityError
from efe_security.fsm import INCIDENT_ALLOWED, guard
from efe_security.incident.detector import IncidentDraft

__all__ = [
    "IncidentCategory",
    "IncidentRecord",
    "IncidentService",
    "IncidentSeverity",
    "IncidentState",
]


class IncidentSeverity(str, enum.Enum):
    low = "low"
    medium = "medium"
    high = "high"
    critical = "critical"


class IncidentCategory(str, enum.Enum):
    authn = "authn"
    authz = "authz"
    pii = "pii"
    key = "key"
    chain = "chain"
    tenancy = "tenancy"
    network = "network"
    other = "other"


class IncidentState(str, enum.Enum):
    detected = "detected"
    triaged = "triaged"
    contained = "contained"
    escalated = "escalated"
    resolved = "resolved"


@dataclass(frozen=True, slots=True)
class IncidentRecord:
    incident_id: str
    tenant_id: str | None
    detected_at: datetime
    severity: IncidentSeverity
    category: IncidentCategory
    summary: str
    payload: dict[str, Any]
    state: IncidentState
    notified_at: datetime | None
    resolved_at: datetime | None
    seal: bytes

    def to_dict(self) -> dict[str, Any]:
        return {
            "incident_id": self.incident_id,
            "tenant_id": self.tenant_id,
            "detected_at": self.detected_at.astimezone(timezone.utc).isoformat(),
            "severity": self.severity.value,
            "category": self.category.value,
            "summary": self.summary,
            "payload": self.payload,
            "state": self.state.value,
            "notified_at": (
                self.notified_at.astimezone(timezone.utc).isoformat()
                if self.notified_at
                else None
            ),
            "resolved_at": (
                self.resolved_at.astimezone(timezone.utc).isoformat()
                if self.resolved_at
                else None
            ),
            "seal": self.seal.hex(),
        }


@dataclass(slots=True)
class IncidentService:
    _rows: dict[str, IncidentRecord] = field(default_factory=dict)
    _lock: asyncio.Lock = field(default_factory=asyncio.Lock, repr=False)

    async def record(
        self,
        draft: IncidentDraft,
        *,
        now: datetime | None = None,
    ) -> IncidentRecord:
        now = now or datetime.now(tz=timezone.utc)
        try:
            severity = IncidentSeverity(draft.severity)
        except ValueError as exc:
            raise SecurityError(f"invalid severity: {draft.severity}") from exc
        try:
            category = IncidentCategory(draft.category)
        except ValueError:
            category = IncidentCategory.other
        incident_id = f"inc_{uuid.uuid4().hex}"
        view = {
            "incident_id": incident_id,
            "tenant_id": draft.tenant_id,
            "detected_at": now.astimezone(timezone.utc).isoformat(),
            "severity": severity.value,
            "category": category.value,
            "summary": draft.summary,
            "payload": draft.payload,
        }
        seal = hmac_seal(DOMAIN_TAG_INCIDENT_SEAL, view)
        record = IncidentRecord(
            incident_id=incident_id,
            tenant_id=draft.tenant_id,
            detected_at=now,
            severity=severity,
            category=category,
            summary=draft.summary,
            payload=dict(draft.payload),
            state=IncidentState.detected,
            notified_at=None,
            resolved_at=None,
            seal=seal,
        )
        async with self._lock:
            self._rows[incident_id] = record
        return record

    async def transition(
        self,
        incident_id: str,
        *,
        to_state: IncidentState,
        now: datetime | None = None,
    ) -> IncidentRecord:
        now = now or datetime.now(tz=timezone.utc)
        async with self._lock:
            r = self._rows.get(incident_id)
            if r is None:
                raise SecurityError(f"incident {incident_id} not found")
            guard(r.state.value, to_state.value, INCIDENT_ALLOWED, aggregate="incident")
            new = IncidentRecord(
                incident_id=r.incident_id,
                tenant_id=r.tenant_id,
                detected_at=r.detected_at,
                severity=r.severity,
                category=r.category,
                summary=r.summary,
                payload=r.payload,
                state=to_state,
                notified_at=r.notified_at,
                resolved_at=now if to_state == IncidentState.resolved else r.resolved_at,
                seal=r.seal,
            )
            self._rows[incident_id] = new
            return new

    async def list(self, *, state: IncidentState | None = None) -> list[IncidentRecord]:
        async with self._lock:
            return [r for r in self._rows.values() if state is None or r.state == state]

    async def get(self, incident_id: str) -> IncidentRecord:
        async with self._lock:
            r = self._rows.get(incident_id)
        if r is None:
            raise SecurityError(f"incident {incident_id} not found")
        return r
