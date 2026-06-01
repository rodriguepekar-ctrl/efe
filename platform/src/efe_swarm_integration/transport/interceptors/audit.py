"""AuditInterceptor — emits M11 audit entries around every RPC."""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any, Protocol

__all__ = ["AuditInterceptor", "AuditPort"]


class AuditPort(Protocol):
    async def append(self, draft: Any) -> Any: ...  # pragma: no cover


@dataclass(slots=True)
class AuditInterceptor:
    """Adapter around :class:`efe_security.audit.AuditStore`.

    Uses duck-typing so callers can inject any object with an
    ``append(draft)`` coroutine. This keeps M12 free of a hard import
    of :mod:`efe_security` for pure-unit tests ; production wiring
    uses the real :class:`AuditEmitter`.
    """

    sink: AuditPort | None
    enabled: bool = True

    async def before(
        self,
        *,
        tenant_id: str | None,
        actor_id: str,
        method: str,
        request_id: str,
    ) -> None:
        if not self.enabled or self.sink is None:
            return
        draft = _build_draft(
            tenant_id=tenant_id,
            actor_id=actor_id,
            action=f"swarm.rpc.invoked.{method}",
            outcome="success",
            payload={"request_id": request_id},
        )
        try:
            await self.sink.append(draft)
        except Exception:
            pass  # pragma: no cover - audit must not break the request

    async def after(
        self,
        *,
        tenant_id: str | None,
        actor_id: str,
        method: str,
        request_id: str,
        ok: bool,
        status: str,
        latency_ms: float,
    ) -> None:
        if not self.enabled or self.sink is None:
            return
        draft = _build_draft(
            tenant_id=tenant_id,
            actor_id=actor_id,
            action=f"swarm.rpc.completed.{method}",
            outcome="success" if ok else "failure",
            payload={
                "request_id": request_id,
                "status": status,
                "latency_ms": latency_ms,
            },
        )
        try:
            await self.sink.append(draft)
        except Exception:
            pass  # pragma: no cover


def _build_draft(
    *,
    tenant_id: str | None,
    actor_id: str,
    action: str,
    outcome: str,
    payload: dict[str, Any],
) -> Any:
    """Build either an :class:`efe_security.audit.AuditDraft` when M11
    is reachable, or a plain dict otherwise."""
    try:
        from efe_security.audit import AuditDraft, AuditOutcome, AuditSeverity  # noqa: WPS433

        return AuditDraft(
            occurred_at=datetime.now(tz=timezone.utc),
            tenant_id=tenant_id,
            actor_kind="service",
            actor_id=actor_id,
            action=action,
            outcome=AuditOutcome(outcome),
            severity=AuditSeverity.info if outcome == "success" else AuditSeverity.warning,
            reasons=(),
            payload=payload,
        )
    except Exception:
        return {
            "occurred_at": datetime.now(tz=timezone.utc).isoformat(),
            "tenant_id": tenant_id,
            "actor_id": actor_id,
            "action": action,
            "outcome": outcome,
            "payload": payload,
        }
