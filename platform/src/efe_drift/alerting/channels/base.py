"""Channel Protocol + shared payload shape + registry.

Every channel is a concrete implementation of :class:`AlertChannel`.
Channels expose :attr:`name` (registry key, e.g. ``"slack"``) and
``async dispatch(payload)`` which returns a :class:`DispatchResult`.

The :class:`IncidentPayload` is the channel-agnostic dict produced
by :func:`build_incident_payload`. Individual channels map it to the
vendor-specific format (Slack blocks, PagerDuty Events API v2, etc.).

Design : channels **never** raise on failure. They return a
:class:`DispatchResult` with ``ok=False`` plus an error string. The
router records both in the incident's ``dispatch_results`` ledger.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime
from typing import Any, Protocol

from efe_drift.alerting.incident import AlertIncident
from efe_drift.alerting.rule import AlertRule
from efe_drift.detection.base import DriftResult

# ---------------------------------------------------------------------------
# DispatchResult
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class DispatchResult:
    ok: bool
    channel: str
    delivered_at: datetime | None = None
    error: str | None = None
    extra: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        out: dict[str, Any] = {
            "ok": self.ok,
            "channel": self.channel,
        }
        if self.delivered_at is not None:
            out["delivered_at"] = self.delivered_at.isoformat()
        if self.error is not None:
            out["error"] = self.error
        if self.extra:
            out["extra"] = self.extra
        return out


# ---------------------------------------------------------------------------
# IncidentPayload
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class IncidentPayload:
    incident_id: str
    rule_id: str
    tenant_id: str
    task_id: str
    feature_name: str
    method: str
    severity: str
    score: float
    p_value: float | None
    window_kind: str | None
    window_start: str
    window_end: str
    baseline_id: str
    fired_at: str
    details: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {
            "incident_id": self.incident_id,
            "rule_id": self.rule_id,
            "tenant_id": self.tenant_id,
            "task_id": self.task_id,
            "feature_name": self.feature_name,
            "method": self.method,
            "severity": self.severity,
            "score": self.score,
            "p_value": self.p_value,
            "window_kind": self.window_kind,
            "window_start": self.window_start,
            "window_end": self.window_end,
            "baseline_id": self.baseline_id,
            "fired_at": self.fired_at,
            "details": self.details,
        }


def build_incident_payload(
    *, incident: AlertIncident, result: DriftResult, rule: AlertRule,
) -> IncidentPayload:
    return IncidentPayload(
        incident_id=incident.incident_id,
        rule_id=rule.rule_id,
        tenant_id=result.tenant_id,
        task_id=result.task_id,
        feature_name=result.feature_name,
        method=result.method,
        severity=result.severity,
        score=result.score,
        p_value=result.p_value,
        window_kind=result.multi_scale_window,
        window_start=result.window_start.isoformat(),
        window_end=result.window_end.isoformat(),
        baseline_id=result.baseline_id,
        fired_at=incident.fired_at.isoformat(),
        details=dict(result.details),
    )


# ---------------------------------------------------------------------------
# Channel Protocol
# ---------------------------------------------------------------------------


class AlertChannel(Protocol):
    @property
    def name(self) -> str:  # pragma: no cover
        ...

    async def dispatch(
        self, payload: IncidentPayload,
    ) -> DispatchResult:  # pragma: no cover
        ...


# ---------------------------------------------------------------------------
# ChannelRegistry
# ---------------------------------------------------------------------------


class ChannelRegistry:
    """Simple name → channel mapping."""

    __slots__ = ("_by_name",)

    def __init__(self) -> None:
        self._by_name: dict[str, AlertChannel] = {}

    def register(self, channel: AlertChannel) -> None:
        if channel.name in self._by_name:
            raise ValueError(
                f"channel {channel.name!r} already registered",
            )
        self._by_name[channel.name] = channel

    def has(self, name: str) -> bool:
        return name in self._by_name

    def get(self, name: str) -> AlertChannel:
        if name not in self._by_name:
            raise KeyError(f"channel {name!r} not registered")
        return self._by_name[name]

    def names(self) -> tuple[str, ...]:
        return tuple(sorted(self._by_name))


__all__ = [
    "AlertChannel",
    "ChannelRegistry",
    "DispatchResult",
    "IncidentPayload",
    "build_incident_payload",
]
