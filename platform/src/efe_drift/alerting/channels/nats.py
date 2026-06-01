"""NATS alert channel — publishes on ``efe.drift.alert``.

M08 guardrails and M12 dashboards subscribe to this subject to
surface drift alerts in other parts of the platform without going
through webhooks.
"""

from __future__ import annotations

from dataclasses import dataclass, field

from efe_drift.alerting.channels.base import (
    DispatchResult,
    IncidentPayload,
)
from efe_drift.clock import Clock, SystemClock
from efe_drift.nats.publisher import Publisher


@dataclass(frozen=True, slots=True)
class NatsAlertChannel:
    publisher: Publisher
    subject: str = "efe.drift.alert"
    clock: Clock = field(default_factory=SystemClock)
    _name: str = "nats"

    @property
    def name(self) -> str:
        return self._name

    async def dispatch(self, payload: IncidentPayload) -> DispatchResult:
        try:
            await self.publisher.publish(self.subject, payload.to_dict())
        except Exception as exc:
            return DispatchResult(
                ok=False, channel=self._name,
                error=f"{type(exc).__name__}: {exc}",
            )
        return DispatchResult(
            ok=True, channel=self._name,
            delivered_at=self.clock.now(),
            extra={"subject": self.subject},
        )


__all__ = ["NatsAlertChannel"]
