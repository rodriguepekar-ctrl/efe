"""PagerDuty Events API v2 channel.

PagerDuty dedupes on ``dedup_key``. We build a stable key from
``tenant_id:task_id:feature_name:method:window_kind:severity`` so
re-firing the same incident updates the existing PD alert rather
than opening a new one.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from efe_drift.alerting.channels.base import (
    DispatchResult,
    IncidentPayload,
)
from efe_drift.alerting.channels.slack import HttpPoster
from efe_drift.clock import Clock, SystemClock

PAGERDUTY_EVENTS_URL = "https://events.pagerduty.com/v2/enqueue"


@dataclass(frozen=True, slots=True)
class PagerDutyChannel:
    """PagerDuty Events v2 channel."""

    routing_key: str
    http_poster: HttpPoster
    source: str = "efe-drift"
    events_url: str = PAGERDUTY_EVENTS_URL
    clock: Clock = field(default_factory=SystemClock)
    _name: str = "pagerduty"

    @property
    def name(self) -> str:
        return self._name

    async def dispatch(self, payload: IncidentPayload) -> DispatchResult:
        body = self._render(payload)
        try:
            status, text = await self.http_poster(self.events_url, body)
        except Exception as exc:
            return DispatchResult(
                ok=False, channel=self._name,
                error=f"{type(exc).__name__}: {exc}",
            )
        if status >= 300:
            return DispatchResult(
                ok=False, channel=self._name,
                error=f"status={status}, body={text[:200]!r}",
            )
        return DispatchResult(
            ok=True, channel=self._name,
            delivered_at=self.clock.now(),
            extra={"dedup_key": body["dedup_key"]},
        )

    def _render(self, p: IncidentPayload) -> dict[str, Any]:
        dedup_key = (
            f"{p.tenant_id}:{p.task_id}:{p.feature_name}:{p.method}:"
            f"{p.window_kind}:{p.severity}"
        )
        return {
            "routing_key": self.routing_key,
            "event_action": "trigger",
            "dedup_key": dedup_key,
            "payload": {
                "summary": (
                    f"EFE drift — {p.severity} "
                    f"({p.tenant_id}/{p.task_id}/{p.feature_name})"
                ),
                "source": self.source,
                "severity": _map_severity(p.severity),
                "component": p.feature_name,
                "group": p.tenant_id,
                "class": p.method,
                "custom_details": p.to_dict(),
            },
        }


def _map_severity(severity: str) -> str:
    # PagerDuty accepts critical / error / warning / info.
    if severity == "critical":
        return "critical"
    if severity == "major":
        return "error"
    return "warning"


__all__ = ["PAGERDUTY_EVENTS_URL", "PagerDutyChannel"]
