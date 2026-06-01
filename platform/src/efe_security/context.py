"""SecurityContext — the immutable value object passed to every handler.

Built by the interceptor layer, handed to the business handler. Handlers
**inspect** it ; they never mutate. For derived values (scoped-down
context), use :meth:`SecurityContext.with_scope`.

Key design decisions :

* ``principal`` is mandatory — invariant **I1**.
* ``tenant_id`` may be ``None`` for platform-level actions ; business
  code that requires a tenant must assert.
* ``obligations`` flows from the PDP back to the handler (e.g.
  ``"mask_pii"``, ``"audit=high"``). Handlers **must** honour them.
* ``request_id`` is propagated end-to-end for tracing.
"""

from __future__ import annotations

import enum
from dataclasses import dataclass, field, replace
from datetime import datetime, timezone
from typing import Any, Literal

__all__ = [
    "Actor",
    "ActorKind",
    "Obligation",
    "Principal",
    "Resource",
    "SecurityContext",
    "empty_context",
]


# ---------------------------------------------------------------------------
# Actor
# ---------------------------------------------------------------------------


class ActorKind(str, enum.Enum):
    human = "human"
    service = "service"
    data_subject = "data_subject"
    anonymous = "anonymous"
    system = "system"  # internal daemons (scheduler, gc)


@dataclass(frozen=True, slots=True)
class Actor:
    """Who is acting (auditable identity).

    ``actor_id`` is the stable identifier : ``sub`` for humans / data
    subjects, ``spiffe_id`` for services, ``"platform"`` for system
    daemons, ``"anonymous"`` for unauthenticated.
    """

    kind: ActorKind
    actor_id: str
    display_name: str | None = None
    amr: tuple[str, ...] = ()  # authentication methods (e.g. "pwd","mfa")
    acr: str | None = None  # authentication context reference

    def __post_init__(self) -> None:
        if not self.actor_id:
            raise ValueError("Actor.actor_id is required")


# ---------------------------------------------------------------------------
# Principal
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class Principal:
    """The authenticated identity bound to this request."""

    actor: Actor
    tenant_id: str | None
    session_id: str | None = None
    jti: str | None = None  # JWT id for revocation
    scopes: frozenset[str] = field(default_factory=frozenset)
    groups: frozenset[str] = field(default_factory=frozenset)
    expires_at: datetime | None = None

    def __post_init__(self) -> None:
        if self.expires_at is not None and self.expires_at.tzinfo is None:
            raise ValueError("Principal.expires_at must be tz-aware")

    def has_scope(self, scope: str) -> bool:
        return scope in self.scopes

    def has_any(self, *scopes: str) -> bool:
        return any(s in self.scopes for s in scopes)

    def has_all(self, *scopes: str) -> bool:
        return all(s in self.scopes for s in scopes)


# ---------------------------------------------------------------------------
# Resource
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class Resource:
    """What is being acted upon (for ABAC / ownership decisions)."""

    kind: str
    id: str | None = None
    tenant_id: str | None = None
    owner_id: str | None = None
    labels: frozenset[str] = field(default_factory=frozenset)
    attributes: tuple[tuple[str, str], ...] = ()

    def attr(self, key: str, default: str | None = None) -> str | None:
        for k, v in self.attributes:
            if k == key:
                return v
        return default


# ---------------------------------------------------------------------------
# Obligation
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class Obligation:
    """An instruction attached to an allow decision.

    Common cases :

    * ``mask_pii`` — mask PII fields in the response.
    * ``audit=high`` — force audit severity.
    * ``rate=slow`` — artificially slow down the response.
    """

    kind: str
    args: tuple[tuple[str, str], ...] = ()

    def arg(self, key: str, default: str | None = None) -> str | None:
        for k, v in self.args:
            if k == key:
                return v
        return default


# ---------------------------------------------------------------------------
# SecurityContext
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class SecurityContext:
    """Immutable bag of everything a handler needs from the trust plane.

    Construction is internal to M11 middleware — handlers never construct
    instances directly.
    """

    principal: Principal
    request_id: str
    obligations: tuple[Obligation, ...] = ()
    issued_at: datetime = field(
        default_factory=lambda: datetime.now(tz=timezone.utc)
    )
    channel: Literal["http", "grpc", "nats", "internal"] = "http"
    client_ip: str | None = None
    locale: str = "en-US"
    extra: tuple[tuple[str, str], ...] = ()

    def __post_init__(self) -> None:
        if not self.request_id:
            raise ValueError("SecurityContext.request_id is required")
        if self.issued_at.tzinfo is None:
            raise ValueError("SecurityContext.issued_at must be tz-aware")

    # ------------------------------------------------------------------
    # Derivations (never mutate — always return a new instance)
    # ------------------------------------------------------------------

    def with_obligations(self, *extra: Obligation) -> SecurityContext:
        return replace(self, obligations=self.obligations + tuple(extra))

    def with_tenant(self, tenant_id: str) -> SecurityContext:
        """Return a context pinned to a specific tenant (used when the
        request's principal has an implicit tenant)."""
        if self.principal.tenant_id is not None and self.principal.tenant_id != tenant_id:
            raise ValueError(
                f"cannot cross-bind tenant: principal={self.principal.tenant_id!r} "
                f"target={tenant_id!r}"
            )
        new_principal = replace(self.principal, tenant_id=tenant_id)
        return replace(self, principal=new_principal)

    # ------------------------------------------------------------------
    # Convenience predicates
    # ------------------------------------------------------------------

    def is_anonymous(self) -> bool:
        return self.principal.actor.kind == ActorKind.anonymous

    def require_tenant(self) -> str:
        if self.principal.tenant_id is None:
            raise ValueError("tenant context missing")
        return self.principal.tenant_id

    def has_obligation(self, kind: str) -> bool:
        return any(o.kind == kind for o in self.obligations)

    def get_obligation(self, kind: str) -> Obligation | None:
        for o in self.obligations:
            if o.kind == kind:
                return o
        return None

    def to_log_dict(self) -> dict[str, Any]:
        """Safe-for-logs representation (no sensitive fields)."""
        return {
            "request_id": self.request_id,
            "actor_kind": self.principal.actor.kind.value,
            "actor_id": self.principal.actor.actor_id,
            "tenant_id": self.principal.tenant_id,
            "scopes": sorted(self.principal.scopes),
            "channel": self.channel,
            "obligations": [o.kind for o in self.obligations],
        }


# ---------------------------------------------------------------------------
# Sentinel: empty anonymous context (for health checks etc.)
# ---------------------------------------------------------------------------


def empty_context(request_id: str, *, channel: str = "http") -> SecurityContext:
    """Build an anonymous-principal context. Use sparingly — only for
    truly public endpoints."""
    return SecurityContext(
        principal=Principal(
            actor=Actor(kind=ActorKind.anonymous, actor_id="anonymous"),
            tenant_id=None,
            scopes=frozenset(),
        ),
        request_id=request_id,
        channel="http" if channel not in ("http", "grpc", "nats", "internal") else channel,  # type: ignore[arg-type]
    )
