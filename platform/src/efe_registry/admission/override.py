"""Break-glass override tracker.

Whenever an SRE disables policy-controller enforcement on a namespace,
we record a :class:`BreakGlassOverride`. The tracker offers:

* :meth:`fire` — log the override, start TTL timer, emit audit event;
* :meth:`expire_stale` — revoke anything past TTL (called by cron);
* :meth:`active` — the list of currently-open overrides.
"""

from __future__ import annotations

import uuid
from dataclasses import dataclass, field
from datetime import datetime, timedelta
from typing import Protocol

from efe_registry.clock import Clock, SystemClock
from efe_registry.config import AdmissionSettings
from efe_registry.errors import BreakGlassExpired


@dataclass(frozen=True, slots=True)
class BreakGlassOverride:
    override_id: str
    namespace: str
    actor: str
    reason: str
    fired_at: datetime
    expires_at: datetime
    revoked_at: datetime | None = None


class OverrideAuditSink(Protocol):
    async def record_override(
        self, override: BreakGlassOverride,
    ) -> None:  # pragma: no cover
        ...


@dataclass
class InMemoryOverrideAudit:
    events: list[BreakGlassOverride] = field(default_factory=list)

    async def record_override(self, override: BreakGlassOverride) -> None:
        self.events.append(override)


class BreakGlassTracker:
    def __init__(
        self,
        *,
        settings: AdmissionSettings,
        audit: OverrideAuditSink,
        clock: Clock | None = None,
    ) -> None:
        self._cfg = settings
        self._audit = audit
        self._clock: Clock = clock or SystemClock()
        self._open: dict[str, BreakGlassOverride] = {}

    async def fire(
        self, *, namespace: str, actor: str, reason: str,
    ) -> BreakGlassOverride:
        now = self._clock.now()
        override = BreakGlassOverride(
            override_id=str(uuid.uuid4()),
            namespace=namespace,
            actor=actor,
            reason=reason,
            fired_at=now,
            expires_at=now + timedelta(seconds=self._cfg.break_glass_ttl_s),
        )
        self._open[override.override_id] = override
        await self._audit.record_override(override)
        return override

    def active(self) -> tuple[BreakGlassOverride, ...]:
        now = self._clock.now()
        return tuple(
            o for o in self._open.values()
            if o.revoked_at is None and o.expires_at > now
        )

    async def expire_stale(self) -> tuple[BreakGlassOverride, ...]:
        now = self._clock.now()
        expired: list[BreakGlassOverride] = []
        for override_id, override in list(self._open.items()):
            if override.revoked_at is not None:
                continue
            if override.expires_at <= now:
                revoked = BreakGlassOverride(
                    override_id=override.override_id,
                    namespace=override.namespace,
                    actor=override.actor,
                    reason=override.reason,
                    fired_at=override.fired_at,
                    expires_at=override.expires_at,
                    revoked_at=now,
                )
                self._open[override_id] = revoked
                expired.append(revoked)
                await self._audit.record_override(revoked)
        return tuple(expired)

    def revoke_manually(
        self, override_id: str, *, actor: str,
    ) -> BreakGlassOverride:
        existing = self._open.get(override_id)
        if existing is None:
            raise BreakGlassExpired(
                f"override {override_id!r} unknown",
            )
        now = self._clock.now()
        revoked = BreakGlassOverride(
            override_id=existing.override_id,
            namespace=existing.namespace,
            actor=f"manual:{actor}",
            reason=existing.reason + " (revoked)",
            fired_at=existing.fired_at,
            expires_at=existing.expires_at,
            revoked_at=now,
        )
        self._open[override_id] = revoked
        return revoked


__all__ = [
    "BreakGlassOverride",
    "BreakGlassTracker",
    "InMemoryOverrideAudit",
    "OverrideAuditSink",
]
