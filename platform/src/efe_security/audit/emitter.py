"""High-level audit emission helpers.

Rather than hand-building :class:`AuditDraft` everywhere, callers use
the :class:`AuditEmitter` methods that encode the conventions :

* stable ``action`` strings,
* proper severity mapping per category,
* automatic tenant + actor extraction from a :class:`SecurityContext`.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any, Protocol

from efe_security.audit.records import AuditDraft, AuditOutcome, AuditSeverity
from efe_security.authz.pdp import PolicyDecision, PolicyRequest
from efe_security.context import SecurityContext

__all__ = ["AuditEmitter", "AuditSink"]


class AuditSink(Protocol):
    async def append(self, draft: AuditDraft) -> Any: ...  # pragma: no cover


@dataclass(slots=True)
class AuditEmitter:
    """Typed façade over an :class:`AuditSink`."""

    sink: AuditSink

    # ------------------------------------------------------------------
    # Generic
    # ------------------------------------------------------------------

    async def _emit(
        self,
        *,
        ctx: SecurityContext | None,
        action: str,
        outcome: AuditOutcome,
        severity: AuditSeverity = AuditSeverity.info,
        resource_kind: str | None = None,
        resource_id: str | None = None,
        reasons: tuple[str, ...] = (),
        payload: dict[str, Any] | None = None,
    ) -> None:
        if ctx is not None:
            actor_kind = ctx.principal.actor.kind.value
            actor_id = ctx.principal.actor.actor_id
            tenant_id = ctx.principal.tenant_id
        else:
            actor_kind = "system"
            actor_id = "platform"
            tenant_id = None
        draft = AuditDraft(
            occurred_at=datetime.now(tz=timezone.utc),
            tenant_id=tenant_id,
            actor_kind=actor_kind,
            actor_id=actor_id,
            action=action,
            outcome=outcome,
            severity=severity,
            resource_kind=resource_kind,
            resource_id=resource_id,
            reasons=reasons,
            payload=dict(payload or {}),
        )
        await self.sink.append(draft)

    # ------------------------------------------------------------------
    # AuthZ decisions
    # ------------------------------------------------------------------

    async def authz_decision(
        self,
        *,
        ctx: SecurityContext,
        request: PolicyRequest,
        decision: PolicyDecision,
    ) -> None:
        action = (
            f"authz.{request.action}.{'allow' if decision.allow else 'deny'}"
        )
        outcome = AuditOutcome.allow if decision.allow else AuditOutcome.deny
        severity = AuditSeverity.info if decision.allow else AuditSeverity.warning
        await self._emit(
            ctx=ctx,
            action=action,
            outcome=outcome,
            severity=severity,
            resource_kind=(request.resource.kind if request.resource is not None else None),
            resource_id=(request.resource.id if request.resource is not None else None),
            reasons=decision.reasons,
            payload={
                "eval_ms": decision.eval_ms,
                "obligations": [o.kind for o in decision.obligations],
            },
        )

    # ------------------------------------------------------------------
    # Consent / DSAR
    # ------------------------------------------------------------------

    async def consent_recorded(
        self,
        *,
        ctx: SecurityContext,
        consent_id: str,
        subject_id: str,
        purposes: tuple[str, ...],
    ) -> None:
        await self._emit(
            ctx=ctx,
            action="consent.recorded",
            outcome=AuditOutcome.success,
            resource_kind="consent",
            resource_id=consent_id,
            payload={"subject_id": subject_id, "purposes": list(purposes)},
        )

    async def consent_withdrawn(
        self, *, ctx: SecurityContext | None, consent_id: str
    ) -> None:
        await self._emit(
            ctx=ctx,
            action="consent.withdrawn",
            outcome=AuditOutcome.success,
            severity=AuditSeverity.warning,
            resource_kind="consent",
            resource_id=consent_id,
        )

    async def dsar_submitted(
        self, *, ctx: SecurityContext, request_id: str, kind: str
    ) -> None:
        await self._emit(
            ctx=ctx,
            action="dsar.submitted",
            outcome=AuditOutcome.success,
            resource_kind="dsar",
            resource_id=request_id,
            payload={"kind": kind},
        )

    async def dsar_completed(
        self, *, ctx: SecurityContext | None, request_id: str, kind: str
    ) -> None:
        await self._emit(
            ctx=ctx,
            action="dsar.completed",
            outcome=AuditOutcome.success,
            severity=AuditSeverity.info,
            resource_kind="dsar",
            resource_id=request_id,
            payload={"kind": kind},
        )

    # ------------------------------------------------------------------
    # Tenancy
    # ------------------------------------------------------------------

    async def tenant_onboarded(
        self, *, ctx: SecurityContext, tenant_id: str, tier: str
    ) -> None:
        await self._emit(
            ctx=ctx,
            action="tenant.onboarded",
            outcome=AuditOutcome.success,
            resource_kind="tenant",
            resource_id=tenant_id,
            payload={"tier": tier},
        )

    async def tenant_offboarded(
        self, *, ctx: SecurityContext, tenant_id: str, mode: str
    ) -> None:
        await self._emit(
            ctx=ctx,
            action="tenant.offboarded",
            outcome=AuditOutcome.success,
            severity=AuditSeverity.warning,
            resource_kind="tenant",
            resource_id=tenant_id,
            payload={"mode": mode},
        )

    # ------------------------------------------------------------------
    # Key rotation
    # ------------------------------------------------------------------

    async def key_rotated(
        self,
        *,
        ctx: SecurityContext | None,
        key_id: str,
        new_version: int,
    ) -> None:
        await self._emit(
            ctx=ctx,
            action="key.rotated",
            outcome=AuditOutcome.success,
            resource_kind="key",
            resource_id=key_id,
            payload={"new_version": new_version},
        )
