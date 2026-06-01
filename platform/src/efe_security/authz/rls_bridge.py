"""Bridge between the Python tenant context and Postgres RLS.

SQL setup :

    ALTER TABLE t ENABLE ROW LEVEL SECURITY;
    CREATE POLICY t_tenant ON t
      USING (tenant_id = current_setting('efe.tenant_id', true));

Runtime, at the start of each transaction :

    SET LOCAL efe.tenant_id = '<tenant>';

This module gives an `async def apply(conn, tenant_id)` helper usable
with ``asyncpg``, ``psycopg``, or a Protocol stub for tests.

Security hardening : ``tenant_id`` is restricted to a strict allowlist
of characters (``[A-Za-z0-9_-]``) to rule out SQL injection vectors via
control characters, quotes, newlines, or null bytes.
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from typing import Protocol

__all__ = ["RlsBridge", "SqlConnection", "TENANT_ID_PATTERN", "validate_tenant_id"]


class SqlConnection(Protocol):
    async def execute(self, sql: str) -> None: ...  # pragma: no cover


TENANT_ID_PATTERN: re.Pattern[str] = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_\-]{0,63}$")
"""Strict shape allowed for tenant identifiers.

Rationale : rule out every potential injection vector in ``SET LOCAL``
by never accepting control characters, whitespace, quotes, backslashes,
null bytes, unicode or anything beyond a restricted alphabet. The first
character must be alphanumeric to avoid leading hyphen / underscore
shell-flag confusion in adjacent tooling.
"""


def validate_tenant_id(tenant_id: str) -> str:
    """Return the validated tenant_id or raise :class:`ValueError`."""
    if not isinstance(tenant_id, str):
        raise ValueError("tenant_id must be a string")
    if not TENANT_ID_PATTERN.match(tenant_id):
        raise ValueError(
            f"tenant_id shape violation: {tenant_id!r} ; "
            f"must match {TENANT_ID_PATTERN.pattern}"
        )
    return tenant_id


@dataclass(slots=True)
class RlsBridge:
    setting_name: str = "efe.tenant_id"

    async def apply(self, conn: SqlConnection, tenant_id: str | None) -> None:
        if tenant_id is None:
            await conn.execute(f"SET LOCAL {self.setting_name} = '__none__'")
            return
        validated = validate_tenant_id(tenant_id)
        await conn.execute(
            f"SET LOCAL {self.setting_name} = '{validated}'"
        )

    async def clear(self, conn: SqlConnection) -> None:
        await conn.execute(f"SET LOCAL {self.setting_name} = '__none__'")
