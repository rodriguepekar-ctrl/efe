"""Unified security middleware — the Policy Enforcement Point.

Wraps every inbound HTTP request in the full trust chain :

1. **Extract** the credential (Bearer JWT or mTLS SPIFFE id).
2. **Authenticate** the principal (raises if invalid).
3. **Bind** the tenant context (``SET LOCAL efe.tenant_id`` via RLS).
4. **Build** the immutable :class:`SecurityContext` and attach it to
   the request scope.
5. **Emit** an audit line tagged ``authn.success`` / ``authn.failure``.
6. **Translate** any :class:`SecurityError` into a ProblemDetails
   response (RFC 7807).

The middleware is framework-agnostic thanks to a thin dispatcher : it
can be mounted as ASGI middleware, as a FastAPI dependency, or as a
standalone async callable in test harnesses.
"""

from __future__ import annotations

import asyncio
import logging
from contextvars import ContextVar
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any, Awaitable, Callable, Protocol

from efe_security.audit.records import AuditDraft, AuditOutcome, AuditSeverity
from efe_security.audit.store import InMemoryAuditStore
from efe_security.authn.jwt_verifier import JwtVerifier
from efe_security.authn.mtls_verifier import MtlsVerifier
from efe_security.authn.session import SessionStore
from efe_security.context import Actor, ActorKind, Principal, SecurityContext
from efe_security.errors import (
    AuthNError,
    MissingToken,
    SecurityError,
)
from efe_security.tenancy.context import set_current_tenant, reset_current_tenant

_LOG = logging.getLogger("efe_security.middleware")

__all__ = [
    "AuditSinkProto",
    "CurrentContext",
    "RequestAdapter",
    "SecurityMiddleware",
    "current_context",
    "set_current_context",
]


_CURRENT_CTX: ContextVar[SecurityContext | None] = ContextVar(
    "efe_security.current_context", default=None
)


def current_context() -> SecurityContext | None:
    """Return the SecurityContext bound to the current async task (if any)."""
    return _CURRENT_CTX.get()


def set_current_context(ctx: SecurityContext | None) -> Any:
    return _CURRENT_CTX.set(ctx)


@dataclass(frozen=True, slots=True)
class CurrentContext:
    ctx: SecurityContext


# ---------------------------------------------------------------------------
# Request adapter — a minimal interface we ask of the framework.
# ---------------------------------------------------------------------------


class RequestAdapter(Protocol):
    """The minimum surface the middleware needs from an HTTP request."""

    def header(self, name: str) -> str | None: ...  # pragma: no cover
    def peer_spiffe_id(self) -> str | None: ...  # pragma: no cover
    def request_id(self) -> str: ...  # pragma: no cover
    def client_ip(self) -> str | None: ...  # pragma: no cover


class AuditSinkProto(Protocol):
    async def append(self, draft: AuditDraft) -> Any: ...  # pragma: no cover


# ---------------------------------------------------------------------------
# Middleware
# ---------------------------------------------------------------------------


@dataclass(slots=True)
class SecurityMiddleware:
    """Orchestrates AuthN → Tenant binding → context injection → audit.

    Construction requires the dependencies already wired ; for HTTP
    apps, :func:`efe_security.api.build_app` does this for you.
    """

    jwt: JwtVerifier | None
    mtls: MtlsVerifier | None
    sessions: SessionStore | None
    audit: AuditSinkProto
    tenant_required: bool = True

    async def authenticate(self, req: RequestAdapter, *, now: datetime) -> Principal:
        """Return the authenticated principal or raise :class:`AuthNError`."""
        # 1) mTLS takes precedence (service-to-service).
        spiffe_uri = req.peer_spiffe_id()
        if spiffe_uri and self.mtls is not None:
            return self.mtls.build_principal(spiffe_uri=spiffe_uri)

        # 2) Bearer token.
        bearer = req.header("authorization") or req.header("Authorization")
        if bearer and bearer.lower().startswith("bearer "):
            token = bearer.split(" ", 1)[1].strip()
            if self.jwt is None:
                raise AuthNError("JWT verifier not configured")
            _, principal = await self.jwt.verify(token, now=now)
            return principal

        # 3) Opaque session token.
        session_hdr = req.header("x-efe-session")
        if session_hdr and self.sessions is not None:
            session = await self.sessions.resolve(session_hdr, now=now)
            return session.principal

        raise MissingToken("no credential on request")

    async def build_context(self, req: RequestAdapter) -> SecurityContext:
        """Full authentication + context build.

        The caller may wrap this in a ``try`` to catch errors, or use
        :meth:`dispatch` which handles the whole flow.
        """
        now = datetime.now(tz=timezone.utc)
        principal = await self.authenticate(req, now=now)
        if self.tenant_required and principal.tenant_id is None and principal.actor.kind not in (
            ActorKind.anonymous,
            ActorKind.system,
            ActorKind.service,
        ):
            raise AuthNError("principal has no tenant binding")
        ctx = SecurityContext(
            principal=principal,
            request_id=req.request_id(),
            channel="http",
            client_ip=req.client_ip(),
        )
        return ctx

    async def dispatch(
        self,
        req: RequestAdapter,
        handler: Callable[[SecurityContext], Awaitable[Any]],
    ) -> Any:
        """Full middleware flow : authenticate, bind, run handler, audit.

        On success : emits an ``authn.success`` audit entry and returns
        the handler output. On failure : emits ``authn.failure`` and
        re-raises.
        """
        ctx: SecurityContext | None = None
        ctx_token = None
        tenant_token = None
        try:
            ctx = await self.build_context(req)
        except SecurityError as exc:
            await self._audit_failure(req, exc)
            raise
        assert ctx is not None

        try:
            ctx_token = set_current_context(ctx)
            if ctx.principal.tenant_id is not None:
                tenant_token = set_current_tenant(ctx.principal.tenant_id)
            await self._audit_success(ctx)
            return await handler(ctx)
        finally:
            if tenant_token is not None:
                reset_current_tenant(tenant_token)
            if ctx_token is not None:
                _CURRENT_CTX.reset(ctx_token)

    # ------------------------------------------------------------------
    # Audit emission helpers
    # ------------------------------------------------------------------

    async def _audit_success(self, ctx: SecurityContext) -> None:
        draft = AuditDraft(
            occurred_at=datetime.now(tz=timezone.utc),
            tenant_id=ctx.principal.tenant_id,
            actor_kind=ctx.principal.actor.kind.value,
            actor_id=ctx.principal.actor.actor_id,
            action="authn.success",
            outcome=AuditOutcome.success,
            severity=AuditSeverity.info,
            reasons=("middleware_dispatch",),
            payload={
                "request_id": ctx.request_id,
                "client_ip": ctx.client_ip,
                "channel": ctx.channel,
            },
        )
        try:
            await self.audit.append(draft)
        except Exception:  # pragma: no cover - audit must never break a request
            _LOG.exception("middleware.audit_success_failed")

    async def _audit_failure(self, req: RequestAdapter, exc: SecurityError) -> None:
        draft = AuditDraft(
            occurred_at=datetime.now(tz=timezone.utc),
            tenant_id=None,
            actor_kind="anonymous",
            actor_id="anonymous",
            action="authn.failure",
            outcome=AuditOutcome.failure,
            severity=AuditSeverity.warning,
            reasons=(exc.code,),
            payload={
                "request_id": req.request_id(),
                "client_ip": req.client_ip(),
                "code": exc.code,
            },
        )
        try:
            await self.audit.append(draft)
        except Exception:  # pragma: no cover
            _LOG.exception("middleware.audit_failure_failed")


_ = asyncio  # retained for Protocol API parity
_ = field
_ = InMemoryAuditStore
