"""RBAC registry : roles × scopes × (tenant, principal) → grants.

The registry is the authoritative source of truth for "who has what
scope". Scope strings follow the convention ``<resource>:<verb>`` —
e.g. ``drift:read``, ``sched:admin``.
"""

from __future__ import annotations

import asyncio
from dataclasses import dataclass, field
from datetime import datetime

__all__ = ["RbacRegistry", "RoleGrant", "ROLE_ACTION_MAP"]


# ---------------------------------------------------------------------------
# Reference mapping (§Annex C): role → set of scopes
# ---------------------------------------------------------------------------

ROLE_ACTION_MAP: dict[str, frozenset[str]] = {
    "anonymous": frozenset(),
    "reader": frozenset(
        {
            "drift:read",
            "sched:read",
            "job:read",
        }
    ),
    "operator": frozenset(
        {
            "drift:read",
            "drift:write",
            "sched:read",
            "sched:write",
            "job:read",
            "job:cancel",
            "rule:write",
        }
    ),
    "admin": frozenset(
        {
            "drift:read",
            "drift:write",
            "baseline:retire",
            "sched:read",
            "sched:write",
            "sched:admin",
            "job:read",
            "job:cancel",
            "rule:write",
            "policy:write",
            "audit:read",
        }
    ),
    "compliance_officer": frozenset(
        {
            "drift:read",
            "sched:read",
            "job:read",
            "audit:read",
            "audit:export",
            "consent:write",
            "dsar:process",
        }
    ),
    "security_officer": frozenset(
        {
            "drift:read",
            "sched:read",
            "job:read",
            "job:cancel",
            "baseline:retire",
            "policy:write",
            "audit:read",
            "audit:export",
            "tenant:onboard",
            "tenant:offboard",
            "key:rotate",
            "break_glass:activate",
        }
    ),
}


@dataclass(frozen=True, slots=True)
class RoleGrant:
    tenant_id: str
    principal_id: str
    role: str
    granted_at: datetime
    granted_by: str
    revoked_at: datetime | None = None

    def is_active(self) -> bool:
        return self.revoked_at is None


@dataclass(slots=True)
class RbacRegistry:
    _grants: dict[tuple[str, str, str], RoleGrant] = field(default_factory=dict)
    _lock: asyncio.Lock = field(default_factory=asyncio.Lock, repr=False)

    async def grant(self, grant: RoleGrant) -> None:
        key = (grant.tenant_id, grant.principal_id, grant.role)
        async with self._lock:
            self._grants[key] = grant

    async def revoke(
        self, *, tenant_id: str, principal_id: str, role: str, now: datetime
    ) -> bool:
        key = (tenant_id, principal_id, role)
        async with self._lock:
            g = self._grants.get(key)
            if g is None or g.revoked_at is not None:
                return False
            self._grants[key] = RoleGrant(
                tenant_id=g.tenant_id,
                principal_id=g.principal_id,
                role=g.role,
                granted_at=g.granted_at,
                granted_by=g.granted_by,
                revoked_at=now,
            )
            return True

    async def scopes_for(
        self, *, tenant_id: str, principal_id: str
    ) -> frozenset[str]:
        async with self._lock:
            roles = {
                g.role
                for g in self._grants.values()
                if g.tenant_id == tenant_id
                and g.principal_id == principal_id
                and g.is_active()
            }
        scopes: set[str] = set()
        for role in roles:
            scopes.update(ROLE_ACTION_MAP.get(role, frozenset()))
        return frozenset(scopes)

    async def roles_for(
        self, *, tenant_id: str, principal_id: str
    ) -> list[str]:
        async with self._lock:
            return sorted(
                g.role
                for g in self._grants.values()
                if g.tenant_id == tenant_id
                and g.principal_id == principal_id
                and g.is_active()
            )
