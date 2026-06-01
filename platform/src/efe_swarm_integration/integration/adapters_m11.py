"""Concrete M11 → M12 adapters.

M12 advertises three Protocols (``PdpPort``, ``PiiLinterPort``,
``AuditHook``) to keep unit tests dependency-free. **In production**
M11 ships the real implementations ; this module is the single place
that performs the *direct import* from :mod:`efe_security` so the rest
of the platform stays decoupled while M12 still fully absorbs M11's
semantics.

Responsabilities :

* :class:`M11PdpAdapter` — wraps :class:`efe_security.authz.PolicyDecisionPoint`
  so :class:`SwarmClient` can call ``await pdp.authorise(...)`` against
  the real policy store and RBAC registry.
* :class:`M11PiiAdapter` — wraps :class:`efe_security.pii.PiiClassifier`
  so M12's raw-prompt guard honours the tenant's :class:`PiiPolicy`.
* :class:`M11AuditBridge` — translates M12's lightweight audit calls
  into :class:`efe_security.audit.AuditDraft` entries appended to the
  tamper-evident chain.
* :func:`security_context_to_swarm` — a helper that turns a
  fully-populated :class:`efe_security.SecurityContext` into the
  metadata / actor / tenant triple M12 needs.

All three adapters **fail closed** if M11 raises — the PDP deny becomes
:class:`UnauthorisedM12`, the PII hit becomes :class:`PiiDetectedInPrompt`,
and the audit failure is logged but does not block the request
(M11 invariant — audit must not break the happy path).
"""

from __future__ import annotations

import logging
from dataclasses import dataclass
from typing import Any

from efe_security.audit import (
    AuditDraft,
    AuditEmitter,
    AuditOutcome,
    AuditSeverity,
    AuditStore,
)
from efe_security.authz import (
    PolicyDecisionPoint,
    PolicyRequest,
    RbacRegistry,
)
# Register M12 actions in M11's policy dispatch table at adapter load time.
# This is the *only* place where M12 extends M11's action vocabulary — it
# happens once, at import, before any PolicyDecisionPoint is constructed.
from efe_security.authz.pdp import ACTION_SCOPES as _M11_ACTION_SCOPES
from efe_security.authz.rbac import ROLE_ACTION_MAP as _M11_ROLE_MAP
from efe_security.context import (
    Actor,
    ActorKind,
    Principal,
    Resource,
    SecurityContext,
)
from efe_security.errors import (
    ActionForbidden,
    ScopeMissing,
    TenantMismatch,
)
from efe_security.pii import PiiClassifier


_M12_REGISTERED_ACTIONS: dict[str, frozenset[str]] = {
    "swarm.classify": frozenset({"swarm:classify"}),
    "swarm.execute": frozenset({"swarm:execute"}),
    "swarm.execute_streaming": frozenset({"swarm:execute"}),
    "swarm.describe_spec": frozenset({"swarm:read"}),
    "swarm.cone.register": frozenset({"cone:write"}),
    "swarm.cone.evolve": frozenset({"cone:write"}),
    "swarm.cone.delete": frozenset({"cone:admin"}),
    "swarm.artefact.sync": frozenset({"swarm:admin"}),
}

for _action, _scopes in _M12_REGISTERED_ACTIONS.items():
    _M11_ACTION_SCOPES.setdefault(_action, _scopes)

# Extend the built-in M11 roles so default admins can invoke M12 actions
# without a separate migration.
_M12_SCOPES_FOR_ADMIN = frozenset(
    {"swarm:classify", "swarm:execute", "swarm:read", "swarm:admin", "cone:write"}
)
_M12_SCOPES_FOR_OPERATOR = frozenset({"swarm:classify", "swarm:read"})
_M12_SCOPES_FOR_READER = frozenset({"swarm:read"})
_M12_SCOPES_FOR_SECURITY = frozenset(
    {"swarm:admin", "cone:admin", "cone:write", "swarm:read"}
)

if "admin" in _M11_ROLE_MAP:
    _M11_ROLE_MAP["admin"] = _M11_ROLE_MAP["admin"] | _M12_SCOPES_FOR_ADMIN
if "operator" in _M11_ROLE_MAP:
    _M11_ROLE_MAP["operator"] = _M11_ROLE_MAP["operator"] | _M12_SCOPES_FOR_OPERATOR
if "reader" in _M11_ROLE_MAP:
    _M11_ROLE_MAP["reader"] = _M11_ROLE_MAP["reader"] | _M12_SCOPES_FOR_READER
if "security_officer" in _M11_ROLE_MAP:
    _M11_ROLE_MAP["security_officer"] = (
        _M11_ROLE_MAP["security_officer"] | _M12_SCOPES_FOR_SECURITY
    )

from efe_swarm_integration.errors import (
    PiiDetectedInPrompt,
    UnauthorisedM12,
)

_LOG = logging.getLogger("efe_swarm_integration.integration.m11")

__all__ = [
    "M11AuditBridge",
    "M11PdpAdapter",
    "M11PiiAdapter",
    "SwarmSecurityBundle",
    "security_context_to_swarm",
]


# ---------------------------------------------------------------------------
# PDP adapter
# ---------------------------------------------------------------------------


@dataclass(slots=True)
class M11PdpAdapter:
    """Concrete :class:`PdpPort` backed by M11's
    :class:`PolicyDecisionPoint`."""

    pdp: PolicyDecisionPoint
    rbac: RbacRegistry | None = None

    async def authorise(
        self,
        *,
        actor_id: str,
        tenant_id: str,
        action: str,
        resource_kind: str,
    ) -> bool:
        """Return True iff M11's PDP allows the action on the resource.

        On **any** failure (``ScopeMissing`` / ``ActionForbidden`` /
        ``TenantMismatch``), the adapter converts the M11 exception
        into a return of ``False`` — the caller then raises
        :class:`UnauthorisedM12`. This keeps M11 error semantics
        inside its own namespace.
        """
        # Build a minimal principal : actor + tenant, service-kind since
        # M12 is invoked from service accounts in production.
        principal = Principal(
            actor=Actor(kind=ActorKind.service, actor_id=actor_id),
            tenant_id=tenant_id,
            scopes=frozenset(),
        )
        resource = Resource(kind=resource_kind, tenant_id=tenant_id)
        req = PolicyRequest(
            principal=principal,
            action=action,
            resource=resource,
            context={"module": "m12"},
        )
        try:
            decision = await self.pdp.decide(req)
        except (ScopeMissing, ActionForbidden, TenantMismatch):
            return False
        return bool(decision.allow)

    async def decide_verbose(
        self,
        *,
        principal: Principal,
        action: str,
        resource: Resource,
        context: dict[str, Any] | None = None,
    ) -> tuple[bool, tuple[str, ...]]:
        """Richer helper returning ``(allow, reasons)`` directly from M11."""
        req = PolicyRequest(
            principal=principal,
            action=action,
            resource=resource,
            context=context or {"module": "m12"},
        )
        decision = await self.pdp.decide(req)
        return bool(decision.allow), tuple(decision.reasons)


# ---------------------------------------------------------------------------
# PII adapter
# ---------------------------------------------------------------------------


@dataclass(slots=True)
class M11PiiAdapter:
    """Concrete :class:`PiiLinterPort` using M11's :class:`PiiClassifier`.

    Exposes the :meth:`find` method M12's :class:`SwarmClient` calls.
    The underlying :class:`PiiClassifier` enforces NFKC + depth-limit +
    regex detection — already M11-hardened.
    """

    classifier: PiiClassifier

    def find(self, text: str) -> list[Any]:
        """Return all PII hits (possibly empty)."""
        return list(self.classifier.find(text))

    def assert_clean(self, text: str, *, context: str = "") -> None:
        """Raise :class:`PiiDetectedInPrompt` if any hit is found."""
        hits = self.classifier.find(text)
        if hits:
            raise PiiDetectedInPrompt(
                f"PII detected in {context or 'text'}",
                detail={"kinds": [h.kind.value for h in hits]},
            )


# ---------------------------------------------------------------------------
# Audit bridge
# ---------------------------------------------------------------------------


@dataclass(slots=True)
class M11AuditBridge:
    """Forward M12 RPC audit events into M11's tamper-evident chain.

    Uses :class:`AuditEmitter` internally so the ``action`` codes follow
    the platform convention (``swarm.rpc.*``).
    """

    store: AuditStore
    _emitter: AuditEmitter | None = None

    def _ensure_emitter(self) -> AuditEmitter:
        if self._emitter is None:
            self._emitter = AuditEmitter(sink=self.store)
        return self._emitter

    async def append(self, draft: Any) -> None:
        """Compatibility method for :class:`AuditInterceptor` — accepts
        either an :class:`AuditDraft` or a raw dict built by the
        interceptor's fallback path.
        """
        if isinstance(draft, AuditDraft):
            await self.store.append(draft)
            return
        if not isinstance(draft, dict):
            _LOG.warning("m11_audit.bridge.bad_type type=%s", type(draft).__name__)
            return
        # Graceful coercion : the interceptor's fallback uses a dict.
        try:
            coerced = AuditDraft(
                occurred_at=_parse_iso(draft.get("occurred_at")),
                tenant_id=draft.get("tenant_id"),
                actor_kind=str(draft.get("actor_kind", "service")),
                actor_id=str(draft.get("actor_id", "anonymous")),
                action=str(draft.get("action", "swarm.rpc.unknown")),
                outcome=AuditOutcome(
                    str(draft.get("outcome", "success"))
                ),
                severity=AuditSeverity.info,
                reasons=(),
                payload=dict(draft.get("payload") or {}),
            )
        except Exception as exc:
            _LOG.warning("m11_audit.bridge.coerce_failed err=%s", exc)
            return
        await self.store.append(coerced)

    async def emit_rpc(
        self,
        *,
        ctx: SecurityContext,
        method: str,
        ok: bool,
        latency_ms: float,
        swarm_id: str | None = None,
        verdict: str | None = None,
    ) -> None:
        """Typed helper for a completed RPC."""
        await self._ensure_emitter().sink.append(
            AuditDraft(
                occurred_at=_now_utc(),
                tenant_id=ctx.principal.tenant_id,
                actor_kind=ctx.principal.actor.kind.value,
                actor_id=ctx.principal.actor.actor_id,
                action=f"swarm.rpc.{method.split('/')[-1].lower()}",
                outcome=AuditOutcome.success if ok else AuditOutcome.failure,
                severity=AuditSeverity.info if ok else AuditSeverity.warning,
                reasons=(),
                payload={
                    "request_id": ctx.request_id,
                    "latency_ms": latency_ms,
                    "swarm_id": swarm_id,
                    "verdict": verdict,
                },
            )
        )


# ---------------------------------------------------------------------------
# SecurityContext → SwarmSecurityBundle
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class SwarmSecurityBundle:
    """Trust triple that M12's :class:`SwarmClient` operates on."""

    tenant_id: str
    actor_id: str
    request_id: str
    principal: Principal


def security_context_to_swarm(ctx: SecurityContext) -> SwarmSecurityBundle:
    """Project a full M11 :class:`SecurityContext` into the fields
    M12 needs.

    Raises :class:`UnauthorisedM12` if the context has no tenant — M12
    refuses anonymous calls (see invariant I3 and architecture §8.2).
    """
    tenant = ctx.principal.tenant_id
    if tenant is None:
        raise UnauthorisedM12(
            "SecurityContext has no tenant binding",
            detail={"request_id": ctx.request_id},
        )
    return SwarmSecurityBundle(
        tenant_id=tenant,
        actor_id=ctx.principal.actor.actor_id,
        request_id=ctx.request_id,
        principal=ctx.principal,
    )


# ---------------------------------------------------------------------------
# Internal helpers
# ---------------------------------------------------------------------------


def _now_utc():
    from datetime import datetime, timezone

    return datetime.now(tz=timezone.utc)


def _parse_iso(val: Any):
    from datetime import datetime, timezone

    if isinstance(val, str):
        return datetime.fromisoformat(val.replace("Z", "+00:00"))
    return datetime.now(tz=timezone.utc)
