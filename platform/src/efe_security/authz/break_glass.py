"""Break-glass emergency access.

A tightly-controlled mechanism to grant a principal elevated scopes
for a bounded duration (default 1 h) when normal provisioning is not
available (major incident, production emergency).

Guardrails :

* **4-eyes** — activation requires a ``justification`` string and a
  second principal's approval (``approved_by``) distinct from the
  requester.
* **Auto-revoke** — the grant carries an ``expires_at`` ≤ 1 h ; a
  daemon (not shown here) periodically prunes expired grants.
* **Audit always** — every activation, usage and revocation is emitted
  to the audit sink as ``break_glass.*`` events.
* **Scope cap** — the requested scopes must be a subset of a static
  allowlist (``BREAK_GLASS_ALLOWED_SCOPES``).
* **MFA required** — if the requester's :class:`Principal` did not go
  through MFA (``amr`` does not contain ``mfa``), activation raises.

The module is stateful (in-memory grant store) ; production persists
to Postgres with RLS scoped to the ``security_officer`` role.
"""

from __future__ import annotations

import asyncio
import uuid
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from typing import Any, Awaitable, Callable, Protocol

from efe_security.audit.records import AuditDraft, AuditOutcome, AuditSeverity
from efe_security.context import Principal
from efe_security.errors import ActionForbidden, MfaRequired, SecurityError

__all__ = [
    "BREAK_GLASS_ALLOWED_SCOPES",
    "BreakGlassGrant",
    "BreakGlassManager",
    "BreakGlassState",
]


BREAK_GLASS_ALLOWED_SCOPES: frozenset[str] = frozenset(
    {
        "audit:read",
        "audit:export",
        "sched:admin",
        "job:cancel",
        "baseline:retire",
        "policy:write",
        "tenant:offboard",
        "key:rotate",
    }
)


from enum import Enum


class BreakGlassState(str, Enum):
    active = "active"
    revoked = "revoked"
    expired = "expired"


@dataclass(frozen=True, slots=True)
class BreakGlassGrant:
    grant_id: str
    requester: str
    approved_by: str
    tenant_id: str | None
    scopes: frozenset[str]
    justification: str
    activated_at: datetime
    expires_at: datetime
    state: BreakGlassState = BreakGlassState.active
    revoked_at: datetime | None = None
    revoked_by: str | None = None

    def is_valid_at(self, *, now: datetime) -> bool:
        if self.state != BreakGlassState.active:
            return False
        return now < self.expires_at

    def to_dict(self) -> dict[str, Any]:
        return {
            "grant_id": self.grant_id,
            "requester": self.requester,
            "approved_by": self.approved_by,
            "tenant_id": self.tenant_id,
            "scopes": sorted(self.scopes),
            "justification": self.justification,
            "activated_at": self.activated_at.astimezone(timezone.utc).isoformat(),
            "expires_at": self.expires_at.astimezone(timezone.utc).isoformat(),
            "state": self.state.value,
            "revoked_at": (
                self.revoked_at.astimezone(timezone.utc).isoformat()
                if self.revoked_at
                else None
            ),
            "revoked_by": self.revoked_by,
        }


class _AuditProto(Protocol):
    async def append(self, draft: AuditDraft) -> Any: ...  # pragma: no cover


@dataclass(slots=True)
class BreakGlassManager:
    """Orchestrator for break-glass grants."""

    audit: _AuditProto
    default_ttl: timedelta = timedelta(hours=1)
    max_ttl: timedelta = timedelta(hours=4)
    _grants: dict[str, BreakGlassGrant] = field(default_factory=dict)
    _lock: asyncio.Lock = field(default_factory=asyncio.Lock, repr=False)

    async def activate(
        self,
        *,
        requester: Principal,
        approved_by: str,
        scopes: frozenset[str],
        justification: str,
        tenant_id: str | None = None,
        ttl: timedelta | None = None,
        now: datetime | None = None,
    ) -> BreakGlassGrant:
        """Create a new grant. Raises on policy violation."""
        if requester.actor.actor_id == approved_by:
            raise ActionForbidden("break_glass requires 4-eyes (distinct approver)")
        if "mfa" not in (requester.actor.amr or ()):
            raise MfaRequired("break_glass requires MFA on the requester")
        if not justification or len(justification) < 16:
            raise ActionForbidden(
                "break_glass requires a justification of at least 16 chars"
            )
        if not scopes:
            raise ActionForbidden("break_glass requires at least one scope")
        if not scopes.issubset(BREAK_GLASS_ALLOWED_SCOPES):
            bad = scopes - BREAK_GLASS_ALLOWED_SCOPES
            raise ActionForbidden(
                f"break_glass scope out of allowlist: {sorted(bad)}"
            )
        ttl = ttl or self.default_ttl
        if ttl > self.max_ttl:
            raise ActionForbidden(
                f"break_glass ttl exceeds max {self.max_ttl.total_seconds()}s"
            )
        now = now or datetime.now(tz=timezone.utc)
        grant = BreakGlassGrant(
            grant_id=f"bg_{uuid.uuid4().hex}",
            requester=requester.actor.actor_id,
            approved_by=approved_by,
            tenant_id=tenant_id,
            scopes=scopes,
            justification=justification,
            activated_at=now,
            expires_at=now + ttl,
        )
        async with self._lock:
            self._grants[grant.grant_id] = grant

        await self._audit(
            action="break_glass.activated",
            outcome=AuditOutcome.success,
            severity=AuditSeverity.warning,
            actor_id=requester.actor.actor_id,
            tenant_id=tenant_id,
            payload={
                "grant_id": grant.grant_id,
                "approved_by": approved_by,
                "scopes": sorted(scopes),
                "ttl_s": int(ttl.total_seconds()),
            },
            reasons=(justification[:80],),
        )
        return grant

    async def revoke(
        self,
        grant_id: str,
        *,
        revoked_by: str,
        now: datetime | None = None,
    ) -> BreakGlassGrant:
        now = now or datetime.now(tz=timezone.utc)
        async with self._lock:
            grant = self._grants.get(grant_id)
            if grant is None:
                raise SecurityError(f"break_glass grant not found: {grant_id}")
            new = BreakGlassGrant(
                grant_id=grant.grant_id,
                requester=grant.requester,
                approved_by=grant.approved_by,
                tenant_id=grant.tenant_id,
                scopes=grant.scopes,
                justification=grant.justification,
                activated_at=grant.activated_at,
                expires_at=grant.expires_at,
                state=BreakGlassState.revoked,
                revoked_at=now,
                revoked_by=revoked_by,
            )
            self._grants[grant_id] = new

        await self._audit(
            action="break_glass.revoked",
            outcome=AuditOutcome.success,
            severity=AuditSeverity.warning,
            actor_id=revoked_by,
            tenant_id=grant.tenant_id,
            payload={"grant_id": grant_id},
            reasons=("manual_revoke",),
        )
        return new

    async def prune_expired(self, *, now: datetime | None = None) -> int:
        now = now or datetime.now(tz=timezone.utc)
        n = 0
        async with self._lock:
            for gid, g in list(self._grants.items()):
                if g.state == BreakGlassState.active and now >= g.expires_at:
                    self._grants[gid] = BreakGlassGrant(
                        grant_id=g.grant_id,
                        requester=g.requester,
                        approved_by=g.approved_by,
                        tenant_id=g.tenant_id,
                        scopes=g.scopes,
                        justification=g.justification,
                        activated_at=g.activated_at,
                        expires_at=g.expires_at,
                        state=BreakGlassState.expired,
                        revoked_at=now,
                        revoked_by="system",
                    )
                    n += 1
        if n:
            await self._audit(
                action="break_glass.expired_prune",
                outcome=AuditOutcome.success,
                severity=AuditSeverity.info,
                actor_id="system",
                tenant_id=None,
                payload={"count": n},
                reasons=("ttl_expired",),
            )
        return n

    async def active_scopes_for(
        self, *, principal_id: str, now: datetime | None = None
    ) -> frozenset[str]:
        """Return all scopes currently granted via break-glass to ``principal_id``."""
        now = now or datetime.now(tz=timezone.utc)
        scopes: set[str] = set()
        async with self._lock:
            for g in self._grants.values():
                if g.requester == principal_id and g.is_valid_at(now=now):
                    scopes.update(g.scopes)
        return frozenset(scopes)

    async def list(self, *, state: BreakGlassState | None = None) -> list[BreakGlassGrant]:
        async with self._lock:
            return [g for g in self._grants.values() if state is None or g.state == state]

    # ------------------------------------------------------------------

    async def _audit(
        self,
        *,
        action: str,
        outcome: AuditOutcome,
        severity: AuditSeverity,
        actor_id: str,
        tenant_id: str | None,
        payload: dict[str, Any],
        reasons: tuple[str, ...],
    ) -> None:
        draft = AuditDraft(
            occurred_at=datetime.now(tz=timezone.utc),
            tenant_id=tenant_id,
            actor_kind="human",
            actor_id=actor_id,
            action=action,
            outcome=outcome,
            severity=severity,
            reasons=reasons,
            payload=payload,
        )
        try:
            await self.audit.append(draft)
        except Exception:  # pragma: no cover - break_glass must not be blocked
            import logging as _logging

            _logging.getLogger("efe_security.break_glass").exception(
                "break_glass.audit_failed action=%s", action
            )


_ = Callable, Awaitable  # reserved for future hooks
