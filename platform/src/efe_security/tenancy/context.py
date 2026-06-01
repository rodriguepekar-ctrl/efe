"""Tenant context injection via :class:`contextvars.ContextVar`.

Handlers running under an async task graph should not pass
``tenant_id`` manually — instead, the interceptor sets the context at
request boundary and downstream code reads :func:`current_tenant`.
"""

from __future__ import annotations

from contextvars import ContextVar, Token
from dataclasses import dataclass, field
from typing import Any

__all__ = [
    "TenantContext",
    "current_tenant",
    "set_current_tenant",
    "reset_current_tenant",
]


_CURRENT_TENANT: ContextVar[str | None] = ContextVar(
    "efe_security.current_tenant", default=None
)


def current_tenant() -> str | None:
    return _CURRENT_TENANT.get()


def set_current_tenant(tenant_id: str | None) -> Token:
    return _CURRENT_TENANT.set(tenant_id)


def reset_current_tenant(token: Token) -> None:
    _CURRENT_TENANT.reset(token)


@dataclass(frozen=True, slots=True)
class TenantContext:
    tenant_id: str
    tier: str = "standard"
    region: str = "eu-west-1"
    labels: tuple[tuple[str, str], ...] = ()

    def __enter__(self) -> "TenantContext":
        # Sync context-manager support for rare sync callers.
        self._token = set_current_tenant(self.tenant_id)  # type: ignore[attr-defined]
        return self

    def __exit__(self, *_: Any) -> None:
        reset_current_tenant(self._token)  # type: ignore[attr-defined]
