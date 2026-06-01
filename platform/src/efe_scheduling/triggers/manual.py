"""Manual trigger — fires on explicit operator request."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from efe_scheduling.triggers.base import Decision, Fire, Hold, TriggerContext
from efe_scheduling.types import JobKind

__all__ = ["ManualSignal", "ManualTrigger"]


@dataclass(frozen=True, slots=True)
class ManualSignal:
    actor: str
    idempotency_key: str
    payload: dict[str, Any] = field(default_factory=dict)
    reason: str = "manual"


class ManualTrigger:
    __slots__ = ("enabled", "job_kind", "task_id", "tenant_id", "trigger_id")

    def __init__(
        self,
        *,
        trigger_id: str,
        tenant_id: str,
        task_id: str | None,
        job_kind: JobKind,
        enabled: bool = True,
    ) -> None:
        self.trigger_id = trigger_id
        self.tenant_id = tenant_id
        self.task_id = task_id
        self.job_kind = job_kind
        self.enabled = enabled

    def should_fire(self, ctx: TriggerContext) -> Decision:
        if not self.enabled:
            return Hold(reason="trigger disabled")
        sig = ctx.signal
        if not isinstance(sig, ManualSignal):
            return Hold(reason="not a ManualSignal")
        payload = {
            "source": "manual",
            "actor": sig.actor,
            "idempotency_key": sig.idempotency_key,
            **sig.payload,
        }
        return Fire(payload=payload, reason=sig.reason)
